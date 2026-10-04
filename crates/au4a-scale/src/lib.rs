//! AU4A 轨道 1.9 — Network Scaling 网络扩展（v1.9.1 → v1.9.9）。
//!
//! 本轨道把「网络结构的 Scaling Law」形式化：**节点数 / 交互复杂度 / 算力预算**如何决定
//! **群体智能水平**（以群体吞吐、完成率、净收益表达），并给出可复现的容量顶点裁决。
//!
//! 两条必须先说清的边界（证据分级）：
//!
//! 1. **本轨道是确定性聚合模型，不是真实分布式压测。** 10k 节点通过解析/聚合模型评估，
//!    秒级完成、内存有界；代码里没有 10k 个线程，也没有网络。
//! 2. 模型推论（外推曲线、极值点位置的解析式）与被实测部分（本机跑出的数值、
//!    与解析式的一致性）在文档里分开标注；`verified` 只用于本机真实执行的断言。
//!
//! 轨道内**串行**开发：v1.9.1 度量 → v1.9.2 实验框架 → v1.9.3 大规模集群 →
//! v1.9.4 效果评估 → v1.9.5 数据收集 → v1.9.6 分析工具 → v1.9.7 测试 →
//! v1.9.8 文档 → v1.9.9 论文。轨道间**零耦合**：只依赖 `au4a-core` 与 `au4a-kernel`。

pub mod analyze;
pub mod cluster;
pub mod collect;
pub mod harness;
pub mod metrics;
pub mod verdict;

use au4a_core::{CoreResult, SelfCheck};
use au4a_kernel::Kernel;
use serde_json::{json, Value};

pub use analyze::{fit, residuals, FitReport, FitSearch};
pub use cluster::{
    bounded_vertex_scan, evaluation_budget, tier_report, TierReport, TierRow, VertexScan,
    MAX_SCAN_SAMPLES, TIERS,
};
pub use collect::{collect, sweep, DataBundle, Record, ScenarioSpec};
pub use harness::{run as run_experiment, ExperimentConfig, ExperimentReport, Row};
pub use verdict::{adjudicate, CapacityVerdict, VerdictKind, VerdictReason};
pub use metrics::{
    aggregate_throughput_milli, analytic_vertex_floor, completion_bp, coordination_base,
    effective_per_node_milli, interaction_complexity, marginal_gain_milli, net_throughput_milli,
    orchestration_overhead_milli, overhead_ratio_bp, ScalingParams,
};

/// 轨道号。
pub const TRACK: &str = "1.9";
/// 轨道标题。
pub const TITLE: &str = "Network Scaling 网络扩展";
/// 版本区间。
pub const RANGE: &str = "v1.9.1 → v1.9.9";
/// 当前小版本（每个小版本落地时前移）。
pub const CURRENT: &str = "v1.9.6";
/// 编译期存在性标记：确保 crate 名与轨道号一致。
pub const CRATE: &str = "au4a_scale";

/// 轨道自检：节点 `verify` 聚合它，观察层「结果」面板展示它。
pub fn self_check() -> Vec<SelfCheck> {
    let mut checks = Vec::new();
    let params = ScalingParams::default();

    checks.push(match params.validate() {
        Ok(()) => SelfCheck::pass(
            TRACK,
            "metrics.params_valid",
            format!(
                "默认参数 N0={} α={} p0={}milli c={}milli 通过准入",
                params.n0, params.alpha, params.p0_milli, params.interaction_cost_milli
            ),
        ),
        Err(err) => SelfCheck::fail(TRACK, "metrics.params_valid", err.to_string()),
    });

    checks.push(match aggregate_throughput_milli(params.n0, &params) {
        Ok(value) => {
            // 手算：p(N0) = p0/2 → T(N0) = N0·p0/2。
            let expected = (params.n0 as i64) * (params.p0_milli / 2);
            if value == expected {
                SelfCheck::pass(
                    TRACK,
                    "metrics.half_point",
                    format!("T(N0)={value} == N0·p0/2（N0 的定义：单节点效率减半）"),
                )
            } else {
                SelfCheck::fail(
                    TRACK,
                    "metrics.half_point",
                    format!("T(N0)={value}，期望 {expected}"),
                )
            }
        }
        Err(err) => SelfCheck::fail(TRACK, "metrics.half_point", err.to_string()),
    });

    checks.push({
        let peaked = ScalingParams::new(1_000, 2, 1_000, 1);
        let before = aggregate_throughput_milli(100, &peaked).unwrap_or(0);
        let peak = aggregate_throughput_milli(1_000, &peaked).unwrap_or(0);
        let after = aggregate_throughput_milli(10_000, &peaked).unwrap_or(0);
        if peak > before && peak > after {
            SelfCheck::pass(
                TRACK,
                "metrics.vertex_exists",
                format!("α=2 时 T(100)={before} < T(1000)={peak} > T(10000)={after}（峰后衰退）"),
            )
        } else {
            SelfCheck::fail(
                TRACK,
                "metrics.vertex_exists",
                format!("{before}/{peak}/{after} 未表现出峰值"),
            )
        }
    });

    checks.push({
        let analytic = analytic_vertex_floor(1_000, 2);
        if analytic == 1_000 {
            SelfCheck::pass(
                TRACK,
                "metrics.analytic_vertex",
                "解析极值点：α=2 时 n* = N0/(α−1)^(1/α) = N0 = 1000（整数二分求解）",
            )
        } else {
            SelfCheck::fail(
                TRACK,
                "metrics.analytic_vertex",
                format!("解析顶点 {analytic}，期望 1000"),
            )
        }
    });

    // v1.9.6：从观测数据反推参数必须能恢复真值（有依据的拟合）。
    checks.push({
        let params = ScalingParams::new(1_000, 2, 1_000_000, 0);
        let bundle = collect(&[ScenarioSpec::new(
            "fit-probe",
            vec![10, 100, 500, 1_000, 2_000, 5_000],
            params,
            100_000,
        )]);
        match bundle.and_then(|bundle| {
            let search = FitSearch::around(1_000, 200, 10, vec![1, 2, 3]);
            fit(&bundle, &search)
        }) {
            Ok(report) => {
                let delta = report.n0_estimate.abs_diff(1_000);
                if report.alpha_estimate == 2 && delta <= 50 && report.max_residual_ppm <= 10_000 {
                    SelfCheck::pass(
                        TRACK,
                        "analyze.recovers_truth",
                        format!(
                            "从 6 条观测恢复 α={} N0={}（真值 2/1000，最大残差 {} ppm）",
                            report.alpha_estimate, report.n0_estimate, report.max_residual_ppm
                        ),
                    )
                } else {
                    SelfCheck::fail(
                        TRACK,
                        "analyze.recovers_truth",
                        format!(
                            "恢复 α={} N0={} 残差 {} ppm",
                            report.alpha_estimate, report.n0_estimate, report.max_residual_ppm
                        ),
                    )
                }
            }
            Err(err) => SelfCheck::fail(TRACK, "analyze.recovers_truth", err.to_string()),
        }
    });

    // v1.9.5：数据收集必须确定性且内容寻址。
    checks.push(match collect(&[
        ScenarioSpec::new(
            "baseline",
            TIERS.to_vec(),
            ScalingParams::default(),
            20_000,
        ),
    ]) {
        Ok(bundle) => match bundle.recompute_digest() {
            Ok(recomputed) if recomputed == bundle.digest && bundle.len() == TIERS.len() => {
                SelfCheck::pass(
                    TRACK,
                    "collect.content_addressed",
                    format!(
                        "{} 条记录，digest={} 可独立复算",
                        bundle.len(),
                        au4a_core::short_id(&bundle.digest)
                    ),
                )
            }
            Ok(_) => SelfCheck::fail(TRACK, "collect.content_addressed", "digest 复算不一致"),
            Err(err) => SelfCheck::fail(TRACK, "collect.content_addressed", err.to_string()),
        },
        Err(err) => SelfCheck::fail(TRACK, "collect.content_addressed", err.to_string()),
    });

    // v1.9.4：容量顶点裁决必须复现，且原因码与数值一致。
    checks.push({
        let params = ScalingParams::new(1_000, 2, 1_000_000, 1);
        match (
            adjudicate(&params, 100_000, 1, 4_000),
            adjudicate(&params, 100_000, 1, 4_000),
        ) {
            (Ok(first), Ok(second)) => {
                let analytic = analytic_vertex_floor(params.n0, params.alpha);
                if first == second
                    && first.kind == VerdictKind::VertexFound
                    && first.has(VerdictReason::MarginalTurnedNegative)
                    && first.vertex_nodes.abs_diff(analytic) <= first.step
                {
                    SelfCheck::pass(
                        TRACK,
                        "verdict.capacity_vertex",
                        format!(
                            "顶点 {}（解析 {analytic}，步长 {}），边际收益自 {} 起转负，开销占比 {} → {} ppm",
                            first.vertex_nodes,
                            first.step,
                            first.marginal_negative_from.unwrap_or(0),
                            first.overhead_ratio_at_peak_ppm,
                            first.overhead_ratio_after_peak_ppm
                        ),
                    )
                } else {
                    SelfCheck::fail(
                        TRACK,
                        "verdict.capacity_vertex",
                        format!("裁决不符合预期：{:?}", first.kind),
                    )
                }
            }
            (Err(err), _) | (_, Err(err)) => {
                SelfCheck::fail(TRACK, "verdict.capacity_vertex", err.to_string())
            }
        }
    });

    // v1.9.3：大规模评估必须在固定预算内完成（与 N 无关）。
    checks.push(match tier_report(&ScalingParams::default(), 20_000) {
        Ok(report) => {
            if report.evaluations <= evaluation_budget() && report.tiers.len() == TIERS.len() {
                SelfCheck::pass(
                    TRACK,
                    "cluster.bounded_evaluation",
                    format!(
                        "10/100/1k/10k 档位 + 顶点扫描共用 {} 次求值（预算 {}），digest={}",
                        report.evaluations,
                        evaluation_budget(),
                        au4a_core::short_id(&report.digest)
                    ),
                )
            } else {
                SelfCheck::fail(
                    TRACK,
                    "cluster.bounded_evaluation",
                    format!("求值 {} 次，超出预算 {}", report.evaluations, evaluation_budget()),
                )
            }
        }
        Err(err) => SelfCheck::fail(TRACK, "cluster.bounded_evaluation", err.to_string()),
    });
    // v1.9.2：实验框架必须确定性且内容寻址。
    checks.push(match (
        harness::run(&ExperimentConfig::default_tiers()),
        harness::run(&ExperimentConfig::default_tiers()),
    ) {
        (Ok(first), Ok(second)) => {
            if first == second && first.digest == second.digest && first.digest.len() == 64 {
                SelfCheck::pass(
                    TRACK,
                    "harness.deterministic",
                    format!(
                        "同一配置两次 run 完全相同，digest={} 覆盖 {} 个档位",
                        au4a_core::short_id(&first.digest),
                        first.rows.len()
                    ),
                )
            } else {
                SelfCheck::fail(TRACK, "harness.deterministic", "两次 run 结果不一致")
            }
        }
        (Err(err), _) | (_, Err(err)) => {
            SelfCheck::fail(TRACK, "harness.deterministic", err.to_string())
        }
    });

    checks
}

/// 轨道产物摘要（只读投影的一部分）。
pub fn results_json() -> CoreResult<Value> {
    let mut kernel = Kernel::new(au4a_kernel::KernelConfig::default());
    let scenario_value = scenario(&mut kernel)?;
    let checks = self_check();
    Ok(json!({
        "track": TRACK,
        "title": TITLE,
        "range": RANGE,
        "current": CURRENT,
        "checks": checks.len(),
        "checks_passed": checks.iter().filter(|c| c.passed).count(),
        "scenario": scenario_value,
    }))
}

/// 端到端自有流程：用共享内核跑一遍本轨道的能力，返回 JSON 摘要。
///
/// 契约（不可改）：不 panic、不读文件、不开网络、不读墙钟；同样的输入给同样的输出。
/// 每一版迭代都应该让这里多做一件真实的事，而不是多打印一行字。
pub fn scenario(kernel: &mut Kernel) -> CoreResult<Value> {
    let config = ExperimentConfig::default_tiers();
    config.validate()?;
    let report = run_experiment(&config)?;
    let tiers = tier_report(&config.params, config.demand_milli)?;
    let verdict = adjudicate(&config.params, config.demand_milli, 1, 4 * config.params.n0)?;
    let bundle = collect(&[ScenarioSpec::new(
        "baseline",
        config.nodes.clone(),
        config.params,
        config.demand_milli,
    )])?;
    let fit_report = fit(&bundle, &FitSearch::around(config.params.n0, 500, 10, vec![1, 2, 3]))?;

    kernel.emit(
        &format!("{TRACK}.scenario"),
        format!(
            "{CURRENT} 档位评估 + 容量裁决 + 数据包 + 拟合：顶点 {}（{:?}），N0 估计 {}（真值 {}），最大残差 {} ppm",
            verdict.vertex_nodes,
            verdict.kind,
            fit_report.n0_estimate,
            config.params.n0,
            fit_report.max_residual_ppm
        ),
    );

    Ok(json!({
        "track": TRACK,
        "title": TITLE,
        "version": CURRENT,
        "config": report.config.to_json()?,
        "rows": report.rows,
        "digest": report.digest,
        "peak_nodes": report.peak().map(|row| row.nodes),
        "first_negative_net_nodes": report.first_negative_net().map(|row| row.nodes),
        "analytic_vertex": analytic_vertex_floor(config.params.n0, config.params.alpha),
        "tiers": tiers.to_json()?,
        "verdict": verdict.to_json()?,
        "bundle": {
            "records": bundle.len(),
            "digest": bundle.digest,
            "scenarios": 1,
        },
        "fit": fit_report.to_json()?,
        "verdict_kind": match verdict.kind {
            VerdictKind::VertexFound => "vertex_found",
            VerdictKind::MonotonicNoVertex => "monotonic_no_vertex",
            VerdictKind::InsufficientRange => "insufficient_range",
        },
        "evaluation_budget": evaluation_budget(),
        "note": "确定性聚合模型（解析式实现），非真实分布式压测",
    }))
}
