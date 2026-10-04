//! v1.6.2 反馈机制集成测试：整数统计精确、样本阈值诚实、公开投影脱敏。

use au4a_core::{AgentKeys, Credits, Did};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_learning::experience::{Experience, ExperienceStore, Outcome};
use au4a_learning::feedback::{FeedbackAnalyser, MIN_SAMPLES};

fn dids(n: u8) -> Vec<Did> {
    (0..n)
        .map(|i| AgentKeys::from_seed(&[0x60 + i; 32]).did())
        .collect()
}

fn push(
    store: &mut ExperienceStore,
    id: &str,
    task_type: &str,
    outcome: Outcome,
    reward: i64,
    peers: &[Did],
) {
    let context = format!("ctx-{id}");
    let exp = Experience::new(
        id,
        task_type,
        &context,
        "deliver",
        outcome,
        Credits(reward),
        1,
        peers,
    )
    .unwrap();
    store.record(exp).unwrap();
}

#[test]
fn statistics_are_exact_integers_and_complementary() {
    let peers = dids(1);
    let mut store = ExperienceStore::new(16).unwrap();
    push(
        &mut store,
        "a",
        "translate.en-zh",
        Outcome::Success,
        40,
        &peers,
    );
    push(
        &mut store,
        "b",
        "translate.en-zh",
        Outcome::Success,
        40,
        &peers,
    );
    push(
        &mut store,
        "c",
        "translate.en-zh",
        Outcome::Partial,
        20,
        &peers,
    );
    push(
        &mut store,
        "d",
        "translate.en-zh",
        Outcome::Failure,
        0,
        &peers,
    );

    let report = FeedbackAnalyser::analyse(&store).unwrap();
    let o = &report.overall;
    assert_eq!(o.sample, 4);
    assert_eq!(o.success_bp, 5_000);
    assert_eq!(o.partial_bp, 2_500);
    assert_eq!(o.failure_bp, 2_500);
    assert_eq!(o.success_bp + o.partial_bp + o.failure_bp, 10_000);
    assert_eq!(o.quality_bp, 6_250); // (10000+10000+5000+0)/4
    assert_eq!(o.reward_total, Credits(100));
    assert_eq!(o.mean_reward, Credits(25));
    assert!(o.sufficient && o.confidence_bp == 10_000);
    assert_eq!(report.total, 4);
    assert_eq!(report.per_type.len(), 1);
    assert_eq!(report.per_type[0].scope, "translate.en-zh");
}

#[test]
fn three_way_split_never_loses_a_basis_point() {
    // 7 条里 2 成功 2 部分 3 失败：独立取整会得到 2857+2857+4285 = 9999；
    // 本实现用互补得到失败率，三者和恒为 10000。
    let mut store = ExperienceStore::new(16).unwrap();
    for i in 0..7 {
        let outcome = match i {
            0 | 1 => Outcome::Success,
            2 | 3 => Outcome::Partial,
            _ => Outcome::Failure,
        };
        push(&mut store, &format!("t-{i}"), "x", outcome, 7, &[]);
    }
    let report = FeedbackAnalyser::analyse(&store).unwrap();
    let o = &report.overall;
    assert_eq!(o.success_bp, 2_857);
    assert_eq!(o.partial_bp, 2_857);
    assert_eq!(o.failure_bp, 4_286);
    assert_eq!(o.success_bp + o.partial_bp + o.failure_bp, 10_000);
}

#[test]
fn low_sample_is_reported_as_insufficient_not_as_success() {
    let mut store = ExperienceStore::new(16).unwrap();
    push(&mut store, "t-1", "x", Outcome::Success, 10, &[]);
    let report = FeedbackAnalyser::analyse(&store).unwrap();
    assert!(!report.overall.sufficient);
    assert_eq!(report.overall.confidence_bp, 2_500);
    assert_eq!(au4a_learning::confidence_of(MIN_SAMPLES), 10_000);
    assert_eq!(au4a_learning::confidence_of(0), 0);
    assert_eq!(au4a_learning::confidence_of(1_000), 10_000);
}

#[test]
fn empty_store_yields_zeroes_not_an_error() {
    let store = ExperienceStore::new(4).unwrap();
    let report = FeedbackAnalyser::analyse(&store).unwrap();
    assert_eq!(report.total, 0);
    assert!(report.per_type.is_empty() && report.per_peer.is_empty());
    assert_eq!(report.overall.sample, 0);
    assert_eq!(report.overall.reward_total, Credits::ZERO);
    assert!(!report.overall.sufficient);
}

#[test]
fn peer_feedback_tracks_the_right_counterparties() {
    let peers = dids(3);
    let mut store = ExperienceStore::new(32).unwrap();
    // peer0：2 次成功；peer1：2 次失败；peer2：1 次部分成功
    push(&mut store, "a", "x", Outcome::Success, 30, &peers[..1]);
    push(&mut store, "b", "x", Outcome::Success, 20, &peers[..1]);
    push(&mut store, "c", "x", Outcome::Failure, 0, &peers[1..2]);
    push(&mut store, "d", "x", Outcome::Failure, 0, &peers[1..2]);
    push(&mut store, "e", "x", Outcome::Partial, 10, &peers[2..3]);

    let report = FeedbackAnalyser::analyse(&store).unwrap();
    assert_eq!(report.per_peer.len(), 3);
    let p0 = report.peer_stats(&peers[0]).unwrap();
    assert_eq!(p0.sample, 2);
    assert_eq!(p0.success_bp, 10_000);
    assert_eq!(p0.mean_reward, Credits(25));
    assert!(!p0.sufficient);
    let p1 = report.peer_stats(&peers[1]).unwrap();
    assert_eq!(p1.success_bp, 0);
    assert_eq!(p1.quality_bp, 0);
    let p2 = report.peer_stats(&peers[2]).unwrap();
    assert_eq!(p2.success_bp, 0);
    assert_eq!(p2.quality_bp, 5_000);

    // 多参与者任务：两个协作者都被计入
    let mut multi = ExperienceStore::new(8).unwrap();
    push(&mut multi, "m", "x", Outcome::Success, 5, &peers);
    let mr = FeedbackAnalyser::analyse(&multi).unwrap();
    assert_eq!(mr.per_peer.len(), 3);
    assert!(mr.per_peer.iter().all(|p| p.sample == 1));
}

#[test]
fn public_projection_never_leaks_peer_identities_or_context() {
    let peers = dids(2);
    let mut store = ExperienceStore::new(16).unwrap();
    for i in 0..5 {
        // 故意使用可被搜索的「秘密」上下文
        let context = format!("SECRET-CONTEXT-{i}-不可外泄");
        let exp = Experience::new(
            &format!("task-{i}"),
            "translate.en-zh",
            &context,
            "deliver",
            Outcome::Success,
            Credits(10),
            1,
            &peers[..1],
        )
        .unwrap();
        store.record(exp).unwrap();
    }
    let report = FeedbackAnalyser::analyse(&store).unwrap();
    let public = report.public_json().unwrap();
    let text = public.to_string();
    assert!(!text.contains("did:au4a:"), "公开投影泄露了 DID：{text}");
    assert!(!text.contains("SECRET-CONTEXT"), "公开投影泄露了上下文");
    assert!(public.get("per_peer").is_none());
    assert!(public.get("per_type").is_some());
    assert_eq!(public["peers_observed"], 1);
    assert_eq!(public["peers_sufficient"], 1);
    // 本地视图仍然保留明细（脱敏只发生在对外路径上）
    assert_eq!(report.per_peer[0].peer, peers[0]);
}

#[test]
fn report_is_deterministic_and_digest_sensitive() {
    let build = |flip: bool| {
        let peers = dids(1);
        let mut store = ExperienceStore::new(16).unwrap();
        for i in 0..6 {
            let outcome = if flip && i == 0 {
                Outcome::Failure
            } else {
                Outcome::Success
            };
            push(&mut store, &format!("t-{i}"), "x", outcome, 10, &peers);
        }
        FeedbackAnalyser::analyse(&store).unwrap()
    };
    let a = build(false);
    let b = build(false);
    assert_eq!(a.digest().unwrap(), b.digest().unwrap());
    assert_ne!(a.digest().unwrap(), build(true).digest().unwrap());
    au4a_core::canonicalize(&a.to_value().unwrap()).unwrap();
}

#[test]
fn per_type_scopes_cover_every_task_type_and_analysis_is_pure() {
    let peers = dids(1);
    let mut store = ExperienceStore::new(16).unwrap();
    for i in 0..6 {
        let kind = if i % 2 == 0 {
            "translate.en-zh"
        } else {
            "classify.zh"
        };
        push(
            &mut store,
            &format!("t-{i}"),
            kind,
            Outcome::Success,
            5,
            &peers,
        );
    }
    let before = store.digest().unwrap();
    let report = FeedbackAnalyser::analyse(&store).unwrap();
    assert_eq!(report.overall.scope, "overall");
    let scopes: Vec<&str> = report.per_type.iter().map(|f| f.scope.as_str()).collect();
    assert_eq!(scopes, vec!["classify.zh", "translate.en-zh"]);
    assert_eq!(report.type_stats("classify.zh").unwrap().sample, 3);
    assert!(report.type_stats("nope").is_none());
    // 分析是只读的：经验库摘要不变
    assert_eq!(store.digest().unwrap(), before);
}

#[test]
fn scenario_settles_successes_through_the_kernel_ledger() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let summary = au4a_learning::scenario(&mut kernel).unwrap();
    let successes = summary["successes"].as_i64().unwrap();
    let settled = summary["settled_total"].as_i64().unwrap();
    assert_eq!(summary["settlement_refused"], 0);
    assert_eq!(settled, successes * 40);
    assert_eq!(summary["agents"], 4);
    assert_eq!(summary["feedback"]["total"], 24);
    assert_eq!(summary["ledger_conserved"], serde_json::json!(true));
    kernel.ledger().check_conservation().unwrap();

    // 结算真的动了账本：学习者可用余额 = 1000 - 10(质押) - settled
    let learner = au4a_learning::scenario::learner_keys().did();
    assert_eq!(
        kernel.ledger().balance(&learner).available,
        Credits(1_000 - 10 - settled)
    );
    assert!(kernel
        .observe()
        .progress
        .iter()
        .any(|e| e.kind == "1.6.settlement" && e.detail.contains("cpu-proto")));
}

#[test]
fn settlement_refusal_is_counted_not_swallowed() {
    // 把 cpu-proto 结算上限压到 0，会让每笔结算被证据闸门拒绝；
    // 场景必须如实记录拒绝次数而不是假装成功。
    let config = KernelConfig {
        cpu_proto_settle_cap = Credits::ZERO,
        ..KernelConfig::default()
    };
    let mut kernel = Kernel::new(config);
    let summary = au4a_learning::scenario(&mut kernel).unwrap();
    assert!(summary["settled_total"].as_i64().unwrap() == 0);
    let refused = summary["settlement_refused"].as_i64().unwrap();
    assert!(refused > 0, "证据闸门拒绝必须被计数");
    assert!(summary["settlement_first_refusal"].is_string());
    kernel.ledger().check_conservation().unwrap();
    assert!(!kernel.refusals().is_empty());
}
