//! v1.9.7 端到端与边界测试：只用公开 API，把整条链路跑通并检查它的**结论**。
//!
//! 三件事在这里被独立验证（不依赖 crate 内部状态）：
//! 1. 链路一致性：收集 → 拟合 → 裁决，三者对同一份参数给出一致的结论；
//! 2. 确定性：同一输入下 digest / 裁决 / scenario 输出完全一致；
//! 3. 有界性：10k 档位与完整的档位+裁决流程在 1 秒内完成（本机实测墙钟）。

use std::time::Instant;

use au4a_kernel::{Kernel, KernelConfig};
use au4a_scale::{
    adjudicate, collect, evaluation_budget, fit, scenario, sweep, tier_report, DataBundle,
    ExperimentConfig, FitSearch, ScalingParams, ScenarioSpec, VerdictKind, VerdictReason, TIERS,
};

fn baseline() -> ScalingParams {
    ScalingParams::new(1_000, 2, 1_000_000, 1)
}

#[test]
fn the_whole_chain_collect_fit_adjudicate_agrees_on_the_same_model() {
    let params = baseline();
    let nodes = vec![10, 100, 500, 1_000, 2_000, 5_000, 10_000];
    let bundle: DataBundle =
        collect(&[ScenarioSpec::new("chain", nodes.clone(), params, 100_000)]).unwrap();
    assert_eq!(bundle.len(), nodes.len());
    assert_eq!(bundle.digest, bundle.recompute_digest().unwrap());

    // 拟合出的参数必须能解释这份数据（残差是「解释力」的直接度量）。
    let report = fit(&bundle, &FitSearch::around(1_000, 500, 10, vec![1, 2, 3])).unwrap();
    assert_eq!(report.alpha_estimate, 2);
    assert!(report.n0_estimate.abs_diff(1_000) <= 50);
    assert!(
        report.max_residual_ppm <= 10_000,
        "残差 {} ppm",
        report.max_residual_ppm
    );

    // 裁决给出的容量顶点必须与解析式一致（α=2 → n* = N0）。
    let verdict = adjudicate(&params, 100_000, 1, 4_000).unwrap();
    assert_eq!(verdict.kind, VerdictKind::VertexFound);
    assert!(verdict.vertex_nodes.abs_diff(1_000) <= verdict.step);
    assert!(verdict.has(VerdictReason::OverheadWorsened));
    assert!(verdict.has(VerdictReason::MarginalTurnedNegative));

    // 档位报告与实验框架对同一档位给同一数值。
    let tiers = tier_report(&params, 100_000).unwrap();
    let experiment =
        au4a_scale::run_experiment(&ExperimentConfig::new(TIERS.to_vec(), params, 100_000))
            .unwrap();
    for tier in &tiers.tiers {
        let row = experiment.row(tier.nodes).unwrap();
        assert_eq!(row.aggregate_milli, tier.aggregate_milli);
        assert_eq!(row.net_milli, tier.net_milli);
        assert_eq!(row.overhead_ratio_bp, tier.overhead_ratio_bp);
    }
}

#[test]
fn repeated_runs_produce_identical_digests_and_verdicts() {
    let params = baseline();
    let first = sweep(&TIERS, &[500, 1_000], &[2], 1_000_000, 1, 100_000).unwrap();
    let second = sweep(&TIERS, &[500, 1_000], &[2], 1_000_000, 1, 100_000).unwrap();
    assert_eq!(first.digest, second.digest);
    assert_eq!(first, second);

    let first_verdict = adjudicate(&params, 100_000, 1, 4_000).unwrap();
    let second_verdict = adjudicate(&params, 100_000, 1, 4_000).unwrap();
    assert_eq!(first_verdict, second_verdict);
}

#[test]
fn the_ten_thousand_node_path_stays_within_the_time_and_evaluation_bounds() {
    let params = ScalingParams::new(10_000, 2, 1_000_000, 1);
    let start = Instant::now();
    let tiers = tier_report(&params, 100_000).unwrap();
    let verdict = adjudicate(&params, 100_000, 1, 40_000).unwrap();
    let elapsed = start.elapsed();

    assert_eq!(tiers.tiers[3].nodes, 10_000);
    assert!(tiers.evaluations <= evaluation_budget());
    assert!(verdict.evaluations <= evaluation_budget());
    assert!(
        elapsed.as_millis() < 1_000,
        "10k 档位 + 裁决耗时 {}ms，超过 1s",
        elapsed.as_millis()
    );
    // 声明：这是**聚合模型**评估，不是分布式压测。
    assert_eq!(tiers.tiers.len(), 4, "档位数量固定，不随 N 增长");
}

#[test]
fn negative_returns_are_flagged_at_the_exact_tier() {
    // 每次交互成本 10 任务：小规模净赚、大规模净亏。
    let expensive = ScalingParams::new(1_000, 2, 1_000_000, 10_000);
    let verdict = adjudicate(&expensive, 100_000, 1, 10_000).unwrap();
    assert!(verdict.has(VerdictReason::NetTurnedNegative));
    let turned = verdict.net_negative_from.expect("必须给出首次转负点");
    assert!(au4a_scale::net_throughput_milli(turned, &expensive).unwrap() < 0);
    assert!(au4a_scale::net_throughput_milli(1, &expensive).unwrap() > 0);
    assert!(turned > 1);

    let cheap = ScalingParams::new(1_000, 2, 1_000_000, 0);
    assert!(!adjudicate(&cheap, 100_000, 1, 4_000)
        .unwrap()
        .has(VerdictReason::NetTurnedNegative));
}

#[test]
fn the_scenario_is_byte_identical_across_fresh_kernels() {
    let mut first = Kernel::new(KernelConfig::default());
    let mut second = Kernel::new(KernelConfig::default());
    let a = scenario(&mut first).unwrap();
    let b = scenario(&mut second).unwrap();
    assert_eq!(a, b);
    assert_eq!(a["digest"], b["digest"]);
    assert_eq!(a["verdict"]["kind"], b["verdict"]["kind"]);
    assert_eq!(
        a["note"],
        serde_json::json!("确定性聚合模型（解析式实现），非真实分布式压测")
    );
}

#[test]
fn the_analytic_vertex_is_where_the_discrete_curve_peaks() {
    // 数学表达 ↔ 数值：解析极值点与「档位报告里吞吐最大的点」在同一步长内。
    let params = ScalingParams::new(1_000, 3, 1_000_000, 1);
    let analytic = au4a_scale::analytic_vertex_floor(params.n0, params.alpha);
    let verdict = adjudicate(&params, 100_000, 1, 4_000).unwrap();
    assert!(
        verdict.vertex_nodes.abs_diff(analytic) <= verdict.step,
        "顶点 {} 与解析 {analytic} 相差超过步长 {}",
        verdict.vertex_nodes,
        verdict.step
    );
    assert_eq!(verdict.kind, VerdictKind::VertexFound);
}
