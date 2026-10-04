//! v1.3.8 集成测试：全链路 + 篡改矩阵 + 边界与拒绝路径。
//!
//! 这一版不加新功能，只做一件事：**把「能跑通」变成「跑不通的情况都被试过」**。
//! 三个矩阵：
//! 1. 篡改矩阵：逐个块 / 逐个块边界 / 逐个存储键地破坏，看是否**恰好**被检出。
//! 2. 故障矩阵：链路每个阶段注入故障，看是否**恰好**回滚。
//! 3. 边界矩阵：空状态、单块、Unicode 键、大 value、超限输入。

use au4a_core::{AgentKeys, CoreError, Credits, Did};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_state::{
    compare_store, materialize, plan, run_chain, self_check, write_snapshot, ChainOptions,
    DeltaChunk, FaultPoint, FindingCode, MemoryStore, NodeId, SignedSnapshot, StateBlock,
    StateDelta, StateSnapshot, StateStore, StateZone, UdosObject,
};
use serde_json::json;

fn agent(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

fn rich_state(seed: u8, v: i64) -> StateSnapshot {
    let blocks = vec![
        StateBlock::new(StateZone::Fs, "/work/a.md", json!({"v": v, "bytes": 10 * v})).unwrap(),
        StateBlock::new(StateZone::Fs, "/work/b.json", json!({"steps": [v, v + 1]})).unwrap(),
        StateBlock::new(StateZone::Memory, "last_task", json!(format!("task-{v}"))).unwrap(),
        StateBlock::new(StateZone::Memory, "counter", json!(v)).unwrap(),
        StateBlock::new(StateZone::Context, "goal", json!({"text": "迁移", "v": v})).unwrap(),
        StateBlock::new(StateZone::Context, "todo", json!(["a", "b"])).unwrap(),
    ];
    StateSnapshot::capture(&agent(seed).did(), "node-a", v as u64, blocks).unwrap()
}

// ---------------------------------------------------------------- 全链路

#[test]
fn the_full_chain_holds_end_to_end() {
    let keys = agent(1);
    let report = run_chain(
        &keys,
        &rich_state(1, 1),
        &rich_state(1, 2),
        &NodeId::new("node-b").unwrap(),
        &ChainOptions::default(),
    )
    .unwrap();
    assert!(report.is_ok(), "{report:#?}");
    assert_eq!(report.live_content_root, report.source_content_root);
    assert_ne!(report.base_content_root, report.source_content_root);
    assert!(report.consistency.clean);
    assert_eq!(report.node_audit_findings, 0);
    assert!(report.udos_object_id.starts_with("sha256:"));
    assert!(report.work.ops_skipped > 0, "链路里必须真的续跑过");
    assert!(report.delta_plan.unchanged > 0);
    assert_eq!(report.delta_ops, report.delta_plan.op_count());
}

#[test]
fn every_stage_fault_rolls_back_and_stays_consistent() {
    let keys = agent(2);
    let base = rich_state(2, 1);
    let target = rich_state(2, 2);
    for point in FaultPoint::ALL {
        let options = ChainOptions {
            fault: Some(point),
            ..ChainOptions::default()
        };
        let report = run_chain(
            &keys,
            &base,
            &target,
            &NodeId::new("node-b").unwrap(),
            &options,
        )
        .unwrap();
        assert!(!report.committed, "{point:?}");
        assert!(report.rollback_clean, "{point:?}");
        assert!(report.consistency.clean, "{point:?}");
        assert_eq!(report.live_content_root, report.base_content_root, "{point:?}");
        assert!(report.is_ok(), "{point:?}");
    }
}

#[test]
fn the_chain_reports_are_byte_identical_across_runs() {
    let run = || {
        let keys = agent(3);
        run_chain(
            &keys,
            &rich_state(3, 1),
            &rich_state(3, 4),
            &NodeId::new("node-b").unwrap(),
            &ChainOptions::default(),
        )
        .unwrap()
        .to_value()
    };
    assert_eq!(run(), run());
}

// ---------------------------------------------------------------- 篡改矩阵

#[test]
fn flipping_any_single_block_is_detected() {
    let keys = agent(4);
    let snapshot = rich_state(4, 1);
    let signed = SignedSnapshot::sign(snapshot.clone(), &keys).unwrap();
    for index in 0..snapshot.blocks().len() {
        let mut value = signed.to_value().unwrap();
        value["snapshot"]["blocks"][index]["value"] = json!({"flipped": true});
        assert!(
            SignedSnapshot::from_value(&value).is_err(),
            "第 {index} 块被改却没被检出"
        );
    }
    // 原始快照依然可用（矩阵没有污染输入）。
    signed.verify().unwrap();
}

#[test]
fn flipping_any_single_store_value_is_located_exactly() {
    let expected = rich_state(5, 1);
    let mut store = MemoryStore::new();
    write_snapshot(&mut store, "live:", &expected).unwrap();
    let keys_list: Vec<String> = store.keys().map(|k| k.to_string()).collect();
    assert_eq!(keys_list.len(), expected.blocks().len());
    for key in &keys_list {
        let mut probe = MemoryStore::new();
        write_snapshot(&mut probe, "live:", &expected).unwrap();
        probe.put(key, json!({"tampered": true})).unwrap();
        let report = compare_store(&probe, "live:", &expected).unwrap();
        assert!(!report.is_clean(), "{key} 被改却没被检出");
        assert_eq!(report.found(FindingCode::Modified).len(), 1, "{key}");
        assert_eq!(report.found(FindingCode::Missing).len(), 0, "{key}");
        assert_eq!(report.found(FindingCode::Extra).len(), 0, "{key}");
    }
}

#[test]
fn deleting_any_single_store_key_is_located_exactly() {
    let expected = rich_state(6, 1);
    let original: Vec<String> = {
        let mut store = MemoryStore::new();
        write_snapshot(&mut store, "live:", &expected).unwrap();
        store.keys().map(|k| k.to_string()).collect()
    };
    for key in &original {
        let mut probe = MemoryStore::new();
        write_snapshot(&mut probe, "live:", &expected).unwrap();
        probe.remove(key).unwrap();
        let report = compare_store(&probe, "live:", &expected).unwrap();
        assert_eq!(report.found(FindingCode::Missing).len(), 1, "{key}");
        assert_eq!(report.found(FindingCode::Modified).len(), 0, "{key}");
    }
}

#[test]
fn tampering_with_any_single_chunk_is_detected() {
    let from = rich_state(7, 1);
    let to = rich_state(7, 2);
    let delta = StateDelta::between(&from, &to).unwrap();
    let chunks = delta.chunked(1).unwrap();
    assert!(chunks.len() > 1);
    for chunk in &chunks {
        let mut value = chunk.to_value().unwrap();
        if let Some(first) = value["set"].as_array_mut().and_then(|s| s.first_mut()) {
            first["value"] = json!({"flipped": true});
        } else {
            value["del"][0]["key"] = json!("flipped");
        }
        assert!(
            serde_json::from_value::<DeltaChunk>(value).is_err(),
            "第 {} 块被改却没被检出",
            chunk.index()
        );
    }
}

#[test]
fn tampering_with_the_signature_or_the_object_id_is_detected() {
    let keys = agent(8);
    let snapshot = rich_state(8, 1);
    let signed = SignedSnapshot::sign(snapshot.clone(), &keys).unwrap();

    let mut bad_sig = signed.to_value().unwrap();
    bad_sig["sig"] = json!("00".repeat(64));
    assert!(SignedSnapshot::from_value(&bad_sig).is_err());

    let object = au4a_state::snapshot_to_object(&snapshot).unwrap();
    let mut bad_object = object.clone();
    bad_object["object_id"] = json!(format!("sha256:{}", "11".repeat(32)));
    assert!(UdosObject::validate(&bad_object).is_err());
    // 原对象仍然有效。
    assert!(UdosObject::validate(&object).is_ok());
}

// ---------------------------------------------------------------- 边界矩阵

#[test]
fn an_empty_state_travels_and_installs_cleanly() {
    let keys = agent(9);
    let empty = StateSnapshot::capture(&keys.did(), "node-a", 1, vec![]).unwrap();
    let one = StateSnapshot::capture(
        &keys.did(),
        "node-a",
        1,
        vec![StateBlock::new(StateZone::Fs, "/a", json!(1)).unwrap()],
    )
    .unwrap();
    // 空 → 一块
    let report = run_chain(
        &keys,
        &empty,
        &one,
        &NodeId::new("node-b").unwrap(),
        &ChainOptions::default(),
    )
    .unwrap();
    assert!(report.is_ok(), "{report:#?}");
    assert_eq!(report.source_content_root, one.content_root().unwrap());
    // 一块 → 空
    let report = run_chain(
        &keys,
        &one,
        &empty,
        &NodeId::new("node-b").unwrap(),
        &ChainOptions::default(),
    )
    .unwrap();
    assert!(report.is_ok(), "{report:#?}");
    assert_eq!(report.live_content_root, empty.content_root().unwrap());
}

#[test]
fn a_state_with_no_change_is_a_no_op_chain() {
    let keys = agent(10);
    let same = rich_state(10, 1);
    let report = run_chain(
        &keys,
        &same,
        &same,
        &NodeId::new("node-b").unwrap(),
        &ChainOptions {
            drop_rest: false,
            ..ChainOptions::default()
        },
    )
    .unwrap();
    // 没有差异时传输为空，但 2PC 仍然把 base 装好并确认。
    assert_eq!(report.delta_ops, 0);
    assert!(report.is_ok(), "{report:#?}");
    assert_eq!(report.live_content_root, report.base_content_root);
}

#[test]
fn unicode_keys_and_control_characters_roundtrip() {
    let keys = agent(11);
    let blocks = vec![
        StateBlock::new(StateZone::Fs, "/工作/笔记-📝.md", json!({"文本": "迁移\n完成"})).unwrap(),
        StateBlock::new(StateZone::Context, "目标", json!({"说明": "从 A 到 B\t好"})).unwrap(),
    ];
    let snapshot = StateSnapshot::capture(&keys.did(), "node-a", 1, blocks).unwrap();
    snapshot.verify().unwrap();
    let object = au4a_state::snapshot_to_object(&snapshot).unwrap();
    let back = au4a_state::object_to_snapshot(&object).unwrap();
    assert_eq!(back.content_root().unwrap(), snapshot.content_root().unwrap());

    let mut store = MemoryStore::new();
    write_snapshot(&mut store, "live:", &snapshot).unwrap();
    let moved = au4a_state::read_snapshot(&store, "live:", &keys.did(), "node-b", 9).unwrap();
    assert_eq!(moved.content_root().unwrap(), snapshot.content_root().unwrap());
}

#[test]
fn a_single_block_state_is_a_valid_chain_endpoint() {
    let keys = agent(12);
    let one = StateSnapshot::capture(
        &keys.did(),
        "node-a",
        1,
        vec![StateBlock::new(StateZone::Memory, "only", json!({"x": 1})).unwrap()],
    )
    .unwrap();
    let two = StateSnapshot::capture(
        &keys.did(),
        "node-a",
        2,
        vec![StateBlock::new(StateZone::Memory, "only", json!({"x": 2})).unwrap()],
    )
    .unwrap();
    let report = run_chain(
        &keys,
        &one,
        &two,
        &NodeId::new("node-b").unwrap(),
        &ChainOptions {
            chunk_ops: 1,
            first_batch_chunks: 0,
            ..ChainOptions::default()
        },
    )
    .unwrap();
    assert!(report.is_ok(), "{report:#?}");
}

#[test]
fn oversized_and_malformed_inputs_are_refused() {
    // 单块超过 64 KiB。
    let huge = "x".repeat(au4a_state::MAX_VALUE_LEN);
    assert_eq!(
        StateBlock::new(StateZone::Fs, "/huge", json!({"blob": huge})),
        Err(CoreError::FrameTooLarge)
    );
    // 浮点。
    assert_eq!(
        StateBlock::new(StateZone::Memory, "f", json!(1.5)),
        Err(CoreError::FloatForbidden)
    );
    // 空 key / 控制字符 key。
    assert_eq!(StateBlock::new(StateZone::Memory, "", json!(1)), Err(CoreError::Encoding));
    assert_eq!(
        StateBlock::new(StateZone::Memory, "a\u{1}b", json!(1)),
        Err(CoreError::Encoding)
    );
    // 超块数。
    let many: Vec<StateBlock> = (0..au4a_state::MAX_BLOCKS + 1)
        .filter_map(|i| StateBlock::new(StateZone::Memory, format!("k{i}"), json!(i)).ok())
        .collect();
    assert_eq!(
        StateSnapshot::capture(&agent(13).did(), "node-a", 1, many),
        Err(CoreError::FrameTooLarge)
    );
}

#[test]
fn the_plan_and_materialization_agree_with_the_delta() {
    let from = rich_state(14, 1);
    let to = rich_state(14, 3);
    let delta_plan = plan(&from, &to).unwrap();
    let mut counter = au4a_state::WorkCounter::new();
    let materialized = materialize(&from, &to, &delta_plan, &mut counter).unwrap();
    let direct = StateDelta::between(&from, &to).unwrap();
    assert_eq!(materialized.id().unwrap(), direct.id().unwrap());
    assert_eq!(counter.blocks_materialized, delta_plan.set.len() as u64);
}

#[test]
fn self_check_covers_the_full_chain() {
    let checks = self_check();
    assert!(checks.len() >= 18, "{checks:?}");
    assert!(au4a_core::all_passed(&checks), "{checks:#?}");
    let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"chain.full_migration"));
}

#[test]
fn scenario_still_passes_all_previous_guarantees() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let keys = agent(15);
    kernel.register(&keys, "carrier", &["state.chain"], Credits(20)).unwrap();
    let out = au4a_state::scenario(&mut kernel).unwrap();
    // 每一版的保证都在同一个 JSON 里，且互不矛盾。
    assert_eq!(out["identical"], json!(true));
    assert_eq!(out["migration_identical"], json!(true));
    assert_eq!(out["tamper_rejected"], json!(true));
    assert_eq!(out["signature"]["tamper_refused"], json!(true));
    assert_eq!(out["two_phase"]["rollback_clean"], json!(true));
    assert_eq!(out["consistency"]["identical"], json!(true));
    assert_eq!(out["udos"]["roundtrip_identical"], json!(true));
    assert_eq!(out["perf"]["second_capture_hashes"], json!(0));
    assert_eq!(out["track"], json!("1.3"));
    let _: Did = keys.did();
    kernel.ledger().check_conservation().unwrap();
}
