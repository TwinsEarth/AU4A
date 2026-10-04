//! v1.3.2 集成测试：块级增量 diff 与可续跑的双节点传输。
//!
//! 证据等级说明：传输是**本地双节点内存实现**（`cpu-proto`）——协议语义完整、可重放、
//! 失败可注入；但它**不是**真实网络，没有真实丢包/TLS/对端作恶。文档里必须这样写。

use au4a_core::{AgentKeys, CoreError, Credits, Did};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_state::{
    decode_frame, encode_frame, pull_into_session, sample_state, send_chunks, send_delta,
    AcceptOutcome, DeltaChunk, LocalNetwork, NodeId, StateBlock, StateDelta, StateSnapshot,
    StateZone, TransferSession, MAX_FRAME,
};
use serde_json::json;

fn agent(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

fn snap(agent: &Did, node: &str, epoch: u64, blocks: Vec<StateBlock>) -> StateSnapshot {
    StateSnapshot::capture(agent, node, epoch, blocks).expect("capture")
}

fn evolved() -> Vec<StateBlock> {
    vec![
        StateBlock::new(StateZone::Fs, "/work/notes.md", json!({"sha256": "77cc", "bytes": 12})).unwrap(),
        StateBlock::new(StateZone::Memory, "last_task", json!("summarize")).unwrap(),
        StateBlock::new(StateZone::Memory, "extra", json!({"n": 1})).unwrap(),
        StateBlock::new(StateZone::Context, "goal", json!({"text": "在 B 上继续"})).unwrap(),
        StateBlock::new(StateZone::Context, "done", json!(["snapshot", "transfer"])).unwrap(),
    ]
}

#[test]
fn diff_covers_all_three_zones_with_set_and_del() {
    let keys = agent(1);
    let from = snap(&keys.did(), "node-a", 1, sample_state().unwrap());
    let to = snap(&keys.did(), "node-a", 1, evolved());
    let delta = StateDelta::between(&from, &to).unwrap();

    for zone in ["fs", "memory", "context"] {
        let sets = delta.per_zone()[zone]["set"].as_u64().unwrap();
        let dels = delta.per_zone()[zone]["del"].as_u64().unwrap();
        assert!(sets > 0, "{zone} 应有 set");
        assert!(dels > 0, "{zone} 应有 del");
    }
    let applied = delta.apply_to(&from).unwrap();
    assert_eq!(applied.content_root().unwrap(), to.content_root().unwrap());
    delta.validate().unwrap();
}

#[test]
fn delta_json_roundtrip_and_tamper_detection() {
    let keys = agent(2);
    let from = snap(&keys.did(), "node-a", 1, sample_state().unwrap());
    let to = snap(&keys.did(), "node-a", 1, evolved());
    let delta = StateDelta::between(&from, &to).unwrap();
    let value = delta.to_value().unwrap();
    assert_eq!(StateDelta::from_value(&value).unwrap(), delta);

    // 改一个 set 的 value，但保留摘要 → 反序列化必须拒绝。
    let mut bad = value.clone();
    bad["set"][0]["value"] = json!({"hijacked": true});
    assert!(StateDelta::from_value(&bad).is_err());

    // 改 from_content_root → 结构自洽但应用时会拒绝（base 不是这个起点）。
    let mut replayed = value.clone();
    replayed["from_content_root"] = json!(to.content_root().unwrap());
    let forged = StateDelta::from_value(&replayed).unwrap();
    assert_eq!(forged.apply_to(&from), Err(CoreError::InvalidSignature));
}

#[test]
fn chunked_transfer_over_two_local_nodes_reproduces_the_source() {
    let keys = agent(3);
    let from = snap(&keys.did(), "node-a", 1, sample_state().unwrap());
    let to = snap(&keys.did(), "node-a", 1, evolved());
    let delta = StateDelta::between(&from, &to).unwrap();
    let chunks = delta.chunked(2).unwrap();

    let na = NodeId::new("node-a").unwrap();
    let nb = NodeId::new("node-b").unwrap();
    let mut net = LocalNetwork::new(&[na.clone(), nb.clone()]);
    let report = send_delta(&mut net, &na, &nb, &delta, 2, 0).unwrap();
    assert_eq!(report.chunks, chunks.len());
    assert_eq!(report.frames, chunks.len() as u64);

    let mut session = TransferSession::open(&chunks[0]).unwrap();
    for frame in net.take(&nb).unwrap() {
        let chunk = serde_json::from_value::<DeltaChunk>(decode_frame(&frame).unwrap()).unwrap();
        session.accept(&chunk).unwrap();
    }
    assert!(session.is_complete(chunks.len()));
    let rebuilt = session.assemble(chunks.len(), keys.did()).unwrap();
    assert_eq!(rebuilt.id().unwrap(), delta.id().unwrap());
    let target = rebuilt.apply_moved(&from, "node-b", 99).unwrap();
    assert_eq!(target.content_root().unwrap(), to.content_root().unwrap());
    assert_eq!(target.source_node(), "node-b");
    assert_ne!(target.root(), to.root());
}

#[test]
fn a_dropped_transfer_resumes_from_the_break_point() {
    let keys = agent(4);
    let from = snap(&keys.did(), "node-a", 1, sample_state().unwrap());
    let to = snap(
        &keys.did(),
        "node-a",
        1,
        (0..20)
            .map(|i| StateBlock::new(StateZone::Memory, format!("k{i:02}"), json!(i)).unwrap())
            .collect(),
    );
    let delta = StateDelta::between(&from, &to).unwrap();
    let chunks = delta.chunked(4).unwrap();
    let total = chunks.len();

    let na = NodeId::new("node-a").unwrap();
    let nb = NodeId::new("node-b").unwrap();
    let mut net = LocalNetwork::new(&[na.clone(), nb.clone()]);

    // 第一段：前 2 块送达。
    send_chunks(&mut net, &na, &nb, &delta, 4, 0, 2).unwrap();
    let (session, accepted_first, _) = pull_into_session(&mut net, &nb, None).unwrap();
    assert_eq!(accepted_first, 2);
    assert_eq!(session.resume_from(), 2);
    // 第二段：全部在「断线」中丢失。
    send_chunks(&mut net, &na, &nb, &delta, 4, 2, usize::MAX).unwrap();
    assert_eq!(net.drop_pending(&nb).unwrap(), total - 2);
    assert_eq!(session.missing(total), (2..total).collect::<Vec<_>>());

    // 续跑：只补发缺的块（不重头来）。
    let resume =
        send_chunks(&mut net, &na, &nb, &delta, 4, session.resume_from(), usize::MAX).unwrap();
    assert_eq!(resume.resumed_from, 2);
    assert_eq!(resume.frames, (total - 2) as u64);
    let (session, accepted, duplicates) = pull_into_session(&mut net, &nb, Some(session)).unwrap();
    assert_eq!(accepted, total - 2);
    assert_eq!(duplicates, 0);
    assert!(session.is_complete(total));

    let rebuilt = session.assemble(total, keys.did()).unwrap();
    assert_eq!(rebuilt.id().unwrap(), delta.id().unwrap());
    assert_eq!(
        rebuilt.apply_to(&from).unwrap().content_root().unwrap(),
        to.content_root().unwrap()
    );
}

#[test]
fn replayed_chunks_are_idempotent_and_conflicts_are_refused() {
    let keys = agent(5);
    let from = snap(&keys.did(), "node-a", 1, sample_state().unwrap());
    let to = snap(&keys.did(), "node-a", 1, evolved());
    let delta = StateDelta::between(&from, &to).unwrap();
    let chunks = delta.chunked(3).unwrap();

    let mut session = TransferSession::open(&chunks[0]).unwrap();
    assert_eq!(session.accept(&chunks[0]).unwrap(), AcceptOutcome::Accepted);
    // 网络重发是常态：重复块幂等接受，不当成攻击。
    assert_eq!(session.accept(&chunks[0]).unwrap(), AcceptOutcome::Duplicate);
    assert_eq!(session.duplicates(), 1);
}

#[test]
fn a_session_refuses_chunks_from_another_transfer() {
    let keys = agent(6);
    let from = snap(&keys.did(), "node-a", 1, sample_state().unwrap());
    let to = snap(&keys.did(), "node-a", 1, evolved());
    let delta_a = StateDelta::between(&from, &to).unwrap();
    let delta_b = StateDelta::between(&to, &from).unwrap();
    let chunk_a = &delta_a.chunked(3).unwrap()[0];
    let chunk_b = &delta_b.chunked(3).unwrap()[0];
    let mut session = TransferSession::open(chunk_a).unwrap();
    assert_eq!(session.accept(chunk_b), Err(CoreError::InvalidVersion));
}

#[test]
fn frames_are_bounded_and_prefix_checked() {
    let value = json!({"blob": "x".repeat(1024)});
    let frame = encode_frame(&value).unwrap();
    assert_eq!(u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize, frame.len() - 4);
    assert_eq!(decode_frame(&frame).unwrap(), value);
    assert_eq!(decode_frame(&frame[..3]), Err(CoreError::FrameTruncated));
    let mut oversized = Vec::from((MAX_FRAME as u32 + 1).to_be_bytes());
    oversized.extend_from_slice(b"{}");
    assert_eq!(decode_frame(&oversized), Err(CoreError::FrameTooLarge));
}

#[test]
fn node_ids_are_validated_and_unknown_nodes_refused() {
    assert!(NodeId::new("node-a").is_ok());
    assert_eq!(NodeId::new(""), Err(CoreError::Encoding));
    let na = NodeId::new("node-a").unwrap();
    let nb = NodeId::new("node-b").unwrap();
    let mut net = LocalNetwork::new(&[na.clone()]);
    assert_eq!(net.deliver(&na, &nb, vec![]), Err(CoreError::UnknownAgent));
    assert_eq!(net.take(&nb), Err(CoreError::UnknownAgent));
    assert_eq!(net.drop_pending(&nb), Err(CoreError::UnknownAgent));
}

#[test]
fn cross_agent_deltas_are_refused() {
    let a = agent(7);
    let b = agent(8);
    let from = snap(&a.did(), "node-a", 1, sample_state().unwrap());
    let to = snap(&b.did(), "node-b", 1, sample_state().unwrap());
    assert_eq!(StateDelta::between(&from, &to), Err(CoreError::InvalidDid));
}

#[test]
fn scenario_transfers_a_delta_and_rebuilds_on_the_target_node() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let keys = agent(9);
    kernel.register(&keys, "carrier", &["state.transfer"], Credits(20)).unwrap();
    let out = au4a_state::scenario(&mut kernel).unwrap();
    assert_eq!(out["migration_identical"], json!(true));
    assert_eq!(out["source_content_root"], out["target_content_root"]);
    assert_eq!(out["delta"]["ops"], json!(8));
    assert_eq!(out["transfer"]["resumed_from"], json!(2));
    assert_eq!(out["transfer"]["dropped_frames"], json!(2));
    assert_eq!(out["transfer"]["duplicates"], json!(0));
    assert_eq!(out["stale_base_refused"], json!(true));
    kernel.ledger().check_conservation().unwrap();
}

#[test]
fn scenario_is_replayable_across_kernels() {
    let mut k1 = Kernel::new(KernelConfig::default());
    let mut k2 = Kernel::new(KernelConfig::default());
    let a = au4a_state::scenario(&mut k1).unwrap();
    let b = au4a_state::scenario(&mut k2).unwrap();
    assert_eq!(a, b, "同样的种子必须给同样的结果");
}
