//! v1.6.5 模型更新集成测试：动量、阻尼、遗忘、漂移钳制、回滚、信誉不可转让。

use std::collections::BTreeMap;

use au4a_core::{AgentKeys, CoreError, Credits, Did};
use au4a_learning::experience::{Experience, ExperienceStore, Outcome};
use au4a_learning::feedback::FeedbackAnalyser;
use au4a_learning::model::{
    LearningModel, ModelConfig, ReputationLedger, REPUTATION_FAILURE, REPUTATION_MAX,
    REPUTATION_MIN, REPUTATION_PARTIAL, REPUTATION_SUCCESS, REPUTATION_VIOLATION,
};
use au4a_learning::policy::{
    adjust, PolicyAdjustment, PolicyBounds, PolicyParams, PolicyTargets, Signals,
};

fn dids(n: u8) -> Vec<Did> {
    (0..n)
        .map(|i| AgentKeys::from_seed(&[0x40 + i; 32]).did())
        .collect()
}

fn adjustment(price_moved: i64, task: &[(&str, i64)], peers: &[(&Did, i64)]) -> PolicyAdjustment {
    PolicyAdjustment {
        next: PolicyParams::baseline(),
        changed: true,
        price_moved_bp: price_moved,
        task_bias_moved_bp: task.iter().map(|(k, v)| ((*k).to_string(), *v)).collect(),
        peer_bias_moved_bp: peers.iter().map(|(k, v)| ((*k).clone(), *v)).collect(),
        reasons: vec!["test-adjustment".to_string()],
    }
}

/// 一个「好类型 / 坏类型 + 好协作者 / 坏协作者」的经验库，用来产生真实的调整意图。
fn report_fixture(peers: &[Did]) -> au4a_learning::feedback::FeedbackReport {
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
    FeedbackAnalyser::analyse(&store).unwrap()
}

#[test]
fn momentum_smooths_the_trajectory() {
    let bounds = PolicyBounds::default();
    let mut model = LearningModel::new(bounds.clone());
    let mut drifts = Vec::new();
    for _ in 0..8 {
        let rec = model.apply(&adjustment(-500, &[], &[])).unwrap();
        drifts.push(rec.price_drift_bp);
    }
    // 第一步被动量压小（350 而不是 500），随后逐步逼近满步长
    assert_eq!(drifts[0], -350);
    assert_eq!(drifts[1], -455);
    assert!(drifts[0].abs() < drifts[1].abs() && drifts[1].abs() < drifts[2].abs());
    assert!(drifts
        .iter()
        .all(|d| d.abs() <= ModelConfig::default().max_drift_bp));
    assert_eq!(model.generation(), 8);
    // 无动量时就是原样一步
    let mut raw = LearningModel::with_config(
        bounds,
        ModelConfig {
            momentum_bp: 0,
            ..ModelConfig::default()
        },
    );
    assert_eq!(
        raw.apply(&adjustment(-500, &[], &[]))
            .unwrap()
            .price_drift_bp,
        -500
    );
}

#[test]
fn reversed_direction_is_damped() {
    let mut model = LearningModel::with_config(
        PolicyBounds::default(),
        ModelConfig {
            momentum_bp: 0,
            ..ModelConfig::default()
        },
    );
    let down = model.apply(&adjustment(-500, &[], &[])).unwrap();
    assert!(!down.damped);
    let up = model.apply(&adjustment(500, &[], &[])).unwrap();
    assert!(up.damped, "方向反转必须被阻尼");
    assert_eq!(up.price_drift_bp, 250);
}

#[test]
fn forgetting_decays_stale_preferences_to_zero_and_drops_them() {
    let mut model = LearningModel::new(PolicyBounds::default());
    let rec = model
        .apply(&adjustment(0, &[("translate.en-zh", 400)], &[]))
        .unwrap();
    assert!(rec.forgotten_entries == 0);
    let peak = model.params().bias_of_task("translate.en-zh");
    assert!(peak > 0 && peak <= 400);
    let mut total_forgotten = 0usize;
    for _ in 0..80 {
        let noop = PolicyAdjustment {
            next: model.params().clone(),
            changed: false,
            price_moved_bp: 0,
            task_bias_moved_bp: BTreeMap::new(),
            peer_bias_moved_bp: BTreeMap::new(),
            reasons: vec!["noop".to_string()],
        };
        total_forgotten += model.apply(&noop).unwrap().forgotten_entries;
    }
    assert_eq!(model.params().bias_of_task("translate.en-zh"), 0);
    assert!(!model.params().task_bias_bp.contains_key("translate.en-zh"));
    assert!(total_forgotten >= 1, "至少有一次遗忘把条目清零");
    // 遗忘率可配置：关掉遗忘后偏好不衰减
    let mut sticky = LearningModel::with_config(
        PolicyBounds::default(),
        ModelConfig {
            forgetting_bp: 0,
            ..ModelConfig::default()
        },
    );
    sticky
        .apply(&adjustment(0, &[("translate.en-zh", 400)], &[]))
        .unwrap();
    for _ in 0..20 {
        let noop = PolicyAdjustment {
            next: sticky.params().clone(),
            changed: false,
            price_moved_bp: 0,
            task_bias_moved_bp: BTreeMap::new(),
            peer_bias_moved_bp: BTreeMap::new(),
            reasons: vec!["noop".to_string()],
        };
        sticky.apply(&noop).unwrap();
    }
    assert!(sticky.params().bias_of_task("translate.en-zh") > 0);
}

#[test]
fn drift_is_clamped_every_generation() {
    let config = ModelConfig::default();
    let mut model = LearningModel::new(PolicyBounds::default());
    let rec = model
        .apply(&adjustment(-10_000, &[("translate.en-zh", 10_000)], &[]))
        .unwrap();
    assert!(rec.clamped);
    assert!(rec.price_drift_bp.abs() <= config.max_drift_bp);
    assert!(rec.price_after_bp >= PolicyBounds::default().price_min_bp);
    let bias = model.params().bias_of_task("translate.en-zh");
    assert!(
        bias.abs() <= config.max_drift_bp,
        "偏好单代位移也被钳制：{bias}"
    );
}

#[test]
fn rollback_restores_the_previous_generation_exactly() {
    let mut model = LearningModel::new(PolicyBounds::default());
    let baseline = model.params().clone();
    assert_eq!(
        model.rollback(),
        Err(CoreError::InvalidVersion),
        "第 0 代无可回滚"
    );

    model.apply(&adjustment(-500, &[], &[])).unwrap();
    let after_one = model.params().clone();
    model.apply(&adjustment(-500, &[], &[])).unwrap();
    assert_ne!(model.params(), &after_one);

    let restored = model.rollback().unwrap();
    assert_eq!(restored, after_one);
    assert_eq!(model.generation(), 1);
    let back_to_start = model.rollback().unwrap();
    assert_eq!(back_to_start, baseline);
    assert_eq!(model.generation(), 0);
    assert_eq!(model.rollback(), Err(CoreError::InvalidVersion));
}

#[test]
fn rollback_history_is_bounded() {
    let mut model = LearningModel::with_config(
        PolicyBounds::default(),
        ModelConfig {
            max_generations: 4,
            ..ModelConfig::default()
        },
    );
    for _ in 0..10 {
        model.apply(&adjustment(-100, &[], &[])).unwrap();
    }
    assert_eq!(model.history_len(), 4, "历史深度必须被封顶");
    let mut count = 0;
    while model.rollback().is_ok() {
        count += 1;
    }
    assert_eq!(count, 4);
    assert_eq!(model.generation(), 6, "只能回滚到最近 4 代的起点");
}

#[test]
fn reputation_is_bounded_clamped_and_non_transferable() {
    let peers = dids(3);
    let mut ledger = ReputationLedger::new();
    assert_eq!(ledger.score_of(&peers[0]), 0, "未知协作者的信誉是 0");

    // 精确增量
    assert_eq!(
        ledger.apply_outcome(&peers[0], Outcome::Success),
        REPUTATION_SUCCESS
    );
    assert_eq!(
        ledger.apply_outcome(&peers[0], Outcome::Partial),
        REPUTATION_PARTIAL
    );
    assert_eq!(
        ledger.apply_outcome(&peers[0], Outcome::Failure),
        REPUTATION_FAILURE
    );
    assert_eq!(
        ledger.score_of(&peers[0]),
        REPUTATION_SUCCESS + REPUTATION_PARTIAL + REPUTATION_FAILURE
    );
    assert_eq!(ledger.apply_violation(&peers[0]), REPUTATION_VIOLATION);

    // 上界钳制
    for _ in 0..200 {
        ledger.apply_outcome(&peers[0], Outcome::Success);
    }
    assert_eq!(ledger.score_of(&peers[0]), REPUTATION_MAX);
    assert!(ledger.clamped() > 0);
    // 下界钳制
    for _ in 0..200 {
        ledger.apply_violation(&peers[1]);
    }
    assert_eq!(ledger.score_of(&peers[1]), REPUTATION_MIN);

    // 不可转让：peers[2] 从未被观察 → 始终为 0；peers[0]/[1] 的分数不因第三方的经历变化
    let before = (ledger.score_of(&peers[0]), ledger.score_of(&peers[1]));
    for _ in 0..50 {
        ledger.apply_outcome(&peers[2], Outcome::Success);
    }
    assert_eq!(ledger.score_of(&peers[2]), REPUTATION_MAX);
    assert_eq!(
        (ledger.score_of(&peers[0]), ledger.score_of(&peers[1])),
        before,
        "一个协作者的更新绝不影响另一个协作者（没有转让路径）"
    );
    assert_eq!(
        ledger.public_json().unwrap()["transferable"],
        serde_json::json!(false)
    );
    assert!(!ledger
        .public_json()
        .unwrap()
        .to_string()
        .contains("did:au4a:"));
}

#[test]
fn reputation_updates_come_only_from_observed_experiences() {
    let peers = dids(2);
    let mut ledger = ReputationLedger::new();
    let context = "ctx".to_string();
    let exp = Experience::new(
        "t-1",
        "x",
        &context,
        "deliver",
        Outcome::Success,
        Credits(10),
        1,
        &peers,
    )
    .unwrap();
    let applied = ledger.apply_experience(&exp).unwrap();
    assert_eq!(applied, 2 * REPUTATION_SUCCESS, "两个参与者各记一次");
    assert_eq!(ledger.score_of(&peers[0]), REPUTATION_SUCCESS);
    assert_eq!(ledger.score_of(&peers[1]), REPUTATION_SUCCESS);
    assert_eq!(ledger.updates(), 2);
}

#[test]
fn model_is_deterministic_and_canonical() {
    let peers = dids(2);
    let report = report_fixture(&peers);
    let run = || {
        let mut model = LearningModel::new(PolicyBounds::default());
        for i in 0..12 {
            let signals = Signals {
                sample: 12,
                confidence_bp: 10_000,
                accept_rate_bp: Some(3_000),
                success_bp: 5_000,
                mean_reward: Credits(10),
                prev_mean_reward: Credits::ZERO,
                prev_price_dir: 0,
                violations: 0,
                reputation_delta: 0,
            };
            let adj = adjust(
                model.params(),
                &report,
                &au4a_learning::violation::ViolationLog::new(),
                &signals,
                &PolicyBounds::default(),
                &PolicyTargets::default(),
            )
            .unwrap();
            model.apply(&adj).unwrap();
            let _ = i;
        }
        model
    };
    let a = run();
    let b = run();
    assert_eq!(a.digest().unwrap(), b.digest().unwrap());
    au4a_core::canonicalize(&a.to_value().unwrap()).unwrap();
    assert_eq!(a.generation(), 12);
    // 公开投影不含 DID
    assert!(!a.public_json().unwrap().to_string().contains("did:au4a:"));
}

#[test]
fn invalid_model_config_is_refused() {
    for bad in [
        ModelConfig {
            learning_rate_bp: -1,
            ..ModelConfig::default()
        },
        ModelConfig {
            momentum_bp: 10_001,
            ..ModelConfig::default()
        },
        ModelConfig {
            forgetting_bp: 10_001,
            ..ModelConfig::default()
        },
        ModelConfig {
            max_generations: 0,
            ..ModelConfig::default()
        },
        ModelConfig {
            max_drift_bp: -1,
            ..ModelConfig::default()
        },
    ] {
        assert_eq!(bad.validate(), Err(CoreError::InvalidKind));
    }
    let mut model = LearningModel::with_config(
        PolicyBounds::default(),
        ModelConfig {
            momentum_bp: 20_000,
            ..ModelConfig::default()
        },
    );
    assert_eq!(
        model.apply(&adjustment(-500, &[], &[])),
        Err(CoreError::InvalidKind)
    );
}

#[test]
fn market_learning_goes_through_the_model() {
    let config = au4a_learning::sim::MarketConfig::default();
    let (control, learning, comparison) = au4a_learning::sim::ab_test(&config).unwrap();
    // 学习组每一轮都推进一代；对照组代际为 0（参数从未被模型更新过）
    assert_eq!(learning.final_generation, config.rounds);
    assert_eq!(control.final_generation, 0);
    assert!(learning.params_changed() && !control.params_changed());
    // 单轮位移不超过漂移上限
    let max_drift = ModelConfig::default().max_drift_bp;
    assert!(learning
        .rounds
        .iter()
        .all(|r| r.price_drift_bp.abs() <= max_drift));
    // 信誉台账在两组都记录了观测结果，但公开投影不含 DID
    assert!(learning.final_reputation["peers"].as_i64().unwrap() > 0);
    assert!(!learning.final_reputation.to_string().contains("did:au4a:"));
    assert!(learning.rounds.iter().any(|r| r.reputation_delta != 0));
    // 学习仍然带来可量化改善（模型更新没有把学习压没）
    assert!(comparison.improved(), "{comparison:?}");
    assert!(comparison.success_lift_bp > 0 && comparison.revenue_lift_credits > Credits::ZERO);
}
