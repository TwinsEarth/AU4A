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

pub mod harness;
pub mod metrics;

use au4a_core::{CoreResult, SelfCheck};
use au4a_kernel::Kernel;
use serde_json::{json, Value};

pub use harness::{run as run_experiment, ExperimentConfig, ExperimentReport, Row};
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
pub const CURRENT: &str = "v1.9.2";
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

    kernel.emit(
        &format!("{TRACK}.scenario"),
        format!(
            "{CURRENT} 实验框架：{} 个档位，digest={}",
            report.rows.len(),
            au4a_core::short_id(&report.digest)
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
        "note": "确定性聚合模型（解析式实现），非真实分布式压测",
    }))
}
