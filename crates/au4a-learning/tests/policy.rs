//! v1.6.3 行为调整集成测试：三个策略杠杆真的会动、没有证据时不动、有界且可解释。

use au4a_core::{AgentKeys, CoreError, Credits, Did};
use au4a_learning::experience::{Experience, ExperienceStore, Outcome};
use au4a_learning::feedback::FeedbackAnalyser;
use au4a_learning::policy::{
    adjust, task_score_bp, PolicyBounds, PolicyParams, PolicyTargets, Signals,
};
use au4a_learning::violation::{Violation, ViolationLog};

fn dids(n: u8) -> Vec<Did> {
    (0..n)
        .map(|i| AgentKeys::from_seed(&[0x40 + i; 32]).did())
        .collect()
}

/// 构造一个「类型 good 好、类型 bad 差；peer0 好、peer1 差」的经验库。
fn store_with_two_quality_levels(peers: &[Did]) -> ExperienceStore {
    let mut store = ExperienceStore::new(64).unwrap();
    for i in 0..16u64 {
        let (task_type, outcome, reward, peer) = if i % 2 == 0 {
            ("good.type", Outcome::Success, 80i64, peers[0].clone())
        } else {
            ("bad.type", Outcome::Failure, 0i64, peers[1].clone())
        };
        let context = format!("ctx-{i}");
        store
            .record(
                Experience::new(
                    &format!("t-{i}"),
                    task_type,
                    &context,
                    "deliver",
                    outcome,
                    Credits(reward),
                    i,
                    &[peer],
                )
                .unwrap(),
            )
            .unwrap();
    }
    store
}

fn signals(accept_bp: Option<i64>) -> Signals {
    Signals {
        sample: 16,
        confidence_bp: 10_000,
        accept_rate_bp: accept_bp,
        success_bp: 5_000,
        mean_reward: Credits(10),
        prev_mean_reward: Credits::ZERO,
        prev_price_dir: 0,
        violations: 0,
        reputation_delta: 0,
    }
}

#[test]
fn no_evidence_means_no_change() {
    let peers = dids(2);
    let store = store_with_two_quality_levels(&peers);
    let report = FeedbackAnalyser::analyse(&store).unwrap();
    let baseline = PolicyParams::baseline();
    let weak = Signals {
        confidence_bp: 2_500,
        ..signals(Some(3_000))
    };
    let out = adjust(
        &baseline,
        &report,
        &ViolationLog::new(),
        &weak,
        &PolicyBounds::default(),
        &PolicyTargets::default(),
    )
    .unwrap();
    assert!(!out.changed);
    assert_eq!(out.next, baseline);
    assert!(out.reasons[0].contains("evidence-below-threshold"));
    assert_eq!(out.price_moved_bp, 0);
    assert!(out.task_bias_moved_bp.is_empty() && out.peer_bias_moved_bp.is_empty());
}

#[test]
fn all_three_levers_move_in_the_right_direction() {
    let peers = dids(2);
    let store = store_with_two_quality_levels(&peers);
    let report = FeedbackAnalyser::analyse(&store).unwrap();
    let baseline = PolicyParams::baseline();
    let bounds = PolicyBounds::default();

    // 接受率 3000bp 远低于目标 8500bp → 降价一步
    let out = adjust(
        &baseline,
        &report,
        &ViolationLog::new(),
        &signals(Some(3_000)),
        &bounds,
        &PolicyTargets::default(),
    )
    .unwrap();
    assert!(out.changed);
    assert_eq!(out.price_moved_bp, -bounds.price_step_bp);
    assert_eq!(out.next.price_bp, baseline.price_bp - bounds.price_step_bp);
    assert!(out.task_bias_moved_bp["good.type"] > 0);
    assert!(out.task_bias_moved_bp["bad.type"] < 0);
    assert!(out.peer_bias_moved_bp[&peers[0]] > 0);
    assert!(out.peer_bias_moved_bp[&peers[1]] < 0);
    // 每一步都有理由，且能追溯到数据
    assert!(out.reasons.iter().any(|r| r.starts_with("pricing:")));
    assert!(out.reasons.iter().any(|r| r.starts_with("task-select: good.type")));
    assert!(out.reasons.iter().any(|r| r.starts_with("peer-select:")));
    assert!(out.reasons.len() >= 4, "每一步都必须给出理由");

    // 接受率远高于目标 → 提价一步
    let up = adjust(
        &baseline,
        &report,
        &ViolationLog::new(),
        &signals(Some(9_800)),
        &bounds,
        &PolicyTargets::default(),
    )
    .unwrap();
    assert_eq!(up.price_moved_bp, bounds.price_step_bp);

    // 死区内 → 价格不动（但任务/协作者偏好仍然可以动）
    let dead = adjust(
        &baseline,
        &report,
        &ViolationLog::new(),
        &signals(Some(8_600)),
        &bounds,
        &PolicyTargets::default(),
    )
    .unwrap();
    assert_eq!(dead.price_moved_bp, 0);
    assert_eq!(dead.next.price_bp, baseline.price_bp);
}

#[test]
fn revenue_trend_overrides_the_accept_rate_direction() {
    let peers = dids(2);
    let store = store_with_two_quality_levels(&peers);
    let report = FeedbackAnalyser::analyse(&store).unwrap();
    let baseline = PolicyParams::baseline();
    let bounds = PolicyBounds::default();

    // 上一轮方向 -1；本轮收益比上一轮**变差** → 反向（+1）
    let worse = Signals {
        prev_mean_reward: Credits(50),
        mean_reward: Credits(30),
        prev_price_dir: -1,
        ..signals(Some(3_000))
    };
    let out = adjust(
        &baseline,
        &report,
        &ViolationLog::new(),
        &worse,
        &bounds,
        &PolicyTargets::default(),
    )
    .unwrap();
    assert_eq!(out.price_moved_bp, bounds.price_step_bp, "收益变差必须反向");

    // 收益变好 → 沿原方向继续
    let better = Signals {
        prev_mean_reward: Credits(30),
        mean_reward: Credits(50),
        prev_price_dir: -1,
        ..signals(Some(3_000))
    };
    let out2 = adjust(
        &baseline,
        &report,
        &ViolationLog::new(),
        &better,
        &bounds,
        &PolicyTargets::default(),
    )
    .unwrap();
    assert_eq!(out2.price_moved_bp, -bounds.price_step_bp, "收益变好必须继续");
}

#[test]
fn violations_penalise_the_selected_peer_immediately() {
    let peers = dids(2);
    let store = store_with_two_quality_levels(&peers);
    let report = FeedbackAnalyser::analyse(&store).unwrap();
    let baseline = PolicyParams::baseline();
    let bounds = PolicyBounds::default();
    let targets = PolicyTargets::default();

    let clean = adjust(
        &baseline,
        &report,
        &ViolationLog::new(),
        &signals(Some(3_000)),
        &bounds,
        &targets,
    )
    .unwrap();
    let mut log = ViolationLog::new();
    log.record(Violation::new(&peers[0], "t-0", "peer-withheld-deliverable", 1).unwrap())
        .unwrap();
    let dirty = adjust(
        &baseline,
        &report,
        &log,
        &signals(Some(3_000)),
        &bounds,
        &targets,
    )
    .unwrap();
    assert!(
        dirty.next.bias_of_peer(&peers[0]) < clean.next.bias_of_peer(&peers[0]),
        "违规协作者偏好必须更低"
    );
    assert!(dirty
        .reasons
        .iter()
        .any(|r| r.contains("violations=1") && r.contains("penalty=")));
}

#[test]
fn parameters_are_always_clamped_inside_bounds() {
    let peers = dids(2);
    let store = store_with_two_quality_levels(&peers);
    let report = FeedbackAnalyser::analyse(&store).unwrap();
    let bounds = PolicyBounds {
        price_min_bp: 9_000,
        price_max_bp: 11_000,
        bias_min_bp: -100,
        bias_max_bp: 100,
        price_step_bp: 10_000,
        bias_step_bp: 10_000,
    };
    let mut params = PolicyParams::baseline();
    params.price_bp = 11_000;
    let out = adjust(
        &params,
        &report,
        &ViolationLog::new(),
        &signals(Some(9_900)),
        &bounds,
        &PolicyTargets::default(),
    )
    .unwrap();
    assert_eq!(out.price_moved_bp, 0, "已在上界时不得越界");
    assert_eq!(out.next.price_bp, 11_000);
    for v in out.next.task_bias_bp.values() {
        assert!((-100..=100).contains(v), "偏好越界: {v}");
    }
}

#[test]
fn invalid_bounds_are_refused() {
    let peers = dids(2);
    let store = store_with_two_quality_levels(&peers);
    let report = FeedbackAnalyser::analyse(&store).unwrap();
    let bad = PolicyBounds {
        price_min_bp: 0,
        ..PolicyBounds::default()
    };
    assert_eq!(
        adjust(
            &PolicyParams::baseline(),
            &report,
            &ViolationLog::new(),
            &signals(Some(5_000)),
            &bad,
            &PolicyTargets::default()
        ),
        Err(CoreError::NegativeAmount)
    );
    let bad_targets = PolicyTargets {
        min_confidence_bp: 20_000,
        ..PolicyTargets::default()
    };
    assert_eq!(
        adjust(
            &PolicyParams::baseline(),
            &report,
            &ViolationLog::new(),
            &signals(Some(5_000)),
            &PolicyBounds::default(),
            &bad_targets
        ),
        Err(CoreError::InvalidKind)
    );
}

#[test]
fn task_scoring_is_a_pure_integer_combination() {
    let peers = dids(2);
    let store = store_with_two_quality_levels(&peers);
    let report = FeedbackAnalyser::analyse(&store).unwrap();
    let overall = task_score_bp(&report.overall, Credits(100));
    // overall: quality 5000bp，mean_reward = (16×40)/16 = 40 → reward_bp = 4000 → (2×5000+4000)/3 = 4666
    assert_eq!(overall, 4_666);
    let good = report.type_stats("good.type").unwrap();
    // good: quality 10000，mean_reward 80 → reward_bp 8000 → (20000+8000)/3 = 9333
    assert_eq!(task_score_bp(good, Credits(100)), 9_333);
    // 参考收益为 0 时收益项按 0 计（不做除零）
    assert_eq!(task_score_bp(good, Credits::ZERO), (2 * good.quality_bp) / 3);
}

#[test]
fn policy_public_projection_hides_peer_identities() {
    let peers = dids(2);
    let store = store_with_two_quality_levels(&peers);
    let report = FeedbackAnalyser::analyse(&store).unwrap();
    let out = adjust(
        &PolicyParams::baseline(),
        &report,
        &ViolationLog::new(),
        &signals(Some(3_000)),
        &PolicyBounds::default(),
        &PolicyTargets::default(),
    )
    .unwrap();
    let public = out.next.public_json().unwrap();
    let text = public.to_string();
    assert!(!text.contains("did:au4a:"));
    assert_eq!(public["peers_tracked"], 2);
    assert!(public["task_bias_bp"]["good.type"].as_i64().unwrap() > 0);
    // 调整理由也可以发布（不含 DID）
    let reasons = out.public_json().unwrap();
    assert!(!reasons["peers_moved"].is_null());
    assert!(reasons["reasons"].as_array().unwrap().len() >= 4);
    assert!(!reasons.to_string().contains("did:au4a:"));
}

#[test]
fn adjust_is_deterministic_for_the_same_inputs() {
    let peers = dids(2);
    let store = store_with_two_quality_levels(&peers);
    let report = FeedbackAnalyser::analyse(&store).unwrap();
    let run = || {
        adjust(
            &PolicyParams::baseline(),
            &report,
            &ViolationLog::new(),
            &signals(Some(3_000)),
            &PolicyBounds::default(),
            &PolicyTargets::default(),
        )
        .unwrap()
    };
    assert_eq!(run(), run());
    // 空任务偏好图（没有反馈）时不会凭空创造条目
    let empty = ExperienceStore::new(4).unwrap();
    let empty_report = FeedbackAnalyser::analyse(&empty).unwrap();
    let out = adjust(
        &PolicyParams::baseline(),
        &empty_report,
        &ViolationLog::new(),
        &signals(Some(3_000)),
        &PolicyBounds::default(),
        &PolicyTargets::default(),
    )
    .unwrap();
    assert!(out.next.task_bias_bp.is_empty() && out.next.peer_bias_bp.is_empty());
    assert_eq!(out.price_moved_bp, -500, "没有任务数据时仍可按接受率动价");
}
