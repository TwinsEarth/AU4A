//! v1.3.1 集成测试：三区状态快照的内容寻址、存储往返与拒绝路径。
//!
//! 这一层的断言就是轨道 1.3 的地基：**同一份状态必须得到同一个 root**，
//! 且任何被替换/篡改的字节都不能被当成「同一份状态」。

use au4a_core::{AgentKeys, CoreError, Credits, Did};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_state::{
    read_snapshot, sample_state, self_check, write_snapshot, MemoryStore, StateBlock,
    StateSnapshot, StateStore, StateZone,
};
use serde_json::json;

fn agent(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

fn snapshot(agent: &Did, node: &str, epoch: u64) -> StateSnapshot {
    StateSnapshot::capture(agent, node, epoch, sample_state().expect("sample state"))
        .expect("capture")
}

#[test]
fn a_snapshot_is_content_addressed_and_insertion_order_free() {
    let keys = agent(1);
    let a = snapshot(&keys.did(), "node-a", 3);
    let mut reversed = sample_state().unwrap();
    reversed.reverse();
    let b = StateSnapshot::capture(&keys.did(), "node-a", 3, reversed).unwrap();
    assert_eq!(a.root(), b.root());
    assert_eq!(a.blocks().len(), 6);
    a.verify().expect("self-consistent snapshot");
}

#[test]
fn all_three_zones_survive_capture() {
    let keys = agent(2);
    let snap = snapshot(&keys.did(), "node-a", 1);
    assert_eq!(snap.block_count(StateZone::Fs), 2);
    assert_eq!(snap.block_count(StateZone::Memory), 2);
    assert_eq!(snap.block_count(StateZone::Context), 2);
    let roots = snap.zone_roots().unwrap();
    assert_eq!(roots.len(), 3);
    assert_ne!(roots[0].1, roots[1].1);
    assert_ne!(roots[1].1, roots[2].1);
    assert!(snap.block(StateZone::Context, "goal").is_some());
    assert!(snap.block(StateZone::Context, "missing").is_none());
}

#[test]
fn an_empty_zone_still_has_a_stable_root() {
    let keys = agent(3);
    let only_fs = vec![StateBlock::new(StateZone::Fs, "/a", json!({"x": 1})).unwrap()];
    let snap = StateSnapshot::capture(&keys.did(), "node-a", 1, only_fs).unwrap();
    assert_eq!(snap.block_count(StateZone::Memory), 0);
    // 空区 root 是确定值，不是「不存在」——否则「三区完整恢复」无法被断言。
    assert_eq!(snap.zone_root(StateZone::Memory).unwrap(), snap.zone_root(StateZone::Memory).unwrap());
    assert_eq!(snap.zone_root(StateZone::Memory).unwrap().len(), 64);
}

#[test]
fn store_roundtrip_is_byte_identical() {
    let keys = agent(4);
    let before = snapshot(&keys.did(), "node-a", 9);
    let mut store = MemoryStore::new();
    assert_eq!(write_snapshot(&mut store, "live:", &before).unwrap(), 6);
    assert_eq!(store.len(), 6);
    // 同一出处读回：文档 root 与逐块内容都一致。
    let after = read_snapshot(&store, "live:", &keys.did(), "node-a", 9).unwrap();
    assert_eq!(before.root(), after.root());
    assert!(before.same_state(&after));
    assert_eq!(before.blocks(), after.blocks());
    // 跨节点读回：状态内容一致（迁移等价性），出处不同所以文档 root 不同。
    let moved = read_snapshot(&store, "live:", &keys.did(), "node-b", 42).unwrap();
    assert_eq!(before.content_root().unwrap(), moved.content_root().unwrap());
    assert_ne!(before.root(), moved.root());
    assert!(before.same_content(&moved));
}

#[test]
fn namespaces_isolate_two_agents_in_one_store() {
    let a = agent(5);
    let b = agent(6);
    let sa = snapshot(&a.did(), "node-a", 1);
    let sb = snapshot(&b.did(), "node-a", 1);
    let mut store = MemoryStore::new();
    write_snapshot(&mut store, "a:", &sa).unwrap();
    write_snapshot(&mut store, "b:", &sb).unwrap();
    assert_eq!(store.list("a:").unwrap().len(), 6);
    let ra = read_snapshot(&store, "a:", &a.did(), "node-a", 1).unwrap();
    let rb = read_snapshot(&store, "b:", &b.did(), "node-a", 1).unwrap();
    assert_eq!(ra.root(), sa.root());
    assert_eq!(rb.root(), sb.root());
    assert_ne!(ra.root(), rb.root(), "两个 Agent 的状态必须可区分");
}

#[test]
fn floats_and_oversized_values_are_refused() {
    assert_eq!(
        StateBlock::new(StateZone::Memory, "p", json!(0.1)),
        Err(CoreError::FloatForbidden)
    );
    let huge = "x".repeat(au4a_state::MAX_VALUE_LEN);
    assert_eq!(
        StateBlock::new(StateZone::Fs, "/huge", json!({"blob": huge})),
        Err(CoreError::FrameTooLarge)
    );
    assert_eq!(
        StateBlock::new(StateZone::Memory, "", json!(1)),
        Err(CoreError::Encoding)
    );
}

#[test]
fn duplicate_keys_and_too_many_blocks_are_refused() {
    let keys = agent(7);
    let mut blocks = sample_state().unwrap();
    blocks.push(StateBlock::new(StateZone::Memory, "last_task", json!("dup")).unwrap());
    assert_eq!(
        StateSnapshot::capture(&keys.did(), "node-a", 1, blocks),
        Err(CoreError::Encoding)
    );

    let many: Vec<StateBlock> = (0..au4a_state::MAX_BLOCKS + 1)
        .filter_map(|i| StateBlock::new(StateZone::Memory, format!("k{i}"), json!(i)).ok())
        .collect();
    assert_eq!(
        StateSnapshot::capture(&keys.did(), "node-a", 1, many),
        Err(CoreError::FrameTooLarge)
    );
}

#[test]
fn a_replaced_snapshot_is_refused() {
    let keys = agent(8);
    let original = snapshot(&keys.did(), "node-a", 1);
    let mut other_blocks = sample_state().unwrap();
    other_blocks.push(StateBlock::new(StateZone::Memory, "injected", json!(true)).unwrap());
    let other = StateSnapshot::capture(&keys.did(), "node-a", 1, other_blocks).unwrap();

    // 攻击者保留原 root，但换成另一份块集合。
    let mut forged = original.to_value().unwrap();
    forged["blocks"] = other.to_value().unwrap()["blocks"].clone();
    assert_eq!(StateSnapshot::from_value(&forged), Err(CoreError::InvalidSignature));

    // 攻击者改 root 让「两个字段自洽」，但 root 与实际内容不符。
    let mut rehashed = forged.clone();
    rehashed["root"] = json!(other.root());
    // 这份其实是另一份完整快照——必须能通过，因为它是自洽的；
    // 但它的 root ≠ 原快照 root，替换者无法让两者相等。
    let accepted = StateSnapshot::from_value(&rehashed).unwrap();
    assert_ne!(accepted.root(), original.root());
}

#[test]
fn store_rejects_non_canonical_values_before_they_enter() {
    let mut store = MemoryStore::new();
    assert_eq!(store.put("memory:x", json!(1.5)), Err(CoreError::FloatForbidden));
    assert_eq!(store.get("memory:x").unwrap(), None);
}

#[test]
fn track_self_check_is_green() {
    let checks = self_check();
    assert_eq!(checks.len(), 7);
    assert!(au4a_core::all_passed(&checks), "{checks:?}");
    assert!(checks.iter().all(|c| c.track == "1.3"));
}

#[test]
fn scenario_runs_the_full_snapshot_chain_on_a_shared_kernel() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let keys = agent(9);
    kernel.register(&keys, "carrier", &["state.snapshot"], Credits(20)).unwrap();
    let out = au4a_state::scenario(&mut kernel).unwrap();
    assert_eq!(out["track"], json!("1.3"));
    assert_eq!(out["identical"], json!(true));
    assert_eq!(out["tamper_rejected"], json!(true));
    assert_eq!(out["migration_identical"], json!(true));
    assert_eq!(out["source_content_root"], out["target_content_root"]);
    assert_eq!(out["blocks"]["fs"], json!(2));
    assert_eq!(out["blocks"]["memory"], json!(2));
    assert_eq!(out["blocks"]["context"], json!(2));
    assert_eq!(out["transfer"]["evidence_grade"], json!("cpu-proto"));
    assert!(out["before_root"].as_str().unwrap().len() == 64);
    kernel.ledger().check_conservation().unwrap();
}

#[test]
fn scenario_is_replayable() {
    let mut k1 = Kernel::new(KernelConfig::default());
    let mut k2 = Kernel::new(KernelConfig::default());
    let a = au4a_state::scenario(&mut k1).unwrap();
    let b = au4a_state::scenario(&mut k2).unwrap();
    assert_eq!(a, b, "同样的种子必须给同样的结果");
}

#[test]
fn store_trait_is_object_safe_for_swappable_backends() {
    let mut store = MemoryStore::new();
    let dynamic: &mut dyn StateStore = &mut store;
    dynamic.put("memory:k", json!({"v": 1})).unwrap();
    assert_eq!(dynamic.list("memory:").unwrap(), vec!["memory:k".to_string()]);
    assert_eq!(dynamic.len(), 1);
}
