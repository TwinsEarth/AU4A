//! v1.6.4 学习信号集成测试：四类信号精确折算、真的参与决策、权重非法被拒。

use au4a_core::{AgentKeys, CoreError, Credits, Did};
use au4a_learning::experience::{Experience, ExperienceStore, Outcome};
use au4a_learning::feedback::FeedbackAnalyser;
use au4a_learning::policy::{adjust, PolicyBounds, PolicyParams, PolicyTargets};
use au4a_learning::signal::{LearningSignal, SignalWeights, VIOLATION_UNIT_BP};
use au4a_learning::violation::{Violation, ViolationLog};

fn dids(n: u8) -> Vec<Did> {
    (0..n)
        .map(|i| AgentKeys::from_seed(&[0x40 + i; 32]).did())
        .collect()
}

fn exp(
    id: &str,
    task_type: &str,
    outcome: Outcome,
    reward: i64,
    peers: &[Did],
) -> Experience {
    let context = format!("ctx-{id}");
    Experience::new(
        id,
        task_type,
        &context,
        "deliver",
        outcome,
        Credits(reward),
        1,
        peers,
    )
    .unwrap()
}

#[test]
fn four_signals_fold_into_one_composite_exactly() {
    let w = SignalWeights::default();
    let s = LearningSignal::compute(5_000, Credits(50), 50, 0, &w).unwrap();
    assert_eq!(s.quality_norm_bp, 5_000);
    assert_eq!(s.settled_norm_bp, 5_000);
    assert_eq!(s.reputation_norm_bp, 5_000);
    assert_eq!(s.violation_norm_bp, 0);
    // 2500 + 1250 + 750 = 4500
    assert_eq!(s.composite_bp, 4_500);

    // 边界：满格质量 + 满格结算 + 满格信誉 = 9000（余下 1000 是违规权重，只在扣分时出现）
    let full = LearningSignal::compute(10_000, Credits(100), 100, 0, &w).unwrap();
    assert_eq!(full.composite_bp, 9_000);
    // 四类全负：质量 0、无结算、信誉 -100、违规 4 → -1500 - 1000 = -2500
    let bad = LearningSignal::compute(0, Credits::ZERO, -100, 4, &w).unwrap();
    assert_eq!(bad.violation_norm_bp, 4 * VIOLATION_UNIT_BP);
    assert_eq!(bad.composite_bp, -2_500);
}

#[test]
fn each_signal_has_the_declared_effect() {
    let w = SignalWeights::default();
    let base = LearningSignal::compute(5_000, Credits(50), 0, 0, &w).unwrap();

    // 质量上升 → 综合上升
    let better_quality = LearningSignal::compute(10_000, Credits(50), 0, 0, &w).unwrap();
    assert!(better_quality.composite_bp > base.composite_bp);
    // 结算金额上升 → 综合上升（且封顶）
    let more_money = LearningSignal::compute(5_000, Credits(500), 0, 0, &w).unwrap();
    assert!(more_money.composite_bp > base.composite_bp);
    assert_eq!(more_money.settled_norm_bp, 10_000);
    // 信誉上升 → 综合上升；信誉下降 → 综合下降
    let good_rep = LearningSignal::compute(5_000, Credits(50), 100, 0, &w).unwrap();
    let bad_rep = LearningSignal::compute(5_000, Credits(50), -100, 0, &w).unwrap();
    assert!(good_rep.composite_bp > base.composite_bp);
    assert!(bad_rep.composite_bp < base.composite_bp);
    // 违规 → 严格下降，且每条递减直到第 4 条吃满
    let v1 = LearningSignal::compute(5_000, Credits(50), 0, 1, &w).unwrap();
    let v2 = LearningSignal::compute(5_000, Credits(50), 0, 2, &w).unwrap();
    let v4 = LearningSignal::compute(5_000, Credits(50), 0, 4, &w).unwrap();
    let v9 = LearningSignal::compute(5_000, Credits(50), 0, 9, &w).unwrap();
    assert!(base.composite_bp > v1.composite_bp);
    assert!(v1.composite_bp > v2.composite_bp);
    assert!(v2.composite_bp > v4.composite_bp);
    assert_eq!(v4.composite_bp, v9.composite_bp);
}

#[test]
fn per_experience_signal_counts_only_that_experiences_peers() {
    let peers = dids(3);
    let w = SignalWeights::default();
    let mut log = ViolationLog::new();
    log.record(Violation::new(&peers[0], "t-0", "peer-withheld-deliverable", 1).unwrap())
        .unwrap();

    let with_bad = exp("a", "x", Outcome::Success, 40, &peers[..1]);
    let with_clean = exp("b", "x", Outcome::Success, 40, &peers[1..2]);
    let s_bad = LearningSignal::from_experience(&with_bad, &log, &w).unwrap();
    let s_clean = LearningSignal::from_experience(&with_clean, &log, &w).unwrap();
    assert_eq!(s_bad.violations, 1);
    assert_eq!(s_clean.violations, 0);
    assert!(s_bad.composite_bp < s_clean.composite_bp);
    assert_eq!(s_bad.quality_bp, 10_000);
    assert_eq!(s_bad.settled, Credits(40));
}

#[test]
fn store_aggregate_averages_quality_and_sums_settlements() {
    let peers = dids(1);
    let mut store = ExperienceStore::new(16).unwrap();
    for (i, (outcome, reward)) in [
        (Outcome::Success, 40i64),
        (Outcome::Success, 40),
        (Outcome::Partial, 20),
        (Outcome::Failure, 0),
    ]
    .into_iter()
    .enumerate()
    {
        store
            .record(exp(&format!("t-{i}"), "x", outcome, reward, &peers))
            .unwrap();
    }
    let s = LearningSignal::from_store(&store, &ViolationLog::new(), 0, &SignalWeights::default())
        .unwrap();
    assert_eq!(s.quality_bp, 6_250); // (10000+10000+5000+0)/4
    assert_eq!(s.settled, Credits(100));
    assert_eq!(s.settled_norm_bp, 10_000); // 100 / reward_ref 100 → 满格
    assert_eq!(s.violations, 0);

    // 空库 → 零信号而不是错误
    let empty = ExperienceStore::new(4).unwrap();
    let e = LearningSignal::from_store(&empty, &ViolationLog::new(), 0, &SignalWeights::default())
        .unwrap();
    assert_eq!(e.quality_bp, 0);
    assert_eq!(e.composite_bp, 0);
}

#[test]
fn weights_are_validated() {
    let w = SignalWeights::default();
    assert_eq!(w.sum_bp(), 10_000);
    assert!(w.validate().is_ok());
    assert_eq!(
        SignalWeights {
            quality_bp: -1,
            ..w.clone()
        }
        .validate(),
        Err(CoreError::NegativeAmount)
    );
    assert_eq!(
        SignalWeights {
            quality_bp: 6_000,
            ..w.clone()
        }
        .validate(),
        Err(CoreError::InvalidKind)
    );
    assert_eq!(
        SignalWeights {
            reward_ref: Credits::ZERO,
            ..w.clone()
        }
        .validate(),
        Err(CoreError::InvalidKind)
    );
    assert_eq!(
        SignalWeights {
            reputation_ref: 0,
            ..w.clone()
        }
        .validate(),
        Err(CoreError::InvalidKind)
    );
    // 自定义权重会真的改变综合值：把质量权重降到 0，质量差异不再影响结论
    let quality_only_off = SignalWeights {
        quality_bp: 0,
        settled_bp: 5_000,
        reputation_bp: 4_000,
        violation_bp: 1_000,
        ..w.clone()
    };
    let a = LearningSignal::compute(0, Credits(50), 0, 0, &quality_only_off).unwrap();
    let b = LearningSignal::compute(10_000, Credits(50), 0, 0, &quality_only_off).unwrap();
    assert_eq!(a.composite_bp, b.composite_bp);
}

#[test]
fn signal_maps_into_policy_inputs() {
    let w = SignalWeights::default();
    let s = LearningSignal::compute(7_500, Credits(80), -40, 2, &w).unwrap();
    let signals = s.to_signals(12, Some(4_000), Credits(30), Credits::ZERO, 0);
    assert_eq!(signals.sample, 12);
    assert_eq!(signals.success_bp, 7_500, "质量信号进入 success_bp");
    assert_eq!(signals.violations, 2);
    assert_eq!(signals.reputation_delta, -40);
    assert_eq!(signals.confidence_bp, 10_000);
    assert_eq!(signals.mean_reward, Credits(30));
}

#[test]
fn a_falling_reputation_freezes_price_increases() {
    let peers = dids(2);
    let mut store = ExperienceStore::new(16).unwrap();
    for i in 0..8 {
        let (task_type, outcome, reward, peer) = if i % 2 == 0 {
            ("good.type", Outcome::Success, 80i64, peers[0].clone())
        } else {
            ("bad.type", Outcome::Failure, 0i64, peers[1].clone())
        };
        store
            .record(exp(&format!("t-{i}"), task_type, outcome, reward, &[peer]))
            .unwrap();
    }
    let report = FeedbackAnalyser::analyse(&store).unwrap();
    let w = SignalWeights::default();
    let base_signals = |reputation_delta: i64| {
        LearningSignal::compute(5_000, Credits(50), reputation_delta, 0, &w)
            .unwrap()
            .to_signals(8, Some(9_800), Credits(40), Credits::ZERO, 0) // 接受率远高于目标 → 想涨价
    };
    let bounds = PolicyBounds::default();
    let targets = PolicyTargets::default();

    let normal = adjust(
        &PolicyParams::baseline(),
        &report,
        &ViolationLog::new(),
        &base_signals(0),
        &bounds,
        &targets,
    )
    .unwrap();
    assert_eq!(normal.price_moved_bp, 500, "信誉正常时应涨价");

    let falling = adjust(
        &PolicyParams::baseline(),
        &report,
        &ViolationLog::new(),
        &base_signals(-40),
        &bounds,
        &targets,
    )
    .unwrap();
    assert_eq!(falling.price_moved_bp, 0, "信誉下降时必须冻结涨价");
    assert!(falling
        .reasons
        .iter()
        .any(|r| r.contains("冻结涨价") && r.contains("reputation_delta=-40")));
    // 任务/协作者偏好不受信誉门影响（信誉只作为定价的风险闸）
    assert!(falling.task_bias_moved_bp["good.type"] > 0);
}

#[test]
fn market_rounds_carry_the_signal_composite() {
    let config = au4a_learning::sim::MarketConfig::default();
    let (control, learning, _) = au4a_learning::sim::ab_test(&config).unwrap();
    // 每轮都有综合信号，且综合信号通常在 [0, 10000]（早期轮次可能有违规扣分，故不假设下界）
    for r in &learning.rounds {
        assert!(r.signal_composite_bp <= 10_000);
        assert!(r.signal_composite_bp >= -10_000);
    }
    assert!(learning
        .rounds
        .iter()
        .any(|r| r.signal_composite_bp != control.rounds[r.round as usize].signal_composite_bp),
        "学习组与对照组的信号不必相同（选择不同 → 结果不同）");
    assert!(au4a_learning::self_check().iter().any(|c| c.name == "signal.exact_weighting"));
}
