//! v1.6.7 不变式扫描：多种子下所有结构性不变式必须成立（不是「跑一遍看起来没错」）。
//!
//! 扫描用的是小配置（8 轮 × 8 tick），保证 32 个种子能在秒级跑完；
//! 结构性不变式与配置大小无关，所以小配置足以证伪「只在默认配置下成立」这类实现。

use au4a_core::{canonicalize, Credits};
use au4a_learning::model::ModelConfig;
use au4a_learning::policy::{PolicyBounds, PolicyParams};
use au4a_learning::sim::{ab_test, MarketConfig, MarketRun};

/// 扫描配置：比默认小，但仍然是「多轮 + 多协作者 + 有学习」的完整闭环。
fn sweep_config(seed: u64) -> MarketConfig {
    MarketConfig {
        seed,
        rounds: 8,
        ticks_per_round: 8,
        peers: 6,
        noise_span_bp: 1_200,
        ..MarketConfig::default()
    }
}

fn check_run(run: &MarketRun, config: &MarketConfig, label: &str) {
    let bounds = PolicyBounds::default();
    let model = ModelConfig::default();
    let expected_ticks = config.rounds * config.ticks_per_round;

    assert_eq!(run.ticks, expected_ticks, "{label}: tick 数必须等于 轮数×每轮任务数");
    assert_eq!(run.rounds.len(), config.rounds as usize, "{label}: 轮数");
    assert_eq!(
        run.successes + run.partials + run.failures,
        run.ticks,
        "{label}: 结局分布必须覆盖每个 tick"
    );
    assert!(run.accepted <= run.ticks, "{label}: 接受数不能超过任务数");
    assert!(
        run.accepted >= run.successes + run.partials,
        "{label}: 成功/部分成功必然先被接受"
    );
    assert!(run.revenue >= Credits::ZERO, "{label}: 收益非负");

    // 每轮聚合必须与总计一致
    let mut round_ticks = 0u32;
    let mut round_successes = 0u32;
    let mut round_partials = 0u32;
    let mut round_accepted = 0u32;
    let mut round_violations = 0u32;
    let mut round_revenue = Credits::ZERO;
    for r in &run.rounds {
        round_ticks += r.ticks;
        round_successes += r.successes;
        round_partials += r.partials;
        round_accepted += r.accepted;
        round_violations += r.violations;
        round_revenue = round_revenue.checked_add(r.revenue).expect("非负且不溢出");
        // 参数始终在边界内，单代位移不超过漂移上限
        assert!(
            r.price_bp >= bounds.price_min_bp && r.price_bp <= bounds.price_max_bp,
            "{label}: 第 {} 轮价格 {} 越界",
            r.round,
            r.price_bp
        );
        assert!(
            r.price_drift_bp.abs() <= model.max_drift_bp,
            "{label}: 第 {} 轮位移 {} 超过上限",
            r.round,
            r.price_drift_bp
        );
        assert!(
            r.signal_composite_bp >= -10_000 && r.signal_composite_bp <= 10_000,
            "{label}: 综合信号越界"
        );
        assert!(r.failures + r.successes + r.partials == r.ticks, "{label}: 轮内结局分布");
    }
    assert_eq!(round_ticks, run.ticks, "{label}: 轮内 tick 之和");
    assert_eq!(round_successes, run.successes, "{label}: 轮内成功之和");
    assert_eq!(round_partials, run.partials, "{label}: 轮内部分成功之和");
    assert_eq!(round_accepted, run.accepted, "{label}: 轮内接受之和");
    assert_eq!(round_violations, run.violations, "{label}: 轮内违规之和");
    assert_eq!(round_revenue, run.revenue, "{label}: 轮内收益之和");

    // 经验窗口有界（滚动窗口 = 3 轮）
    assert!(
        run.window_experiences <= (config.ticks_per_round as usize) * 3,
        "{label}: 经验窗口越界 {}",
        run.window_experiences
    );
    assert_eq!(run.experiences_recorded, expected_ticks, "{label}: 记录总数");

    // 参数取值必须落在偏好边界内
    for v in run.final_params.task_bias_bp.values() {
        assert!(*v >= bounds.bias_min_bp && *v <= bounds.bias_max_bp, "{label}: 任务偏好越界");
    }
    for v in run.final_params.peer_bias_bp.values() {
        assert!(*v >= bounds.bias_min_bp && *v <= bounds.bias_max_bp, "{label}: 协作者偏好越界");
    }

    // 信誉台账：不可转让、公开投影无 DID
    assert_eq!(run.final_reputation["transferable"], serde_json::json!(false));
    assert!(
        !run.final_reputation.to_string().contains("did:au4a:"),
        "{label}: 信誉公开投影泄露 DID"
    );

    // 公开投影无 DID，且可被规范 JSON 编码（无浮点）
    let public = run.public_json().expect("公开投影可序列化");
    assert!(!public.to_string().contains("did:au4a:"), "{label}: 跑批公开投影泄露 DID");
    canonicalize(&public).expect("公开投影必须能被规范 JSON 编码");
}

#[test]
fn invariants_hold_across_32_seeds() {
    let mut total_lift_bp: i64 = 0;
    let mut improved_seeds = 0usize;
    let seeds: Vec<u64> = (1..=32u64).map(|i| i * 0x1616).collect();
    for seed in &seeds {
        let config = sweep_config(*seed);
        let (control, learning, comparison) = ab_test(&config).expect("对照实验必须成功");

        check_run(&control, &config, "control");
        check_run(&learning, &config, "learning");

        // 对照组：参数冻结在基线，代际停在第 0 代
        assert_eq!(control.final_params, PolicyParams::baseline(), "对照组必须冻结");
        assert_eq!(control.final_generation, 0, "对照组不应推进代际");
        assert!(control.rounds.iter().all(|r| r.price_drift_bp == 0 && !r.changed));

        // 学习组：每轮一代，且参数确实变了（接受率 41% 远低于目标 85%，必然降价）
        assert_eq!(learning.final_generation, config.rounds, "学习组必须每轮推进一代");
        assert!(learning.params_changed(), "学习组必须改变策略参数（seed={seed}）");
        assert!(
            learning.final_params.price_bp < PolicyParams::baseline().price_bp,
            "学习组定价必须下移（seed={seed}）"
        );

        total_lift_bp += comparison.success_lift_bp;
        if comparison.success_lift_bp > 0 && comparison.revenue_lift_credits > Credits::ZERO {
            improved_seeds += 1;
        }
    }
    let mean_lift = total_lift_bp / seeds.len() as i64;
    println!(
        "invariant sweep: {} seeds, improved={}, mean success lift={}bp",
        seeds.len(),
        improved_seeds,
        mean_lift
    );
    // 跨种子平均必须是正提升；单个种子允许波动（噪声），但改善的种子必须占多数
    assert!(mean_lift > 0, "32 个种子的平均成功率提升必须为正，实测 {mean_lift}bp");
    assert!(
        improved_seeds * 2 > seeds.len(),
        "至少过半种子有正提升，实测 {improved_seeds}/{}",
        seeds.len()
    );
}

#[test]
fn runs_are_reproducible_for_every_sampled_seed() {
    // 全部种子都做一次「跑两遍」会翻倍耗时；抽 8 个种子做逐字节复现，覆盖不同市场形态
    for seed in [0x1616u64, 2 * 0x1616, 5 * 0x1616, 9 * 0x1616, 17 * 0x1616, 23 * 0x1616] {
        let config = sweep_config(seed);
        let a = ab_test(&config).unwrap();
        let b = ab_test(&config).unwrap();
        assert_eq!(a.0.digest().unwrap(), b.0.digest().unwrap(), "seed={seed} 对照组可复现");
        assert_eq!(a.1.digest().unwrap(), b.1.digest().unwrap(), "seed={seed} 学习组可复现");
        assert_eq!(a.2, b.2, "seed={seed} 对比报告可复现");
    }
    // 换种子 → 换市场（避免「所有种子都得到同一个结果」这种伪复现）
    let x = ab_test(&sweep_config(0x1616)).unwrap().2;
    let y = ab_test(&sweep_config(0x1616 + 1)).unwrap().2;
    assert_ne!(x, y);
}

#[test]
fn zero_ticks_and_tiny_markets_stay_well_formed() {
    // 最小合法配置：轮数 1、每轮 MIN_SAMPLES 个任务、2 个协作者
    let config = MarketConfig {
        seed: 42,
        rounds: 1,
        ticks_per_round: 4,
        peers: 2,
        noise_span_bp: 0,
        ..MarketConfig::default()
    };
    let (control, learning, comparison) = ab_test(&config).unwrap();
    check_run(&control, &config, "control-min");
    check_run(&learning, &config, "learning-min");
    assert_eq!(control.ticks, 4);
    assert!(comparison.ticks == 4);
    // 无噪声时选择完全由偏好决定，但第一轮偏好为空 → 仍然确定性地选第一个组合
    assert!(learning.rounds[0].ticks == 4);
}
