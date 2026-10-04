//! v1.6.8 可解释性测试：解释必须与行为**逐项一致**，且整份解释可以直接发布。

use au4a_core::{AgentKeys, Credits, Did};
use au4a_learning::experience::{Experience, ExperienceStore, Outcome};
use au4a_learning::explain::explain;
use au4a_learning::feedback::FeedbackAnalyser;
use au4a_learning::policy::{adjust, PolicyBounds, PolicyParams, PolicyTargets, Signals};
use au4a_learning::violation::{Violation, ViolationLog};

fn dids(n: u8) -> Vec<Did> {
    (0..n)
        .map(|i| AgentKeys::from_seed(&[0x40 + i; 32]).did())
        .collect()
}

fn fixture(peers: &[Did]) -> (ExperienceStore, au4a_learning::feedback::FeedbackReport) {
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
    let report = FeedbackAnalyser::analyse(&store).unwrap();
    (store, report)
}

fn base_signals() -> Signals {
    Signals {
        sample: 16,
        confidence_bp: 10_000,
        accept_rate_bp: Some(3_000),
        success_bp: 5_000,
        mean_reward: Credits(10),
        prev_mean_reward: Credits::ZERO,
        prev_price_dir: 0,
        violations: 0,
        reputation_delta: 0,
    }
}

#[test]
fn explanation_implies_exactly_what_adjust_does() {
    let peers = dids(2);
    let (_store, report) = fixture(&peers);
    let params = PolicyParams::baseline();
    let bounds = PolicyBounds::default();
    let targets = PolicyTargets::default();
    let violations = ViolationLog::new();

    let cases = [
        ("远低于目标 → 降价", Signals { accept_rate_bp: Some(1_000), ..base_signals() }),
        ("略低于死区外 → 降价", Signals { accept_rate_bp: Some(7_500), ..base_signals() }),
        ("死区内 → 停手", Signals { accept_rate_bp: Some(8_600), ..base_signals() }),
        ("高于目标 → 提价", Signals { accept_rate_bp: Some(9_800), ..base_signals() }),
        (
            "信誉下降 → 冻结涨价",
            Signals { accept_rate_bp: Some(9_800), reputation_delta: -40, ..base_signals() },
        ),
        (
            "收益显著下滑 → 反向",
            Signals {
                accept_rate_bp: Some(1_000),
                prev_mean_reward: Credits(100),
                mean_reward: Credits(10),
                ..base_signals()
            },
        ),
        (
            "收益小幅波动 → 不改方向",
            Signals {
                accept_rate_bp: Some(1_000),
                prev_mean_reward: Credits(100),
                mean_reward: Credits(99),
                ..base_signals()
            },
        ),
    ];
    for (label, signals) in cases {
        let e = explain(&params, &report, &violations, &signals, &bounds, &targets).unwrap();
        let a = adjust(&params, &report, &violations, &signals, &bounds, &targets).unwrap();
        assert_eq!(
            e.price.implied_move_bp, a.price_moved_bp,
            "{label}：解释蕴含 {}bp，实际 {}bp",
            e.price.implied_move_bp, a.price_moved_bp
        );
        assert_eq!(e.price.implied_direction, a.price_moved_bp.signum());
        // 任务偏好：解释蕴含的增量必须逐项等于实际移动
        for b in &e.task_biases {
            let before = params.bias_of_task(&b.target);
            let after =
                (before + b.implied_delta_bp).clamp(bounds.bias_min_bp, bounds.bias_max_bp);
            let moved = a.task_bias_moved_bp.get(&b.target).copied().unwrap_or(0);
            assert_eq!(after - before, moved, "{label}：{b:?}");
        }
        // 协作者偏好：同上
        for b in &e.peer_biases {
            if b.target.starts_with("peer:") {
                // 解释用脱敏标签，实际移动用 DID：按顺序对齐（report.per_peer 是升序）
                let idx = e
                    .peer_biases
                    .iter()
                    .position(|x| x.target == b.target)
                    .unwrap();
                let peer = report.per_peer[idx].peer.clone();
                let before = params.bias_of_peer(&peer);
                let after = (before + b.implied_delta_bp)
                    .clamp(bounds.bias_min_bp, bounds.bias_max_bp);
                let moved = a.peer_bias_moved_bp.get(&peer).copied().unwrap_or(0);
                assert_eq!(after - before, moved, "{label}：协作者 {b:?}");
            }
        }
    }
}

#[test]
fn explanations_are_derived_from_the_data() {
    let peers = dids(2);
    let (_store, report) = fixture(&peers);
    let mut log = ViolationLog::new();
    log.record(Violation::new(&peers[1], "t-1", "peer-withheld-deliverable", 1).unwrap())
        .unwrap();
    let params = PolicyParams::baseline();
    let e = explain(
        &params,
        &report,
        &log,
        &base_signals(),
        &PolicyBounds::default(),
        &PolicyTargets::default(),
    )
    .unwrap();

    // 定价解释引用接受率与目标/死区
    assert_eq!(e.price.accept_rate_bp, Some(3_000));
    assert_eq!(e.price.target_bp, 8_500);
    assert_eq!(e.price.deadband_bp, 500);
    assert!(e.price.reason.contains("3000bp") && e.price.reason.contains("8500bp"));
    assert_eq!(e.price.implied_move_bp, -500);

    // 任务解释：好类型评分更高 → 正增量；坏类型 → 负增量；且引用样本数
    let good = e.task_biases.iter().find(|b| b.target == "good.type").unwrap();
    let bad = e.task_biases.iter().find(|b| b.target == "bad.type").unwrap();
    assert!(good.score_bp > bad.score_bp);
    assert!(good.implied_delta_bp > 0 && bad.implied_delta_bp < 0);
    assert!(good.reason.contains("样本 8 条"));
    assert_eq!(good.score_bp, 9_333);
    // 坏类型：8 条全失败（质量 0）且收益 0 → 评分 (2×0 + 0)/3 = 0
    assert_eq!(bad.score_bp, 0);
    assert_eq!(e.task_biases.len(), 2);

    // 协作者解释：有违规的一方出现惩罚说明
    assert_eq!(e.peer_biases.len(), 2);
    let punished = e.peer_biases.iter().find(|b| b.violations > 0).unwrap();
    assert!(punished.reason.contains("违规"));
    assert!(punished.implied_delta_bp < 0);
    assert!(e.evidence.contains("门槛"));

    // 样本不足的类型不给增量
    let mut thin = ExperienceStore::new(8).unwrap();
    let context = "ctx-thin".to_string();
    thin.record(
        Experience::new("t-1", "thin.type", &context, "d", Outcome::Success, Credits(1), 1, &peers[..1])
            .unwrap(),
    )
    .unwrap();
    let thin_report = FeedbackAnalyser::analyse(&thin).unwrap();
    let thin_explanation = explain(
        &params,
        &thin_report,
        &ViolationLog::new(),
        &Signals { sample: 1, confidence_bp: 2_500, ..base_signals() },
        &PolicyBounds::default(),
        &PolicyTargets::default(),
    )
    .unwrap();
    let thin_bias = thin_explanation
        .task_biases
        .iter()
        .find(|b| b.target == "thin.type")
        .unwrap();
    assert_eq!(thin_bias.implied_delta_bp, 0);
    assert!(thin_bias.reason.contains("不足"));
}

#[test]
fn explanations_are_publishable_and_deterministic() {
    let peers = dids(2);
    let (_store, report) = fixture(&peers);
    let mut log = ViolationLog::new();
    log.record(Violation::new(&peers[1], "t-1", "peer-withheld-deliverable", 1).unwrap())
        .unwrap();
    let args = (
        PolicyParams::baseline(),
        report.clone(),
        log.clone(),
        base_signals(),
        PolicyBounds::default(),
        PolicyTargets::default(),
    );
    let e1 = explain(&args.0, &args.1, &args.2, &args.3, &args.4, &args.5).unwrap();
    let e2 = explain(&args.0, &args.1, &args.2, &args.3, &args.4, &args.5).unwrap();
    assert_eq!(e1, e2);
    assert_eq!(e1.digest().unwrap(), e2.digest().unwrap());

    let text = e1.summary();
    let json = e1.to_value().unwrap().to_string();
    assert!(!text.contains("did:au4a:") && !json.contains("did:au4a:"));
    assert!(json.contains("peer:"), "协作者应以脱敏标签出现");
    au4a_core::canonicalize(&e1.to_value().unwrap()).unwrap();
    // 人类可读摘要覆盖三类内容
    assert!(text.contains("定价"));
    assert!(text.contains("任务偏好"));
    assert!(text.contains("协作偏好"));
}

#[test]
fn explanation_respects_bounds_and_reputation_guard() {
    let peers = dids(2);
    let (_store, report) = fixture(&peers);
    // 价格已在上界：解释蕴含的上移被钳制成 0
    let params = PolicyParams {
        price_bp: 20_000,
        task_bias_bp: std::collections::BTreeMap::new(),
        peer_bias_bp: std::collections::BTreeMap::new(),
    };
    let signals = Signals { accept_rate_bp: Some(9_900), ..base_signals() };
    let e = explain(
        &params,
        &report,
        &ViolationLog::new(),
        &signals,
        &PolicyBounds::default(),
        &PolicyTargets::default(),
    )
    .unwrap();
    assert_eq!(e.price.implied_direction, 1, "方向仍是上移");
    assert_eq!(e.price.implied_move_bp, 0, "但位移被边界钳制成 0");
    let a = adjust(
        &params,
        &report,
        &ViolationLog::new(),
        &signals,
        &PolicyBounds::default(),
        &PolicyTargets::default(),
    )
    .unwrap();
    assert_eq!(a.price_moved_bp, 0);
}

#[test]
fn self_check_reports_the_explainability_evidence() {
    let checks = au4a_learning::self_check();
    assert!(au4a_core::all_passed(&checks), "自检未全绿");
    let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"explain.matches_behaviour"));
    assert!(names.contains(&"explain.publishable_and_derived"));
    // 每一项都必须有非空证据说明
    assert!(checks.iter().all(|c| !c.detail.trim().is_empty()));
}
