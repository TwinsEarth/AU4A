//! v1.3.7 集成测试：性能优化（确定性的工作量核算）。
//!
//! 本轨道禁止读墙钟，因此「更快」由**可复算的工作量**证明：哈希次数、物化块数、
//! 续跑省下的操作数。同样的输入给同样的数字——优化本身也因此可被证伪。

use au4a_core::{AgentKeys, Credits};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_state::{
    materialize, measure_receive, measure_transfer, plan, resume_savings, self_check,
    DigestCache, LocalNetwork, NodeId, StateBlock, StateDelta, StateSnapshot, StateZone,
    WorkCounter,
};
use serde_json::json;

fn agent(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

fn raw(n: usize, v: i64) -> Vec<(StateZone, String, serde_json::Value)> {
    (0..n)
        .map(|i| (StateZone::Memory, format!("k{i:02}"), json!(v)))
        .collect()
}

fn snapshot(keys: &AgentKeys, v: i64, n: usize, epoch: u64) -> StateSnapshot {
    StateSnapshot::capture(
        &keys.did(),
        "node-a",
        epoch,
        (0..n)
            .map(|i| StateBlock::new(StateZone::Memory, format!("k{i:02}"), json!(v)).unwrap())
            .collect(),
    )
    .unwrap()
}

#[test]
fn a_second_capture_of_unchanged_state_does_no_hashing_at_all() {
    let keys = agent(1);
    let mut cache = DigestCache::new();
    let mut first = WorkCounter::new();
    let a = cache
        .capture_raw(&keys.did(), "node-a", 1, raw(32, 7), &mut first)
        .unwrap();
    let mut second = WorkCounter::new();
    let b = cache
        .capture_raw(&keys.did(), "node-a", 1, raw(32, 7), &mut second)
        .unwrap();
    assert_eq!(first.block_hashes, 32);
    assert_eq!(second.block_hashes, 0);
    assert_eq!(second.hashes_reused, 32);
    assert_eq!(second.reuse_ratio_bp(), 10_000);
    assert_eq!(cache.len(), 32);
    // 正确性没有被优化牺牲：root 与逐块内容完全一致，且整体自洽。
    assert_eq!(a.root(), b.root());
    assert_eq!(a.blocks(), b.blocks());
    b.verify().unwrap();
    assert_eq!(
        b.content_root().unwrap(),
        snapshot(&keys, 7, 32, 1).content_root().unwrap()
    );
}

#[test]
fn changing_one_block_rehashes_exactly_one_block() {
    let keys = agent(2);
    let mut cache = DigestCache::new();
    let mut counter = WorkCounter::new();
    let a = cache
        .capture_raw(&keys.did(), "node-a", 1, raw(16, 1), &mut counter)
        .unwrap();
    let mut changed = raw(16, 1);
    changed[9].2 = json!(2);
    let mut second = WorkCounter::new();
    let b = cache
        .capture_raw(&keys.did(), "node-a", 2, changed, &mut second)
        .unwrap();
    assert_eq!(second.block_hashes, 1);
    assert_eq!(second.hashes_reused, 15);
    assert_ne!(a.content_root().unwrap(), b.content_root().unwrap());
    b.verify().unwrap();
}

#[test]
fn the_plan_names_the_changed_blocks_without_cloning_values() {
    let keys = agent(3);
    let from = snapshot(&keys, 1, 50, 1);
    let mut blocks: Vec<StateBlock> = (0..50)
        .map(|i| {
            let v = if i == 7 || i == 33 { json!(2) } else { json!(1) };
            StateBlock::new(StateZone::Memory, format!("k{i:02}"), v).unwrap()
        })
        .collect();
    blocks.retain(|b| b.key() != "k11");
    blocks.push(StateBlock::new(StateZone::Fs, "/new", json!(true)).unwrap());
    let to = StateSnapshot::capture(&keys.did(), "node-a", 2, blocks).unwrap();

    let delta_plan = plan(&from, &to).unwrap();
    assert_eq!(delta_plan.set.len(), 3, "两块被改 + 一块新增");
    assert_eq!(delta_plan.del.len(), 1);
    assert_eq!(delta_plan.unchanged, 47);
    assert_eq!(delta_plan.op_count(), 4);

    let mut counter = WorkCounter::new();
    let delta = materialize(&from, &to, &delta_plan, &mut counter).unwrap();
    // 只物化了 3 个 set 块；47 个未变块一次 value 克隆都没有发生。
    assert_eq!(counter.blocks_materialized, 3);
    assert_eq!(delta.set().len(), 3);
    assert_eq!(
        delta.apply_to(&from).unwrap().content_root().unwrap(),
        to.content_root().unwrap()
    );
}

#[test]
fn resuming_costs_strictly_less_than_restarting() {
    let keys = agent(4);
    let from = snapshot(&keys, 1, 2, 1);
    let to = snapshot(&keys, 2, 40, 2);
    let delta = StateDelta::between(&from, &to).unwrap();
    let total_chunks = delta.chunked(4).unwrap().len();
    let na = NodeId::new("node-a").unwrap();
    let nb = NodeId::new("node-b").unwrap();

    // 全量重传的对照。
    let mut full = LocalNetwork::new(&[na.clone(), nb.clone()]);
    let mut full_work = WorkCounter::new();
    let frames_full = measure_transfer(&mut full, &na, &nb, &delta, 4, 0, &mut full_work).unwrap();
    assert_eq!(frames_full, total_chunks);
    assert_eq!(full_work.ops_skipped, 0);

    // 续跑：先收到 3 块，其余丢失，然后补缺口。
    let mut net = LocalNetwork::new(&[na.clone(), nb.clone()]);
    let mut work = WorkCounter::new();
    au4a_state::send_chunks(&mut net, &na, &nb, &delta, 4, 0, 3).unwrap();
    let (session, accepted) = measure_receive(&mut net, &nb, None, &mut work).unwrap();
    assert_eq!(accepted, 3);
    net.drop_pending(&nb).unwrap();
    let frames = measure_transfer(&mut net, &na, &nb, &delta, 4, session.resume_from(), &mut work)
        .unwrap();
    assert_eq!(frames, total_chunks - 3);
    assert!(work.ops_skipped > 0);
    assert!(work.ops_transferred < delta.op_count() as u64);
    let (session, _) = measure_receive(&mut net, &nb, Some(session), &mut work).unwrap();
    let rebuilt = session.assemble(total_chunks, keys.did()).unwrap();
    assert_eq!(
        rebuilt.apply_to(&from).unwrap().content_root().unwrap(),
        to.content_root().unwrap()
    );
}

#[test]
fn work_counters_are_replayable_and_have_no_time_field() {
    let keys = agent(5);
    let run = || {
        let mut cache = DigestCache::new();
        let mut counter = WorkCounter::new();
        let _ = cache
            .capture_raw(&keys.did(), "node-a", 1, raw(8, 3), &mut counter)
            .unwrap();
        let _ = cache
            .capture_raw(&keys.did(), "node-a", 1, raw(8, 3), &mut counter)
            .unwrap();
        counter
    };
    let a = run();
    let b = run();
    assert_eq!(a, b);
    let value = au4a_state::perf::summary(&a);
    assert_eq!(value["wall_clock_used"], json!(false));
    assert_eq!(value["total_digest_lookups"], json!(16));
    assert_eq!(value["reuse_ratio_bp"], json!(5000));
    assert_eq!(a.reuse_ratio_bp(), 5_000);
}

#[test]
fn resume_savings_arithmetic_is_exact() {
    // 100 个操作、每块 4 个操作、从第 10 块续跑 → 省 40，传 60。
    assert_eq!(resume_savings(100, 10, 4), (60, 40));
    // 边界：从头开始 → 一点没省。
    assert_eq!(resume_savings(100, 0, 4), (100, 0));
    // 边界：超出总量时不出现负数。
    assert_eq!(resume_savings(10, 100, 4), (0, 400));
}

#[test]
fn the_cache_never_reuses_a_digest_for_a_different_value() {
    let keys = agent(6);
    let mut cache = DigestCache::new();
    let mut counter = WorkCounter::new();
    let a = cache
        .capture_raw(&keys.did(), "node-a", 1, raw(4, 1), &mut counter)
        .unwrap();
    // 同一个键、不同的值：必须是新的摘要，不能命中。
    let mut changed = raw(4, 1);
    changed[0].2 = json!(999);
    let mut second = WorkCounter::new();
    let b = cache
        .capture_raw(&keys.did(), "node-a", 1, changed, &mut second)
        .unwrap();
    assert_eq!(second.block_hashes, 1);
    assert_eq!(second.hashes_reused, 3);
    assert_ne!(a.blocks()[0].digest(), b.blocks()[0].digest());
    a.verify().unwrap();
    b.verify().unwrap();
}

#[test]
fn self_check_covers_performance_claims() {
    let checks = self_check();
    assert!(checks.len() >= 17, "{checks:?}");
    assert!(au4a_core::all_passed(&checks), "{checks:?}");
    let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"perf.digest_cache_reuse"));
    assert!(names.contains(&"perf.resume_saves_work"));
}

#[test]
fn scenario_reports_deterministic_work_counters() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let keys = agent(7);
    kernel.register(&keys, "carrier", &["state.perf"], Credits(20)).unwrap();
    let out = au4a_state::scenario(&mut kernel).unwrap();
    assert_eq!(out["perf"]["second_capture_hashes"], json!(0));
    assert_eq!(out["perf"]["hashes_reused"], json!(6));
    assert_eq!(out["perf"]["reuse_ratio_bp"], json!(10000));
    assert_eq!(out["perf"]["counter"]["wall_clock_used"], json!(false));
    assert!(out["perf"]["ops_skipped"].as_u64().unwrap() > 0);
    assert_eq!(out["perf"]["delta_plan"]["ops"], json!(8));
    kernel.ledger().check_conservation().unwrap();
}

#[test]
fn scenario_is_replayable_including_the_counters() {
    let mut k1 = Kernel::new(KernelConfig::default());
    let mut k2 = Kernel::new(KernelConfig::default());
    let a = au4a_state::scenario(&mut k1).unwrap();
    let b = au4a_state::scenario(&mut k2).unwrap();
    assert_eq!(a["perf"], b["perf"], "工作量数字也必须可重放");
    assert_eq!(a, b);
}
