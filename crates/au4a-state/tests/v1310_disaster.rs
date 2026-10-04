//! v1.3.10 集成测试：灾难恢复演练（备份 → 源丢失 → 2PC 重建 → 增量续跑）。
//!
//! 这是整条轨道的收官测试：把前面九版全部拉进最坏情况里跑一遍。
//! 关键断言不是「恢复了」，而是：
//! 1. **备份是可独立验证的**：内容寻址 + Agent 签名，改一个字节就拒；
//! 2. **源真的丢了**：重建只用备份与签名，不碰源介质；
//! 3. **重建经历失败**：第一次 2PC 注入故障 → 回滚干净 → 重试成功；
//! 4. **增量续跑**：只补缺口（`ops_skipped > 0`），最终与源逐字节一致。

use au4a_core::{AgentKeys, CoreError, Credits};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_state::{
    compare, run_drill, self_check, write_snapshot, Backup, DrillOptions, FaultPoint, MemoryStore,
    NodeId, SignedSnapshot, SnapshotPolicy, StateBlock, StateDelta, StateSnapshot, StateStore,
    StateZone,
};
use serde_json::json;

fn agent(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

fn state(seed: u8, v: i64) -> StateSnapshot {
    let blocks = vec![
        StateBlock::new(StateZone::Fs, "/work/a", json!({"v": v})).unwrap(),
        StateBlock::new(StateZone::Fs, "/work/b", json!({"v": v * 2})).unwrap(),
        StateBlock::new(StateZone::Memory, "counter", json!(v)).unwrap(),
        StateBlock::new(StateZone::Memory, "last_task", json!(format!("t{v}"))).unwrap(),
        StateBlock::new(
            StateZone::Context,
            "goal",
            json!({"text": "灾后继续", "v": v}),
        )
        .unwrap(),
    ];
    StateSnapshot::capture(&agent(seed).did(), "node-a", v as u64, blocks).unwrap()
}

#[test]
fn a_full_drill_survives_the_loss_of_the_source() {
    let keys = agent(1);
    let report = run_drill(
        &keys,
        &state(1, 1),
        &state(1, 2),
        &NodeId::new("node-b").unwrap(),
        &DrillOptions::default(),
    )
    .unwrap();
    assert!(report.is_ok(), "{report:#?}");
    assert!(report.backup_verified);
    assert!(report.source_lost);
    assert!(report.backup_object_id.starts_with("sha256:"));
    assert_eq!(report.backup_blocks, 5);

    // 重建：第一次注入故障回滚，第二次提交。
    assert!(report.rebuild_first_clean, "{report:#?}");
    assert!(report.rebuild_first_attempt.starts_with("rolled_back@"));
    assert_eq!(report.rebuild_first_live_root, report.backup_content_root);
    assert!(report.rebuild_committed);

    // 增量续跑：确实丢了帧、确实省了操作。
    assert!(report.dropped_frames > 0);
    assert!(report.resumed_from > 0);
    assert!(report.ops_skipped > 0);

    // 最终状态：与源逐字节一致（规范 JSON 哈希相等），且三项检查都干净。
    assert_eq!(report.final_content_root, report.source_content_root);
    assert!(report.identical_to_source);
    assert!(report.consistency_clean);
    assert_eq!(report.node_findings, 0);
    assert_eq!(report.rebuild_orphans, 0);
    assert!(report.udos_object_id.starts_with("sha256:"));
    assert_eq!(report.evidence_grade, "verified");
}

#[test]
fn a_corrupted_backup_is_refused_instead_of_restored() {
    let keys = agent(2);
    let source = state(2, 1);
    let signed = SignedSnapshot::sign(source.clone(), &keys).unwrap();
    let policy = SnapshotPolicy::for_agent(keys.did())
        .expecting_content_root(source.content_root().unwrap());

    // 完好介质：验证通过。
    let mut good = MemoryStore::new();
    write_snapshot(&mut good, "backup:", &source).unwrap();
    assert!(au4a_state::verify_media(&good, "backup:", &signed, &policy).is_ok());

    // 介质损坏（一块的值被改）：拒绝，而不是「尽力恢复」。
    let mut corrupted = MemoryStore::new();
    write_snapshot(&mut corrupted, "backup:", &source).unwrap();
    corrupted
        .put("backup:memory:counter", json!({"v": 999}))
        .unwrap();
    assert_eq!(
        au4a_state::verify_media(&corrupted, "backup:", &signed, &policy),
        Err(CoreError::InvalidSignature)
    );

    // 介质少了块：同样拒绝。
    let mut missing = MemoryStore::new();
    write_snapshot(&mut missing, "backup:", &source).unwrap();
    missing.remove("backup:context:goal").unwrap();
    assert_eq!(
        au4a_state::verify_media(&missing, "backup:", &signed, &policy),
        Err(CoreError::InvalidSignature)
    );
}

#[test]
fn a_backup_cannot_be_claimed_by_another_agent() {
    let a = agent(3);
    let b = agent(4);
    // 不能替别人备份。
    assert!(Backup::create(&b, &state(3, 1)).is_err());
    // 也不能用别人的策略验证自己的备份。
    let backup = Backup::create(&a, &state(3, 1)).unwrap();
    assert_eq!(
        backup.verify(&SnapshotPolicy::for_agent(b.did())),
        Err(CoreError::InvalidDid)
    );
    // 签名者与主体不符的快照即使自洽也过不了导入。
    let forged = SignedSnapshot::sign(state(4, 1), &b).unwrap();
    let policy = SnapshotPolicy::for_agent(a.did());
    assert!(forged.verify_policy(&policy).is_err());
}

#[test]
fn the_drill_without_a_fault_still_rebuilds_identically() {
    let keys = agent(5);
    let options = DrillOptions {
        fault_then_retry: false,
        ..DrillOptions::default()
    };
    let report = run_drill(
        &keys,
        &state(5, 1),
        &state(5, 3),
        &NodeId::new("node-b").unwrap(),
        &options,
    )
    .unwrap();
    assert!(report.rebuild_committed);
    assert!(report.identical_to_source);
    assert!(report.is_ok(), "{report:#?}");
}

#[test]
fn the_drill_report_is_replayable() {
    let run = || {
        let keys = agent(6);
        run_drill(
            &keys,
            &state(6, 1),
            &state(6, 4),
            &NodeId::new("node-b").unwrap(),
            &DrillOptions::default(),
        )
        .unwrap()
        .to_value()
    };
    assert_eq!(run(), run());
}

#[test]
fn every_fault_point_can_be_survived_by_backup_plus_retry() {
    // 灾后重建的核心保证：任何阶段的失败都能回滚，然后重试成功。
    for point in FaultPoint::ALL {
        let keys = agent(7);
        let source = state(7, 1);
        let evolved = state(7, 2);
        let backup = Backup::create(&keys, &source).unwrap();
        let policy = SnapshotPolicy::for_agent(keys.did())
            .expecting_content_root(source.content_root().unwrap())
            .with_min_epoch(1);
        let restored = backup.verify(&policy).unwrap();
        assert_eq!(
            restored.content_root().unwrap(),
            source.content_root().unwrap()
        );

        // 用 2PC 在恢复出的 base 上做一次迁移，并在指定阶段注入故障。
        let delta = StateDelta::between(&source, &evolved).unwrap();
        let signed = SignedSnapshot::sign(evolved.clone(), &keys).unwrap();
        let plan = au4a_state::MigrationPlan::new(
            &keys.did(),
            &NodeId::new("node-a").unwrap(),
            &NodeId::new("node-b").unwrap(),
            source.content_root().unwrap(),
            evolved.content_root().unwrap(),
            delta.id().unwrap(),
            2,
        )
        .unwrap();
        let mut node = au4a_state::NodeStore::open(
            NodeId::new("node-b").unwrap(),
            keys.did(),
            MemoryStore::new(),
        )
        .unwrap();
        backup.restore_into(&mut node, &policy).unwrap();
        let first = au4a_state::migrate(
            &mut node,
            plan.clone(),
            &delta,
            &signed,
            &mut au4a_state::FaultInjector::at(point),
        )
        .unwrap();
        assert!(!first.is_confirmed(), "{point:?}");
        assert_eq!(
            node.live_content_root().unwrap(),
            source.content_root().unwrap(),
            "{point:?}"
        );
        // 重试。
        let second = au4a_state::migrate(
            &mut node,
            plan,
            &delta,
            &signed,
            &mut au4a_state::FaultInjector::none(),
        )
        .unwrap();
        assert!(second.is_confirmed(), "{point:?}");
        let live = node.live_snapshot().unwrap();
        assert!(compare(&evolved, &live).unwrap().is_clean(), "{point:?}");
    }
}

#[test]
fn self_check_covers_the_disaster_drill() {
    let checks = self_check();
    assert_eq!(checks.len(), 21);
    assert!(au4a_core::all_passed(&checks), "{checks:#?}");
    let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"disaster.drill"));
    assert!(names.contains(&"disaster.corrupted_backup_refused"));
}

#[test]
fn capability_coverage_is_still_complete_after_the_last_version() {
    let coverage = au4a_state::coverage_check().unwrap();
    assert!(coverage.is_clean(), "{coverage:#?}");
    assert_eq!(coverage.capabilities, 21);
    assert_eq!(coverage.checks, 21);
    // 十个版本全部落地。
    let manifest = au4a_state::capability_manifest();
    for minor in 1..=10 {
        let tag = format!("v1.3.{minor}");
        let items = manifest["items"].as_array().unwrap();
        assert!(
            items.iter().any(|i| i["since"] == json!(tag)),
            "版本 {tag} 没有对应能力"
        );
    }
}

#[test]
fn scenario_carries_the_drill_evidence() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let keys = agent(8);
    kernel
        .register(&keys, "carrier", &["state.disaster"], Credits(20))
        .unwrap();
    let out = au4a_state::scenario(&mut kernel).unwrap();
    assert_eq!(out["disaster"]["backup_verified"], json!(true));
    assert_eq!(out["disaster"]["source_lost"], json!(true));
    assert_eq!(out["disaster"]["rebuild_committed"], json!(true));
    assert_eq!(out["disaster"]["identical_to_source"], json!(true));
    assert_eq!(out["disaster"]["consistency_clean"], json!(true));
    assert_eq!(out["disaster"]["is_ok"], json!(true));
    assert!(out["disaster"]["ops_skipped"].as_u64().unwrap() > 0);
    assert!(out["disaster"]["backup_object_id"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
    kernel.ledger().check_conservation().unwrap();
}

#[test]
fn scenario_is_replayable_end_to_end() {
    let mut k1 = Kernel::new(KernelConfig::default());
    let mut k2 = Kernel::new(KernelConfig::default());
    let a = au4a_state::scenario(&mut k1).unwrap();
    let b = au4a_state::scenario(&mut k2).unwrap();
    assert_eq!(a, b, "整个 scenario 必须逐字节可重放");
    // 关键保证同时成立，互不矛盾。
    assert_eq!(a["identical"], json!(true));
    assert_eq!(a["migration_identical"], json!(true));
    assert_eq!(a["two_phase"]["committed"], json!(true));
    assert_eq!(a["consistency"]["identical"], json!(true));
    assert_eq!(a["udos"]["roundtrip_identical"], json!(true));
    assert_eq!(a["perf"]["second_capture_hashes"], json!(0));
    assert_eq!(a["docs"]["coverage"]["unclaimed_checks"], json!([]));
    assert_eq!(a["disaster"]["is_ok"], json!(true));
}
