//! v1.6.1 经验库集成测试：可寻址、有界、可移植、可重放。
//!
//! 覆盖三类路径：正常路径（记录/查询/往返）、拒绝路径（非法字段/非法容量/篡改输入）、
//! 不变式（内容键唯一、FIFO 淘汰有计数、同种子两次运行逐字节一致）。

use au4a_core::{AgentKeys, CoreError, Credits, Did};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_learning::experience::{Experience, ExperienceStore, Outcome, RecordOutcome};
use au4a_learning::rng::SplitMix64;

fn dids(n: u8) -> Vec<Did> {
    (0..n)
        .map(|i| AgentKeys::from_seed(&[0x50 + i; 32]).did())
        .collect()
}

fn exp(id: &str, kind: &str, outcome: Outcome, reward: i64, peers: &[Did]) -> Experience {
    Experience::new(
        id,
        kind,
        &format!("context for {id}"),
        "quote-then-deliver",
        outcome,
        Credits(reward),
        1,
        peers,
    )
    .unwrap()
}

#[test]
fn records_are_content_addressed_and_deduplicated() {
    let peers = dids(2);
    let mut store = ExperienceStore::new(8).unwrap();
    let first = exp("t-1", "translate.en-zh", Outcome::Success, 10, &peers);
    let key = first.key().unwrap();

    assert!(matches!(
        store.record(first.clone()).unwrap(),
        RecordOutcome::Added { .. }
    ));
    // 同一条经验（字节相同）第二次记录被识别为重复，而不是产生第二条数据。
    match store.record(first).unwrap() {
        RecordOutcome::Duplicate { key: dup } => assert_eq!(dup, key),
        other => panic!("expected duplicate, got {other:?}"),
    }
    assert_eq!(store.len(), 1);
    assert_eq!(store.duplicates(), 1);
    assert!(store.contains(&key));
    assert_eq!(store.get(&key).unwrap().task_id, "t-1");
}

#[test]
fn capacity_evicts_oldest_and_reports_it() {
    let peers = dids(1);
    let mut store = ExperienceStore::new(3).unwrap();
    for i in 0..4 {
        let e = exp(&format!("t-{i}"), "summarize.zh", Outcome::Partial, 5, &peers);
        let result = store.record(e).unwrap();
        if i < 3 {
            assert!(matches!(result, RecordOutcome::Added { .. }));
        } else {
            // 淘汰必须被调用方看到：学习系统不允许静默丢数据。
            assert!(matches!(result, RecordOutcome::Evicted { .. }));
        }
    }
    assert_eq!(store.len(), 3);
    assert_eq!(store.evictions(), 1);
    assert_eq!(store.entries()[0].task_id, "t-1");
    assert_eq!(store.entries()[2].task_id, "t-3");
    assert_eq!(store.stats().by_outcome.get("partial"), Some(&3));
}

#[test]
fn malformed_experience_is_refused() {
    let peers = dids(2);
    // 空 task_id
    assert_eq!(
        Experience::new("", "t", "c", "a", Outcome::Success, Credits(0), 1, &[]),
        Err(CoreError::InvalidKind)
    );
    // 非法任务类型字符（含空格）
    assert_eq!(
        Experience::new("id", "bad type", "c", "a", Outcome::Success, Credits(0), 1, &[]),
        Err(CoreError::InvalidKind)
    );
    // 超长上下文
    assert_eq!(
        Experience::new(
            "id",
            "t",
            &"x".repeat(au4a_learning::experience::MAX_CONTEXT_LEN + 1),
            "a",
            Outcome::Success,
            Credits(0),
            1,
            &[]
        ),
        Err(CoreError::InvalidKind)
    );
    // 空上下文
    assert_eq!(
        Experience::new("id", "t", "", "a", Outcome::Success, Credits(0), 1, &[]),
        Err(CoreError::InvalidKind)
    );
    // 上下文含控制字符
    assert_eq!(
        Experience::new("id", "t", "a\u{7}b", "a", Outcome::Success, Credits(0), 1, &[]),
        Err(CoreError::InvalidKind)
    );
    // 协作者超上限
    let too_many = dids(17);
    assert_eq!(
        Experience::new("id", "t", "c", "a", Outcome::Success, Credits(0), 1, &too_many),
        Err(CoreError::InvalidKind)
    );
    // 手工构造的未排序协作者列表必须被 validate 拒绝（内容键唯一性的前提）
    let mut raw = exp("id", "t", Outcome::Success, 1, &peers);
    raw.peer_agents.reverse();
    assert_eq!(raw.validate(), Err(CoreError::InvalidKind));
    // 容量 0 的库无法记录任何经验
    assert_eq!(ExperienceStore::new(0), Err(CoreError::InvalidKind));
}

#[test]
fn peers_are_normalized_sorted_and_deduplicated() {
    let peers = dids(3);
    let mut shuffled = vec![peers[2].clone(), peers[0].clone(), peers[1].clone(), peers[0].clone()];
    let e = Experience::new("id", "t", "c", "a", Outcome::Success, Credits(1), 1, &shuffled).unwrap();
    let mut sorted = peers.clone();
    sorted.sort();
    assert_eq!(e.peer_agents, sorted);
    assert_eq!(e.peer_count(), 3);

    // 顺序不同、集合相同 → 同一个内容键
    shuffled = vec![peers[1].clone(), peers[2].clone(), peers[0].clone()];
    let e2 = Experience::new("id", "t", "c", "a", Outcome::Success, Credits(1), 1, &shuffled).unwrap();
    assert_eq!(e.key().unwrap(), e2.key().unwrap());
}

#[test]
fn storage_is_byte_identical_across_two_independent_runs() {
    let build = || {
        let peers = dids(2);
        let mut store = ExperienceStore::new(32).unwrap();
        let mut rng = SplitMix64::new(0x6_1601);
        for i in 0..16u64 {
            let outcome = match rng.below(3) {
                0 => Outcome::Success,
                1 => Outcome::Partial,
                _ => Outcome::Failure,
            };
            let id = format!("t-{i:03}");
            store
                .record(exp(&id, "classify.zh", outcome, rng.range_i64(0, 50), &peers))
                .unwrap();
        }
        store
    };
    let a = build();
    let b = build();
    assert_eq!(a.canonical_json().unwrap(), b.canonical_json().unwrap());
    assert_eq!(a.digest().unwrap(), b.digest().unwrap());
    assert_eq!(a.digest().unwrap().len(), 64);
}

#[test]
fn canonical_json_roundtrips_and_tampering_is_refused() {
    let peers = dids(2);
    let mut store = ExperienceStore::new(8).unwrap();
    for i in 0..5 {
        store
            .record(exp(&format!("t-{i}"), "translate.en-zh", Outcome::Success, 3, &peers))
            .unwrap();
    }
    let json = store.canonical_json().unwrap();
    let restored = ExperienceStore::from_json_str(&json).unwrap();
    assert_eq!(restored.digest().unwrap(), store.digest().unwrap());
    assert_eq!(restored.len(), store.len());
    assert_eq!(restored.capacity(), store.capacity());

    // 篡改 1：序列化里塞入重复条目 → 拒绝（载入路径重建索引后条目数不一致）
    let mut value = store.to_value().unwrap();
    let entries = value.get_mut("entries").unwrap().as_array_mut().unwrap();
    let first = entries[0].clone();
    entries.push(first);
    assert_eq!(
        ExperienceStore::from_json_str(&value.to_string()),
        Err(CoreError::InvalidKind)
    );

    // 篡改 2：非法字段 → 拒绝
    let mut value2 = store.to_value().unwrap();
    value2["entries"][0]["task_id"] = serde_json::json!("");
    assert_eq!(
        ExperienceStore::from_json_str(&value2.to_string()),
        Err(CoreError::InvalidKind)
    );

    // 篡改 3：容量小于条目数 → 拒绝
    let mut value3 = store.to_value().unwrap();
    value3["capacity"] = serde_json::json!(1);
    assert_eq!(
        ExperienceStore::from_json_str(&value3.to_string()),
        Err(CoreError::InvalidKind)
    );

    // 篡改 4：不是 JSON → Encoding
    assert_eq!(
        ExperienceStore::from_json_str("{not json"),
        Err(CoreError::Encoding)
    );
}

#[test]
fn outcome_parsing_and_quality_are_exact() {
    for o in [Outcome::Success, Outcome::Partial, Outcome::Failure] {
        assert_eq!(Outcome::parse(o.as_str()), Some(o));
    }
    assert_eq!(Outcome::parse("maybe"), None);
    assert_eq!(Outcome::Success.quality_bp(), 10_000);
    assert_eq!(Outcome::Partial.quality_bp(), 5_000);
    assert_eq!(Outcome::Failure.quality_bp(), 0);
    assert!(Outcome::Success.is_success() && !Outcome::Failure.is_success());
}

#[test]
fn by_task_type_and_recent_windows_are_deterministic() {
    let peers = dids(1);
    let mut store = ExperienceStore::new(16).unwrap();
    for i in 0..6 {
        let kind = if i % 2 == 0 { "translate.en-zh" } else { "summarize.zh" };
        store
            .record(exp(&format!("t-{i}"), kind, Outcome::Success, 1, &peers))
            .unwrap();
    }
    assert_eq!(store.by_task_type("translate.en-zh").count(), 3);
    assert_eq!(store.task_types(), vec!["summarize.zh", "translate.en-zh"]);
    assert_eq!(store.recent(2).len(), 2);
    assert_eq!(store.recent(2)[1].task_id, "t-5");
    assert_eq!(store.stats().len, 6);
}

#[test]
fn track_self_check_is_green_and_scenario_is_reproducible() {
    let checks = au4a_learning::self_check();
    assert!(checks.len() >= 5, "自检项过少：{}", checks.len());
    assert!(
        au4a_core::all_passed(&checks),
        "自检未全绿：{:?}",
        checks.iter().filter(|c| !c.passed).collect::<Vec<_>>()
    );

    let mut k1 = Kernel::new(KernelConfig::default());
    let mut k2 = Kernel::new(KernelConfig::default());
    let a = au4a_learning::scenario(&mut k1).unwrap();
    let b = au4a_learning::scenario(&mut k2).unwrap();
    assert_eq!(a, b, "同一内核配置下 scenario 必须给出相同结果");
    assert_eq!(a["experiences"], 24);
    assert_eq!(k1.agent_count(), 1);
    assert_eq!(k1.ledger().balance(&AgentKeys::from_seed(&[0x16; 32]).did()).locked, Credits(10));
    k1.ledger().check_conservation().unwrap();
    // 幂等：同一内核上再跑一次不报 DuplicateAgent
    let c = au4a_learning::scenario(&mut k1).unwrap();
    assert_eq!(c["experiences"], 24);
    assert_eq!(k1.agent_count(), 1);

    // 进度事件必须真的产生（观察层「看进度」的数据源）
    assert!(k1.observe().progress.iter().any(|e| e.kind == "1.6.experience.collect"));
}

#[test]
fn results_json_reports_passing_checks() {
    let r = au4a_learning::results_json().unwrap();
    assert_eq!(r["track"], "1.6");
    assert_eq!(r["version"], au4a_learning::VERSION);
    assert_eq!(r["all_passed"], serde_json::json!(true));
    assert_eq!(r["checks"], serde_json::json!(au4a_learning::self_check().len()));
    assert_eq!(r["scenario"]["experiences"], 24);
    // 摘要必须能被规范 JSON 编码（禁止浮点），观察层按整数读
    au4a_core::canonicalize(&r).unwrap();
}
