//! v1.6.3 合成市场对照实验测试：学习组真的变策略、对照组不变、提升可量化、两组面对同一市场。

use au4a_core::{canonicalize, CoreError, Credits};
use au4a_learning::policy::PolicyParams;
use au4a_learning::sim::{
    ab_test, compare, peer_profiles, run, MarketConfig, TASK_PROFILES,
};

#[test]
fn learning_arm_changes_behaviour_while_control_stays_frozen() {
    let config = MarketConfig::default();
    let (control, learning, comparison) = ab_test(&config).unwrap();

    assert!(!control.params_changed(), "对照组不得改动任何策略参数");
    assert_eq!(control.final_params, PolicyParams::baseline());
    assert!(learning.params_changed(), "学习组必须真的改变策略参数");

    // 定价：从 12000bp 的保守高报价往下走（接受率远低于目标）
    assert!(
        learning.final_params.price_bp < PolicyParams::baseline().price_bp,
        "学习组定价应下移：{} → {}",
        PolicyParams::baseline().price_bp,
        learning.final_params.price_bp
    );
    // 任务选择与协作对象选择都被学到
    assert_eq!(
        learning.final_params.task_bias_bp.len(),
        TASK_PROFILES.len(),
        "三类任务都应形成偏好"
    );
    assert!(learning.final_params.peer_bias_bp.len() >= 2);
    let biases: Vec<i64> = learning.final_params.peer_bias_bp.values().copied().collect();
    assert!(biases.iter().any(|b| *b > 0) && biases.iter().any(|b| *b < 0), "偏好应有正有负");
    // 对照组的 public 投影证明它没学
    assert_eq!(comparison.params_differ, true);
    assert!(!control.public_json().unwrap()["params_changed"].as_bool().unwrap());
}

#[test]
fn improvement_is_quantified_not_asserted() {
    let config = MarketConfig::default();
    let (control, learning, comparison) = ab_test(&config).unwrap();

    assert!(comparison.improved());
    assert!(comparison.success_lift_bp > 2_000, "成功率提升应显著：{comparison:?}");
    assert!(comparison.revenue_lift_credits > Credits::ZERO);
    assert!(comparison.revenue_lift_bp > 1_000, "收益提升应显著：{comparison:?}");
    assert!(
        comparison.learning_violations < comparison.control_violations,
        "违规次数必须下降：{} → {}",
        comparison.control_violations,
        comparison.learning_violations
    );
    // 学习组的价格更接近市场最优点（接受率更高）
    assert!(learning.accept_rate_bp() > control.accept_rate_bp());
    assert!(learning.success_rate_bp() > control.success_rate_bp());
    // 部分成功率也说明 Partial 不是被丢弃的中间档
    assert_eq!(
        learning.successes + learning.partials + (learning.ticks - learning.successes - learning.partials),
        learning.ticks
    );
}

#[test]
fn both_arms_face_the_identical_market_in_round_zero() {
    let config = MarketConfig::default();
    let (control, learning, _) = ab_test(&config).unwrap();
    let c0 = &control.rounds[0];
    let l0 = &learning.rounds[0];
    // 第一轮两组都还没有学到任何东西 → 市场结果必须逐字段相同
    assert_eq!(c0.ticks, l0.ticks);
    assert_eq!(c0.accepted, l0.accepted);
    assert_eq!(c0.successes, l0.successes);
    assert_eq!(c0.partials, l0.partials);
    assert_eq!(c0.violations, l0.violations);
    assert_eq!(c0.revenue, l0.revenue);
    assert_eq!(control.ticks, learning.ticks);
    // 第一轮两组都用初始价 → 成交结果一致；差别只出现在轮末的学习更新之后
    assert_eq!(c0.price_bp, PolicyParams::baseline().price_bp);
    assert!(l0.price_bp < PolicyParams::baseline().price_bp);
    assert!(l0.changed && !c0.changed);
    // 对照组的价格路径恒定；学习组的价格路径必须真的移动过
    assert!(control.rounds.iter().all(|r| r.price_bp == 12_000));
    assert!(learning.rounds.iter().any(|r| r.price_bp != 12_000));
}

#[test]
fn violations_are_learned_away() {
    let config = MarketConfig::default();
    let (control, learning, _) = ab_test(&config).unwrap();
    let profiles = peer_profiles(config.seed, config.peers).unwrap();
    let malicious: Vec<_> = profiles.iter().filter(|p| p.violation_bp > 1_000).collect();
    assert!(!malicious.is_empty(), "市场必须至少有一个恶意协作者");
    for m in &malicious {
        let bias = learning.final_params.bias_of_peer(&m.did);
        assert!(bias < 0, "恶意协作者偏好必须为负：{:?}", bias);
    }
    // 偏好最高的协作者不应是有违规记录的那些
    let top = profiles
        .iter()
        .max_by_key(|p| learning.final_params.bias_of_peer(&p.did))
        .expect("非空");
    assert!(
        top.violation_bp <= 1_000,
        "偏好最高的协作者竟然是恶意的：{:?}",
        top
    );
    assert!(learning.violations <= control.violations);
}

#[test]
fn runs_are_byte_identical_across_two_invocations() {
    let config = MarketConfig::default();
    let first = ab_test(&config).unwrap();
    let second = ab_test(&config).unwrap();
    assert_eq!(first.0.digest().unwrap(), second.0.digest().unwrap());
    assert_eq!(first.1.digest().unwrap(), second.1.digest().unwrap());
    assert_eq!(first.2, second.2);
    // 规范 JSON 必须能编码（无浮点），且两次跑批逐字节一致
    let a = canonicalize(&first.1.to_value().unwrap()).unwrap();
    let b = canonicalize(&second.1.to_value().unwrap()).unwrap();
    assert_eq!(a, b);
    // 换个种子 → 换一个市场（结果不同，但仍然可复现）
    let other = MarketConfig {
        seed: 7,
        ..MarketConfig::default()
    };
    let third = ab_test(&other).unwrap();
    assert_ne!(third.2, first.2);
}

#[test]
fn public_projection_of_a_run_never_leaks_peer_dids() {
    let config = MarketConfig::default();
    let (_, learning, _) = ab_test(&config).unwrap();
    let text = learning.public_json().unwrap().to_string();
    assert!(!text.contains("did:au4a:"), "公开投影泄露 DID");
    assert!(learning.public_json().unwrap()["rounds"].as_array().unwrap().len() > 0);
}

#[test]
fn different_markets_are_not_comparable() {
    let a = run(&MarketConfig::default()).unwrap();
    let b = run(&MarketConfig {
        seed: 99,
        ..MarketConfig::default()
    })
    .unwrap();
    assert_eq!(compare(&a, &b), Err(CoreError::InvalidKind));
}

#[test]
fn invalid_configuration_is_refused() {
    for bad in [
        MarketConfig {
            rounds: 0,
            ..MarketConfig::default()
        },
        MarketConfig {
            ticks_per_round: 1, // < MIN_SAMPLES：一轮攒不出一次学习
            ..MarketConfig::default()
        },
        MarketConfig {
            peers: 1,
            ..MarketConfig::default()
        },
        MarketConfig {
            peers: 17,
            ..MarketConfig::default()
        },
        MarketConfig {
            noise_span_bp: -1,
            ..MarketConfig::default()
        },
    ] {
        assert_eq!(bad.validate(), Err(CoreError::InvalidKind));
        assert_eq!(run(&bad), Err(CoreError::InvalidKind));
        assert_eq!(ab_test(&bad), Err(CoreError::InvalidKind));
    }
    assert!(peer_profiles(1, 0).is_err());
    assert!(peer_profiles(1, 17).is_err());
}

#[test]
fn track_self_check_covers_the_learning_evidence() {
    let checks = au4a_learning::self_check();
    assert!(au4a_core::all_passed(&checks), "自检未全绿");
    let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
    for expected in [
        "market.learning_changes_behaviour",
        "market.quantified_improvement",
        "market.reproducible",
        "policy.evidence_gate",
        "policy.three_levers_move",
        "scenario.learning_changes_behaviour",
    ] {
        assert!(names.contains(&expected), "缺少自检项 {expected}");
    }
    assert!(checks.len() >= 19, "自检项数量不足：{}", checks.len());
}

/// 把实测对照数据打进测试日志（证据留档：docs/tracks/1.6.md 里的数字来自这里）。
#[test]
fn evidence_snapshot_of_the_ab_experiment() {
    let config = MarketConfig::default();
    let (control, learning, comparison) = ab_test(&config).unwrap();
    println!(
        "AB evidence: ticks={} control(success={}bp accept={}bp revenue={} violations={} price={}bp) \
         learning(success={}bp accept={}bp revenue={} violations={} price={}bp) lift(success=+{}bp revenue=+{} = +{}bp)",
        comparison.ticks,
        comparison.control_success_bp,
        control.accept_rate_bp(),
        comparison.control_revenue,
        comparison.control_violations,
        comparison.control_price_bp,
        comparison.learning_success_bp,
        learning.accept_rate_bp(),
        comparison.learning_revenue,
        comparison.learning_violations,
        comparison.learning_price_bp,
        comparison.success_lift_bp,
        comparison.revenue_lift_credits,
        comparison.revenue_lift_bp
    );
    println!(
        "learning price path: {:?}",
        learning.rounds.iter().map(|r| r.price_bp).collect::<Vec<_>>()
    );
    println!(
        "control price path: {:?}",
        control.rounds.iter().map(|r| r.price_bp).collect::<Vec<_>>()
    );
    assert!(comparison.improved());
}
