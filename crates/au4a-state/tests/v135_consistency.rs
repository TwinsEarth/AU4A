//! v1.3.5 集成测试：一致性检查。
//!
//! 「恢复成功」必须是一个**可以被检查**的结论。这里测试的不是「返回 Ok」，
//! 而是：源与恢复后的状态逐区摘要相等；一旦有人动了存储的字节，报告能说出
//! **哪一个区、哪一个键、发生了什么**（缺失/多余/被改）。

use au4a_core::{AgentKeys, Credits, Did};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_state::{
    audit_node, compare, compare_store, migrate, self_check, summarize, write_snapshot,
    FaultInjector, FindingCode, MemoryStore, MigrationPlan, NodeId, NodeStore, SignedSnapshot,
    StateBlock, StateDelta, StateSnapshot, StateStore, StateZone, INTENT_KEY,
};
use serde_json::json;

fn agent(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

fn three_zone(v: i64) -> Vec<StateBlock> {
    vec![
        StateBlock::new(StateZone::Fs, "/work/a", json!({"v": v})).unwrap(),
        StateBlock::new(StateZone::Fs, "/work/b", json!({"v": v})).unwrap(),
        StateBlock::new(StateZone::Memory, "counter", json!(v)).unwrap(),
        StateBlock::new(StateZone::Context, "goal", json!({"v": v})).unwrap(),
    ]
}

fn snap(keys: &AgentKeys, v: i64) -> StateSnapshot {
    StateSnapshot::capture(&keys.did(), "node-a", 1, three_zone(v)).unwrap()
}

#[test]
fn identical_content_reports_clean_with_equal_zone_roots() {
    let keys = agent(1);
    let a = snap(&keys, 1);
    let moved = a.rebase("node-b", 7).unwrap();
    // 出处不同（node/epoch），但内容一致：报告必须干净。
    assert_ne!(a.root(), moved.root());
    let report = compare(&a, &moved).unwrap();
    assert!(report.is_clean());
    assert!(report.identical);
    assert_eq!(report.changed_blocks(), 0);
    assert_eq!(report.zones.len(), 3);
    for zone in &report.zones {
        assert!(zone.equal, "{zone:?}");
        assert_eq!(zone.left_root, zone.right_root);
    }
    assert!(summarize(&report).starts_with("一致"));
}

#[test]
fn damaged_state_is_located_per_zone_and_key() {
    let keys = agent(2);
    let source = snap(&keys, 1);
    let damaged = StateSnapshot::capture(
        &keys.did(),
        "node-b",
        2,
        vec![
            // /work/a 被改
            StateBlock::new(StateZone::Fs, "/work/a", json!({"v": 99})).unwrap(),
            // /work/b 保留
            StateBlock::new(StateZone::Fs, "/work/b", json!({"v": 1})).unwrap(),
            // counter 缺失
            // goal 保留，另加一块
            StateBlock::new(StateZone::Context, "goal", json!({"v": 1})).unwrap(),
            StateBlock::new(StateZone::Context, "injected", json!(true)).unwrap(),
        ],
    )
    .unwrap();
    let report = compare(&source, &damaged).unwrap();
    assert!(!report.is_clean());
    assert_eq!(report.found(FindingCode::Modified).len(), 1);
    assert_eq!(report.found(FindingCode::Missing).len(), 1);
    assert_eq!(report.found(FindingCode::Extra).len(), 1);
    let fs = report
        .zones
        .iter()
        .find(|z| z.zone == StateZone::Fs)
        .unwrap();
    assert_eq!(fs.modified, vec!["/work/a".to_string()]);
    let memory = report
        .zones
        .iter()
        .find(|z| z.zone == StateZone::Memory)
        .unwrap();
    assert_eq!(memory.missing, vec!["counter".to_string()]);
    assert!(summarize(&report).contains("memory"));
}

#[test]
fn store_level_damage_is_found_without_going_through_read_snapshot() {
    let keys = agent(3);
    let expected = snap(&keys, 1);
    let mut store = MemoryStore::new();
    write_snapshot(&mut store, "live:", &expected).unwrap();
    assert!(compare_store(&store, "live:", &expected)
        .unwrap()
        .is_clean());

    store.put("live:fs:/work/a", json!({"v": 42})).unwrap();
    store.remove("live:memory:counter").unwrap();
    store.put("live:context:extra", json!(1)).unwrap();
    store.put("live:not-a-zone-key", json!(1)).unwrap();
    let report = compare_store(&store, "live:", &expected).unwrap();
    assert!(!report.is_clean());
    assert_eq!(report.found(FindingCode::Modified).len(), 1);
    assert_eq!(report.found(FindingCode::Missing).len(), 1);
    assert_eq!(report.found(FindingCode::Extra).len(), 1);
    assert_eq!(report.found(FindingCode::BadKey).len(), 1);
    // 观测到的内容根与期望不同，且被如实报告出来。
    assert_ne!(
        report.observed_content_root.as_deref(),
        Some(report.expected_content_root.as_str())
    );
    // 4 块原始 -1 删除 +1 多余 = 4（坏键不计入块，只计入 findings）。
    assert_eq!(report.observed_blocks, 4);
}

#[test]
fn a_clean_node_audit_reports_no_orphans_or_intent() {
    let keys = agent(4);
    let base = snap(&keys, 1);
    let mut node = NodeStore::open(
        NodeId::new("node-b").unwrap(),
        keys.did(),
        MemoryStore::new(),
    )
    .unwrap();
    node.install(&base, true).unwrap();
    let audit = audit_node(&node).unwrap();
    assert!(audit.is_clean(), "{:?}", audit.findings);
    assert_eq!(audit.live_content_root, base.content_root().unwrap());
    assert_eq!(audit.live_blocks, 4);
    assert!(audit.orphan_generations.is_empty());
    assert!(audit.intent.is_none());
    assert_eq!(audit.to_value()["head"], json!(audit.head));
}

#[test]
fn an_unfinished_migration_is_visible_in_the_node_audit() {
    let keys = agent(5);
    let base = snap(&keys, 1);
    let mut node = NodeStore::open(
        NodeId::new("node-b").unwrap(),
        keys.did(),
        MemoryStore::new(),
    )
    .unwrap();
    node.install(&base, true).unwrap();
    node.store_mut()
        .put(
            INTENT_KEY,
            json!({"tx": "feed", "staged": 9, "previous": 2, "target": "deadbeef"}),
        )
        .unwrap();
    let audit = audit_node(&node).unwrap();
    assert!(!audit.is_clean());
    assert_eq!(audit.found(FindingCode::UnfinishedMigration).len(), 1);
    assert_eq!(audit.found(FindingCode::IntentMismatch).len(), 1);
}

#[test]
fn after_a_2pc_commit_source_and_target_agree_block_by_block() {
    let keys = agent(6);
    let base = snap(&keys, 1);
    let target = snap(&keys, 2);
    let delta = StateDelta::between(&base, &target).unwrap();
    let signed = SignedSnapshot::sign(target.clone(), &keys).unwrap();
    let plan = MigrationPlan::new(
        &keys.did(),
        &NodeId::new("node-a").unwrap(),
        &NodeId::new("node-b").unwrap(),
        base.content_root().unwrap(),
        target.content_root().unwrap(),
        delta.id().unwrap(),
        1,
    )
    .unwrap();
    let mut node = NodeStore::open(
        NodeId::new("node-b").unwrap(),
        keys.did(),
        MemoryStore::new(),
    )
    .unwrap();
    node.install(&base, true).unwrap();
    let outcome = migrate(&mut node, plan, &delta, &signed, &mut FaultInjector::none()).unwrap();
    assert!(outcome.is_confirmed());

    let live = node.live_snapshot().unwrap();
    let report = compare(&target, &live).unwrap();
    assert!(report.is_clean());
    // 逐块比对，而不只是总哈希。
    assert_eq!(live.blocks(), target.blocks());
    assert_eq!(live.content_root().unwrap(), target.content_root().unwrap());
    assert!(audit_node(&node).unwrap().is_clean());
}

#[test]
fn a_rolled_back_node_still_agrees_with_the_base() {
    let keys = agent(7);
    let base = snap(&keys, 1);
    let target = snap(&keys, 2);
    let delta = StateDelta::between(&base, &target).unwrap();
    let signed = SignedSnapshot::sign(target.clone(), &keys).unwrap();
    let plan = MigrationPlan::new(
        &keys.did(),
        &NodeId::new("node-a").unwrap(),
        &NodeId::new("node-b").unwrap(),
        base.content_root().unwrap(),
        target.content_root().unwrap(),
        delta.id().unwrap(),
        1,
    )
    .unwrap();
    let mut node = NodeStore::open(
        NodeId::new("node-b").unwrap(),
        keys.did(),
        MemoryStore::new(),
    )
    .unwrap();
    node.install(&base, true).unwrap();
    let outcome = migrate(
        &mut node,
        plan,
        &delta,
        &signed,
        &mut FaultInjector::at(au4a_state::FaultPoint::AfterStaging),
    )
    .unwrap();
    assert!(!outcome.is_confirmed());
    // 回滚之后的节点必须与 base 完全一致——一致性检查能证明这件事。
    let live = node.live_snapshot().unwrap();
    assert!(compare(&base, &live).unwrap().is_clean());
    assert!(!compare(&target, &live).unwrap().is_clean());
    assert!(audit_node(&node).unwrap().is_clean());
}

#[test]
fn zone_roots_are_the_units_of_diagnosis() {
    let keys = agent(8);
    let base = snap(&keys, 1);
    // 只改 memory 区一块：其他两个区的摘要必须不变，便于定位。
    let mut blocks = three_zone(1);
    blocks.retain(|b| !(b.zone() == StateZone::Memory && b.key() == "counter"));
    blocks.push(StateBlock::new(StateZone::Memory, "counter", json!(2)).unwrap());
    let changed = StateSnapshot::capture(&keys.did(), "node-b", 1, blocks).unwrap();

    let report = compare(&base, &changed).unwrap();
    let fs = report
        .zones
        .iter()
        .find(|z| z.zone == StateZone::Fs)
        .unwrap();
    let memory = report
        .zones
        .iter()
        .find(|z| z.zone == StateZone::Memory)
        .unwrap();
    let context = report
        .zones
        .iter()
        .find(|z| z.zone == StateZone::Context)
        .unwrap();
    assert!(fs.equal && context.equal, "未改动的区摘要必须保持相等");
    assert!(!memory.equal);
    assert_eq!(memory.modified, vec!["counter".to_string()]);
}

#[test]
fn self_check_covers_consistency() {
    let checks = self_check();
    // 自检项只增不减：这一版至少要有 v1.3.5 的 13 条。
    assert!(checks.len() >= 13, "{checks:?}");
    assert!(au4a_core::all_passed(&checks), "{checks:?}");
    let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"consistency.clean"));
    assert!(names.contains(&"consistency.locates_damage"));
}

#[test]
fn scenario_reports_clean_consistency_and_localized_damage() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let keys = agent(9);
    kernel
        .register(&keys, "carrier", &["state.verify"], Credits(20))
        .unwrap();
    let out = au4a_state::scenario(&mut kernel).unwrap();
    assert_eq!(out["consistency"]["identical"], json!(true));
    assert_eq!(out["consistency"]["changed_blocks"], json!(0));
    assert_eq!(out["consistency"]["zones"].as_array().unwrap().len(), 3);
    assert_eq!(out["consistency"]["node_findings"], json!(0));
    assert_eq!(out["consistency"]["damage_detected"]["modified"], json!(1));
    assert_eq!(out["consistency"]["damage_detected"]["missing"], json!(1));
    assert!(out["consistency"]["summary"]
        .as_str()
        .unwrap()
        .starts_with("一致"));
    kernel.ledger().check_conservation().unwrap();
}

#[test]
fn scenario_is_still_replayable() {
    let mut k1 = Kernel::new(KernelConfig::default());
    let mut k2 = Kernel::new(KernelConfig::default());
    let a = au4a_state::scenario(&mut k1).unwrap();
    let b = au4a_state::scenario(&mut k2).unwrap();
    assert_eq!(a, b);
    let _: Did = agent(10).did();
}
