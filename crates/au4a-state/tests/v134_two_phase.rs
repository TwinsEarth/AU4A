//! v1.3.4 集成测试：两阶段提交恢复协议。
//!
//! 这一版的核心断言不是「迁移成功了」，而是：
//! **任意阶段失败之后，目标节点的 live 状态要么是完整的 base，要么是完整的 target，
//! 不存在第三种可能。** 部分状态是最危险的失败模式，因为它无法被自检发现。

use au4a_core::{AgentKeys, CoreError, Credits, Did};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_state::{
    migrate, self_check, FaultInjector, FaultPoint, MemoryStore, Migration, MigrationOutcome,
    MigrationPlan, NodeId, NodeStore, Phase, SignedSnapshot, StateBlock, StateDelta, StateSnapshot,
    StateZone,
};
use serde_json::json;

fn agent(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

fn versioned(v: i64) -> Vec<StateBlock> {
    vec![
        StateBlock::new(StateZone::Fs, "/work/a", json!({"v": v})).unwrap(),
        StateBlock::new(StateZone::Memory, "counter", json!(v)).unwrap(),
        StateBlock::new(StateZone::Context, "goal", json!({"v": v})).unwrap(),
    ]
}

struct Rig {
    base: StateSnapshot,
    target: StateSnapshot,
    delta: StateDelta,
    signed: SignedSnapshot,
    plan: MigrationPlan,
    node: NodeStore<MemoryStore>,
}

fn rig(seed: u8) -> Rig {
    let keys = agent(seed);
    let did = keys.did();
    let base = StateSnapshot::capture(&did, "node-a", 1, versioned(1)).unwrap();
    let target = StateSnapshot::capture(&did, "node-a", 1, versioned(2)).unwrap();
    let delta = StateDelta::between(&base, &target).unwrap();
    let signed = SignedSnapshot::sign(target.clone(), &keys).unwrap();
    let plan = MigrationPlan::new(
        &did,
        &NodeId::new("node-a").unwrap(),
        &NodeId::new("node-b").unwrap(),
        base.content_root().unwrap(),
        target.content_root().unwrap(),
        delta.id().unwrap(),
        1,
    )
    .unwrap();
    let mut node = NodeStore::open(NodeId::new("node-b").unwrap(), did, MemoryStore::new()).unwrap();
    node.install(&base, true).unwrap();
    Rig {
        base,
        target,
        delta,
        signed,
        plan,
        node,
    }
}

#[test]
fn a_happy_path_runs_prepare_commit_confirm_in_order() {
    let r = rig(1);
    let mut node = r.node.clone();
    let mut m = Migration::begin(r.plan.clone());
    let prepared = m
        .prepare(&mut node, &r.delta, &r.signed, &mut FaultInjector::none())
        .unwrap();
    assert_eq!(m.phase(), Phase::Prepared);
    assert_eq!(prepared.target_content_root, r.target.content_root().unwrap());
    assert_eq!(prepared.blocks, 3);
    // prepare 之后 live 仍然是 base！
    assert_eq!(node.live_content_root().unwrap(), r.base.content_root().unwrap());

    let committed = m.commit(&mut node, &mut FaultInjector::none()).unwrap();
    assert_eq!(m.phase(), Phase::Committed);
    assert_eq!(committed.live_content_root, r.target.content_root().unwrap());
    assert_eq!(node.live_content_root().unwrap(), r.target.content_root().unwrap());

    let confirmed = m.confirm(&mut node, &mut FaultInjector::none()).unwrap();
    assert_eq!(m.phase(), Phase::Confirmed);
    assert_eq!(confirmed.live_content_root, r.target.content_root().unwrap());
    assert!(node.intent().unwrap().is_none());
    assert!(node.orphan_generations(node.head().unwrap()).unwrap().is_empty());
}

#[test]
fn a_fault_at_every_point_rolls_back_without_partial_state() {
    let base_root = rig(2).base.content_root().unwrap();
    for point in FaultPoint::ALL {
        let r = rig(2);
        let mut node = r.node.clone();
        let outcome = migrate(
            &mut node,
            r.plan.clone(),
            &r.delta,
            &r.signed,
            &mut FaultInjector::at(point),
        )
        .unwrap();
        match outcome {
            MigrationOutcome::RolledBack {
                state_restored,
                orphans,
                reason,
                ..
            } => {
                assert!(state_restored, "{point:?}");
                assert_eq!(orphans, 0, "{point:?}");
                assert_eq!(reason, CoreError::Overflow);
            }
            MigrationOutcome::Confirmed { .. } => panic!("{point:?} 的故障没有生效"),
        }
        assert_eq!(node.live_content_root().unwrap(), base_root, "{point:?}");
        assert_eq!(node.live_snapshot().unwrap().blocks().len(), 3, "{point:?}");
        assert!(node.intent().unwrap().is_none(), "{point:?}");
    }
}

#[test]
fn live_never_mixes_base_and_target_blocks() {
    let r = rig(3);
    let base_blocks: Vec<(String, String)> = r
        .base
        .blocks()
        .iter()
        .map(|b| (b.zone().as_str().to_string(), b.digest().to_string()))
        .collect();
    let target_blocks: Vec<(String, String)> = r
        .target
        .blocks()
        .iter()
        .map(|b| (b.zone().as_str().to_string(), b.digest().to_string()))
        .collect();

    for point in FaultPoint::ALL {
        let mut node = r.node.clone();
        let _ = migrate(
            &mut node,
            r.plan.clone(),
            &r.delta,
            &r.signed,
            &mut FaultInjector::at(point),
        )
        .unwrap();
        // 逐块比对：live 的块集合必须**整体等于** base 或整体等于 target。
        let live: Vec<(String, String)> = node
            .live_snapshot()
            .unwrap()
            .blocks()
            .iter()
            .map(|b| (b.zone().as_str().to_string(), b.digest().to_string()))
            .collect();
        assert!(
            live == base_blocks || live == target_blocks,
            "{point:?}: live 是混合状态 {live:?}"
        );
    }
}

#[test]
fn phase_guards_reject_out_of_order_calls() {
    let r = rig(4);
    let mut node = r.node.clone();
    let mut m = Migration::begin(r.plan.clone());
    assert_eq!(m.commit(&mut node, &mut FaultInjector::none()), Err(CoreError::InvalidVersion));
    assert_eq!(m.confirm(&mut node, &mut FaultInjector::none()), Err(CoreError::InvalidVersion));
    assert_eq!(m.rollback(&mut node), Ok(0));
    assert_eq!(m.phase(), Phase::RolledBack);

    let mut m2 = Migration::begin(r.plan.clone());
    m2.prepare(&mut node, &r.delta, &r.signed, &mut FaultInjector::none())
        .unwrap();
    assert_eq!(
        m2.prepare(&mut node, &r.delta, &r.signed, &mut FaultInjector::none()),
        Err(CoreError::InvalidVersion)
    );
    m2.commit(&mut node, &mut FaultInjector::none()).unwrap();
    m2.confirm(&mut node, &mut FaultInjector::none()).unwrap();
    assert_eq!(m2.confirm(&mut node, &mut FaultInjector::none()), Err(CoreError::InvalidVersion));
}

#[test]
fn rollback_after_commit_flips_the_head_back() {
    let r = rig(5);
    let mut node = r.node.clone();
    let mut m = Migration::begin(r.plan.clone());
    m.prepare(&mut node, &r.delta, &r.signed, &mut FaultInjector::none())
        .unwrap();
    let base_gen = node.head().unwrap();
    m.commit(&mut node, &mut FaultInjector::none()).unwrap();
    let committed_gen = node.head().unwrap();
    assert_ne!(base_gen, committed_gen);
    assert_eq!(node.live_content_root().unwrap(), r.target.content_root().unwrap());

    m.rollback(&mut node).unwrap();
    assert_eq!(node.head().unwrap(), base_gen);
    assert_eq!(node.live_content_root().unwrap(), r.base.content_root().unwrap());
    assert!(node.orphan_generations(base_gen).unwrap().is_empty());
}

#[test]
fn a_wrong_base_is_refused_before_touching_the_target_node() {
    let r = rig(6);
    let mut node = r.node.clone();
    // 目标节点上先装一份别的状态（模拟竞争：另一笔迁移先落地）。
    node.install(&r.target, true).unwrap();
    let head_before = node.head().unwrap();
    let outcome = migrate(
        &mut node,
        r.plan.clone(),
        &r.delta,
        &r.signed,
        &mut FaultInjector::none(),
    )
    .unwrap();
    assert!(!outcome.is_confirmed());
    // 被拒绝的迁移不得改动任何东西。
    assert_eq!(node.head().unwrap(), head_before);
    assert_eq!(node.live_content_root().unwrap(), r.target.content_root().unwrap());
}

#[test]
fn a_wrong_signed_snapshot_blocks_the_migration() {
    let r = rig(7);
    let keys = agent(7);
    let mut node = r.node.clone();
    let head_before = node.head().unwrap();
    // 签名有效，但内容是另一份状态 → 语义闸门拒绝。
    let other = SignedSnapshot::sign(
        StateSnapshot::capture(&keys.did(), "node-a", 1, versioned(9)).unwrap(),
        &keys,
    )
    .unwrap();
    let outcome = migrate(
        &mut node,
        r.plan.clone(),
        &r.delta,
        &other,
        &mut FaultInjector::none(),
    )
    .unwrap();
    assert!(!outcome.is_confirmed());
    assert_eq!(node.head().unwrap(), head_before);
    assert_eq!(node.live_content_root().unwrap(), r.base.content_root().unwrap());
}

#[test]
fn the_plan_binds_agent_nodes_roots_delta_and_epoch() {
    let r = rig(8);
    let keys = agent(8);
    assert_eq!(r.plan.agent, keys.did());
    assert_eq!(r.plan.from_node, "node-a");
    assert_eq!(r.plan.to_node, "node-b");
    assert_eq!(r.plan.base_content_root, r.base.content_root().unwrap());
    assert_eq!(r.plan.target_content_root, r.target.content_root().unwrap());
    assert_eq!(r.plan.delta_id, r.delta.id().unwrap());

    let other = MigrationPlan::new(
        &keys.did(),
        &NodeId::new("node-a").unwrap(),
        &NodeId::new("node-b").unwrap(),
        r.plan.base_content_root.clone(),
        r.plan.target_content_root.clone(),
        r.plan.delta_id.clone(),
        2, // epoch 变了，tx 必须变
    )
    .unwrap();
    assert_ne!(other.tx, r.plan.tx);
}

#[test]
fn outcome_json_is_machine_readable_evidence() {
    let r = rig(9);
    let mut node = r.node.clone();
    let ok = migrate(
        &mut node,
        r.plan.clone(),
        &r.delta,
        &r.signed,
        &mut FaultInjector::none(),
    )
    .unwrap();
    let value = ok.to_value();
    assert_eq!(value["outcome"], json!("confirmed"));
    assert_eq!(value["report"]["blocks"], json!(3));
    assert_eq!(value["report"]["cleanup_removed"], json!(1));

    let mut node2 = r.node.clone();
    let bad = migrate(
        &mut node2,
        r.plan.clone(),
        &r.delta,
        &r.signed,
        &mut FaultInjector::at(FaultPoint::BeforeFlip),
    )
    .unwrap();
    let value = bad.to_value();
    assert_eq!(value["outcome"], json!("rolled_back"));
    assert_eq!(value["state_restored"], json!(true));
    assert_eq!(value["orphans"], json!(0));
}

#[test]
fn self_check_covers_two_phase_and_rollback() {
    let checks = self_check();
    assert_eq!(checks.len(), 11);
    assert!(au4a_core::all_passed(&checks), "{checks:?}");
    let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"migration.two_phase_commit"));
    assert!(names.contains(&"migration.rollback_clean"));
}

#[test]
fn scenario_commits_one_migration_and_rolls_back_another() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let keys = agent(10);
    kernel.register(&keys, "carrier", &["state.2pc"], Credits(20)).unwrap();
    let out = au4a_state::scenario(&mut kernel).unwrap();
    assert_eq!(out["two_phase"]["committed"], json!(true));
    assert_eq!(out["two_phase"]["target_live_content_root"], out["target_content_root"]);
    assert_eq!(out["two_phase"]["orphan_generations"], json!(0));
    assert_eq!(out["two_phase"]["rollback_clean"], json!(true));
    assert_eq!(out["two_phase"]["rollback_live_content_root"], out["before_content_root"]);
    assert_eq!(out["two_phase"]["outcome"]["outcome"], json!("rolled_back"));
    kernel.ledger().check_conservation().unwrap();
}

#[test]
fn a_second_migration_on_the_same_node_uses_a_new_generation() {
    let r = rig(11);
    let mut node = r.node.clone();
    let first = migrate(
        &mut node,
        r.plan.clone(),
        &r.delta,
        &r.signed,
        &mut FaultInjector::none(),
    )
    .unwrap();
    assert!(first.is_confirmed());
    let gen_after_first = node.head().unwrap();

    // 第二笔：v2 → v3。
    let keys = agent(11);
    let v3 = StateSnapshot::capture(&keys.did(), "node-a", 2, versioned(3)).unwrap();
    let delta2 = StateDelta::between(&r.target, &v3).unwrap();
    let signed2 = SignedSnapshot::sign(v3.clone(), &keys).unwrap();
    let plan2 = MigrationPlan::new(
        &keys.did(),
        &NodeId::new("node-a").unwrap(),
        &NodeId::new("node-b").unwrap(),
        r.target.content_root().unwrap(),
        v3.content_root().unwrap(),
        delta2.id().unwrap(),
        2,
    )
    .unwrap();
    let second = migrate(
        &mut node,
        plan2,
        &delta2,
        &signed2,
        &mut FaultInjector::none(),
    )
    .unwrap();
    assert!(second.is_confirmed());
    assert!(node.head().unwrap() > gen_after_first);
    assert_eq!(node.live_content_root().unwrap(), v3.content_root().unwrap());
    assert!(node.orphan_generations(node.head().unwrap()).unwrap().is_empty());
}

#[test]
fn migration_is_replayable_from_the_same_inputs() {
    let r1 = rig(12);
    let r2 = rig(12);
    let mut n1 = r1.node.clone();
    let mut n2 = r2.node.clone();
    let a = migrate(
        &mut n1,
        r1.plan.clone(),
        &r1.delta,
        &r1.signed,
        &mut FaultInjector::none(),
    )
    .unwrap();
    let b = migrate(
        &mut n2,
        r2.plan.clone(),
        &r2.delta,
        &r2.signed,
        &mut FaultInjector::none(),
    )
    .unwrap();
    assert_eq!(a.to_value(), b.to_value());
    assert_eq!(n1.live_content_root().unwrap(), n2.live_content_root().unwrap());
    let _: Did = r1.plan.agent.clone();
}
