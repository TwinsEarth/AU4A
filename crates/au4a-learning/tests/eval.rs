//! v1.6.10 评估测试：固定种子对照评估、跨种子稳健性、消融实验、可复现与可发布。

use au4a_core::{canonicalize, CoreError, Credits};
use au4a_learning::eval::{ablations, evaluate, EvalConfig, EVAL_SEEDS};
use au4a_learning::sim::MarketConfig;

#[test]
fn fixed_seed_evaluation_quantifies_the_lift() {
    let config = EvalConfig::default();
    let report = evaluate(&config).unwrap();

    assert_eq!(report.seeds, EVAL_SEEDS.len());
    assert_eq!(report.per_seed.len(), EVAL_SEEDS.len());
    // 每个 arm 的 tick 数 = 种子数 × 轮数 × 每轮任务数
    let expected_ticks = EVAL_SEEDS.len() as u32 * 16 * 12;
    assert_eq!(report.ticks_per_arm, expected_ticks);
    assert_eq!(report.control.ticks, expected_ticks);
    assert_eq!(report.learning.ticks, expected_ticks);
    assert!(report.improved(), "评估必须给出正提升：{report:?}");
    assert!(report.stable(), "跨种子必须稳定：{report:?}");
    assert!(report.success_lift_bp > 0);
    assert!(report.revenue_lift_credits > Credits::ZERO);
    assert!(report.revenue_lift_bp > 0);
    assert!(report.violations_reduction >= 0);
    assert!(report.median_success_lift_bp > 0);
    assert!(report.improved_seeds * 2 >= report.seeds);
    assert!(report.negative_seeds * 3 <= report.seeds, "明显变差的种子不能超过 1/3");
    // 每个种子里学习组都真的改了参数，对照组都没改
    for s in &report.per_seed {
        assert!(s.params_differ, "seed={} 两组参数必须不同", s.seed);
        assert!(s.learning_price_bp < 12_000, "学习组定价必须下移（seed={}）", s.seed);
        assert!(s.learning_revenue >= Credits::ZERO);
    }
    // 汇总数字必须与逐种子数字自洽
    let sum_revenue: i64 = report.per_seed.iter().map(|s| s.learning_revenue.get()).sum();
    assert_eq!(sum_revenue, report.learning.revenue.get());
    let sum_control: i64 = report.per_seed.iter().map(|s| s.control_revenue.get()).sum();
    assert_eq!(sum_control, report.control.revenue.get());
    assert_eq!(report.control.params_changed_runs, 0, "对照组不应有一次参数变化");
    assert_eq!(report.learning.params_changed_runs, report.seeds as u32);
    println!(
        "eval evidence: seeds={} control(success={}bp revenue={} violations={}) learning(success={}bp revenue={} violations={}) lift(success=+{}bp median=+{}bp min={}bp max={}bp revenue=+{}bp improved_seeds={} negative={})",
        report.seeds,
        report.control.success_rate_bp,
        report.control.revenue,
        report.control.violations,
        report.learning.success_rate_bp,
        report.learning.revenue,
        report.learning.violations,
        report.success_lift_bp,
        report.median_success_lift_bp,
        report.min_success_lift_bp,
        report.max_success_lift_bp,
        report.revenue_lift_bp,
        report.improved_seeds,
        report.negative_seeds
    );
}

#[test]
fn evaluation_is_reproducible_and_publishable() {
    let config = EvalConfig::default().limited(3);
    let a = evaluate(&config).unwrap();
    let b = evaluate(&config).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.digest().unwrap(), b.digest().unwrap());
    assert_eq!(
        canonicalize(&a.to_value().unwrap()).unwrap(),
        canonicalize(&b.to_value().unwrap()).unwrap()
    );
    let summary = a.summary().unwrap();
    assert!(!summary.to_string().contains("did:au4a:"));
    assert_eq!(summary["seeds"], 3);
    assert_eq!(summary["stable"], serde_json::json!(true));
    // 换种子集合 → 换报告
    let other = evaluate(&EvalConfig {
        seeds: vec![999, 1_000, 1_001],
        ..EvalConfig::default()
    })
    .unwrap();
    assert_ne!(a.digest().unwrap(), other.digest().unwrap());
}

#[test]
fn cross_seed_stability_holds_with_margin() {
    let report = evaluate(&EvalConfig::default()).unwrap();
    // 稳健性用分布说话：中位数为正、最差种子不至于崩、改善占多数
    assert!(report.median_success_lift_bp > 0);
    assert!(
        report.min_success_lift_bp > -2_000,
        "最差种子不应大幅倒退：{}bp",
        report.min_success_lift_bp
    );
    assert!(
        report.improved_seeds >= report.seeds * 2 / 3,
        "至少 2/3 的种子改善：{}/{}",
        report.improved_seeds,
        report.seeds
    );
    // 更多种子（抽样 8 个）仍然稳定
    let wide = evaluate(&EvalConfig {
        seeds: (1..=8u64).map(|i| i * 0x1616).collect(),
        ..EvalConfig::default()
    })
    .unwrap();
    assert!(wide.stable(), "8 种子扫描也必须稳定：{wide:?}");
    assert!(wide.improved_seeds * 2 > wide.seeds);
}

#[test]
fn ablations_show_which_lever_carries_the_improvement() {
    let config = EvalConfig::default().limited(3);
    let rows = ablations(&config).unwrap();
    assert_eq!(rows.len(), 4);
    let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(labels, vec!["price-only", "task-only", "peer-only", "all-levers"]);

    let price = rows.iter().find(|r| r.label == "price-only").unwrap();
    let task = rows.iter().find(|r| r.label == "task-only").unwrap();
    let peer = rows.iter().find(|r| r.label == "peer-only").unwrap();
    let all = rows.iter().find(|r| r.label == "all-levers").unwrap();

    // 定价是主杠杆（接受率远低于目标 → 降价带来最多成交量）
    assert!(price.success_lift_bp > 0, "只学定价也必须有正提升：{price:?}");
    assert!(price.revenue_lift_bp > 0);
    // 三个杠杆全开必须至少不差于只学任务/只学协作
    assert!(all.success_lift_bp > 0);
    assert!(all.success_lift_bp >= task.success_lift_bp);
    assert!(all.success_lift_bp >= peer.success_lift_bp);
    // 只学协作对象会带来违规下降（恶意协作者被避开）
    assert!(peer.violations_reduction >= 0);
    assert!(all.revenue_lift_bp >= task.revenue_lift_bp);
    println!(
        "ablation evidence: price(+{}bp/+{}bp) task(+{}bp/+{}bp) peer(+{}bp/+{}bp) all(+{}bp/+{}bp)",
        price.success_lift_bp,
        price.revenue_lift_bp,
        task.success_lift_bp,
        task.revenue_lift_bp,
        peer.success_lift_bp,
        peer.revenue_lift_bp,
        all.success_lift_bp,
        all.revenue_lift_bp
    );
}

#[test]
fn invalid_evaluation_config_is_refused() {
    assert_eq!(
        evaluate(&EvalConfig {
            seeds: Vec::new(),
            ..EvalConfig::default()
        }),
        Err(CoreError::InvalidKind)
    );
    assert_eq!(
        evaluate(&EvalConfig {
            seeds: (0..65u64).collect(),
            ..EvalConfig::default()
        }),
        Err(CoreError::InvalidKind)
    );
    assert_eq!(
        evaluate(&EvalConfig {
            market: MarketConfig {
                rounds: 0,
                ..MarketConfig::default()
            },
            ..EvalConfig::default()
        }),
        Err(CoreError::InvalidKind)
    );
    // 三个杠杆全屏蔽但 learn=true → 配置自相矛盾
    assert_eq!(
        evaluate(&EvalConfig {
            market: MarketConfig {
                learn_price: false,
                learn_task: false,
                learn_peer: false,
                ..MarketConfig::default()
            },
            seeds: vec![1],
        }),
        Err(CoreError::InvalidKind)
    );
    // limited() 至少要保留一个种子
    assert_eq!(EvalConfig::default().limited(0).seeds.len(), 1);
}

#[test]
fn self_check_covers_the_evaluation() {
    let checks = au4a_learning::self_check();
    assert!(au4a_core::all_passed(&checks), "自检未全绿");
    let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"eval.learning_beats_control"));
    assert!(names.contains(&"eval.reproducible_and_publishable"));
    assert!(checks.len() >= 26, "自检项数量不足：{}", checks.len());
}
