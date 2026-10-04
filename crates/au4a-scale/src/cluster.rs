//! v1.9.3 大规模集群：10 / 100 / 1k / 10k 节点的**有界**评估。
//!
//! 「大规模」在本轨道里的含义必须说清楚：**不是**起 10k 个线程或 10k 条连接，
//! 而是用解析式模型在**有界**的评估预算内给出 10k 节点的聚合指标：
//!
//! * **时间有界**：一次档位评估的求值次数 ≤ `2·MAX_SCAN_SAMPLES`（默认 4096×2），
//!   与节点数无关；10k 档位在本机是**毫秒级**（测试里有墙钟断言）。
//! * **内存有界**：函数只返回定长结构，没有长度随 N 增长的容器。
//! * **正确性有界**：有界窗口扫描的顶点与「全量逐点扫描」的顶点相差 ≤ 1 个粗扫步长
//!   （测试用全量扫描对照，不是自证）。
//!
//! 顶点搜索用两段式：粗扫（均匀采样）→ 局部细化（在粗扫最优点邻域逐点）。
//! 这是标准的先粗后细策略，代价固定、结果可复现。

use au4a_core::{canonical_hash, CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::metrics::{
    aggregate_throughput_milli, completion_bp, effective_per_node_milli, net_throughput_milli,
    overhead_ratio_bp, ScalingParams,
};

/// 本轨道的四个标准档位：10 / 100 / 1k / 10k。
pub const TIERS: [u64; 4] = [10, 100, 1_000, 10_000];
/// 粗扫样本数上界（与节点数无关）。
pub const MAX_SCAN_SAMPLES: u64 = 4_096;
/// 细化阶段的求值次数上界。
pub const MAX_REFINE_EVALUATIONS: u64 = 4_096;

/// 一次有界顶点扫描的结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VertexScan {
    pub lo: u64,
    pub hi: u64,
    /// 粗扫步长。
    pub step: u64,
    pub vertex_nodes: u64,
    pub peak_aggregate_milli: i64,
    /// 总求值次数（粗扫 + 细化）。
    pub evaluations: u64,
    /// 是否执行了细化阶段。
    pub refined: bool,
}

/// 一个档位的观测（与 `harness::Row` 同源，但只保留大规模评估需要的字段）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TierRow {
    pub nodes: u64,
    pub per_node_milli: i64,
    pub aggregate_milli: i64,
    pub net_milli: i64,
    pub overhead_ratio_bp: i64,
    pub completion_bp: i64,
}

/// 档位报告：4 行 + 一次有界顶点扫描。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TierReport {
    pub params: ScalingParams,
    pub demand_milli: i64,
    pub tiers: Vec<TierRow>,
    pub vertex: VertexScan,
    pub evaluations: u64,
    pub digest: String,
}

impl TierReport {
    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }
}

/// 有界窗口扫描顶点：粗扫 + 邻域细化。
///
/// `samples` 是粗扫采样点数（≤ [`MAX_SCAN_SAMPLES`]），因此总求值次数
/// ≤ `samples + 2·step + 1`，且 `step ≤ (hi−lo)/(samples−1)`。
pub fn bounded_vertex_scan(
    params: &ScalingParams,
    lo: u64,
    hi: u64,
    samples: u64,
) -> CoreResult<VertexScan> {
    params.validate()?;
    if lo == 0 || hi < lo {
        return Err(CoreError::InvalidKind);
    }
    if !(2..=MAX_SCAN_SAMPLES).contains(&samples) {
        return Err(CoreError::InvalidKind);
    }

    let span = hi.saturating_sub(lo);
    let step = if samples <= 1 {
        1
    } else {
        (span / (samples - 1)).max(1)
    };

    let mut evaluations = 0u64;
    let mut best_nodes = lo;
    let mut best_value = i64::MIN;
    let mut n = lo;
    loop {
        let value = aggregate_throughput_milli(n, params)?;
        evaluations += 1;
        if value > best_value {
            best_value = value;
            best_nodes = n;
        }
        if n >= hi {
            break;
        }
        n = n.saturating_add(step).min(hi);
    }

    // 邻域细化：在 [best-step, best+step] 内逐点，次数上界 MAX_REFINE_EVALUATIONS。
    let refine_lo = best_nodes.saturating_sub(step).max(lo);
    let refine_hi = best_nodes.saturating_add(step).min(hi);
    let mut refined = false;
    let mut m = refine_lo;
    let mut budget = MAX_REFINE_EVALUATIONS;
    while m <= refine_hi && budget > 0 {
        let value = aggregate_throughput_milli(m, params)?;
        evaluations += 1;
        budget -= 1;
        refined = true;
        if value > best_value {
            best_value = value;
            best_nodes = m;
        }
        m = m.saturating_add(1);
    }

    Ok(VertexScan {
        lo,
        hi,
        step,
        vertex_nodes: best_nodes,
        peak_aggregate_milli: best_value.max(0),
        evaluations,
        refined,
    })
}

/// 10 / 100 / 1k / 10k 档位的有界评估 + 一次顶点扫描。
pub fn tier_report(params: &ScalingParams, demand_milli: i64) -> CoreResult<TierReport> {
    params.validate()?;
    if demand_milli <= 0 {
        return Err(CoreError::ZeroAmount);
    }

    let mut tiers = Vec::with_capacity(TIERS.len());
    for nodes in TIERS {
        tiers.push(TierRow {
            nodes,
            per_node_milli: effective_per_node_milli(nodes, params)?,
            aggregate_milli: aggregate_throughput_milli(nodes, params)?,
            net_milli: net_throughput_milli(nodes, params)?,
            overhead_ratio_bp: overhead_ratio_bp(nodes, params)?,
            completion_bp: completion_bp(nodes, params, demand_milli)?,
        });
    }

    // 顶点搜索范围取 [1, 4·N0]：解析极值点必然落在这个区间内（α>1 时 n*<N0）。
    let hi = params.n0.saturating_mul(4).max(1);
    let vertex = bounded_vertex_scan(params, 1, hi, MAX_SCAN_SAMPLES)?;
    let evaluations = vertex.evaluations;

    let digest = canonical_hash(&serde_json::to_value(&tiers).map_err(|_| CoreError::Encoding)?)?;
    Ok(TierReport {
        params: *params,
        demand_milli,
        tiers,
        vertex,
        evaluations,
        digest,
    })
}

/// 单次档位评估的求值次数上界（用于文档与断言）。
pub fn evaluation_budget() -> u64 {
    MAX_SCAN_SAMPLES + MAX_REFINE_EVALUATIONS
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn params() -> ScalingParams {
        ScalingParams::new(1_000, 2, 1_000_000, 1)
    }

    #[test]
    fn tier_rows_come_from_the_metrics_module() {
        let report = tier_report(&params(), 100_000).unwrap();
        assert_eq!(report.tiers.len(), TIERS.len());
        assert_eq!(report.tiers[3].nodes, 10_000);
        for row in &report.tiers {
            assert_eq!(
                row.per_node_milli,
                effective_per_node_milli(row.nodes, &report.params).unwrap()
            );
            assert_eq!(
                row.aggregate_milli,
                aggregate_throughput_milli(row.nodes, &report.params).unwrap()
            );
            assert_eq!(
                row.net_milli,
                net_throughput_milli(row.nodes, &report.params).unwrap()
            );
            assert_eq!(
                row.overhead_ratio_bp,
                overhead_ratio_bp(row.nodes, &report.params).unwrap()
            );
        }
        assert_eq!(report.evaluations, report.vertex.evaluations);
        assert!(report.evaluations <= evaluation_budget());
    }

    #[test]
    fn the_bounded_scan_matches_a_full_scan_within_one_step() {
        for params in [
            ScalingParams::new(1_000, 2, 1_000_000, 1),
            ScalingParams::new(100, 3, 1_000_000, 1),
            ScalingParams::new(10_000, 2, 1_000_000, 1),
        ] {
            let hi = params.n0 * 4;
            let scan = bounded_vertex_scan(&params, 1, hi, MAX_SCAN_SAMPLES).unwrap();
            // 全量逐点扫描（测试里做对照，不用被测量的实现自证）。
            let mut best_n = 1u64;
            let mut best_v = i64::MIN;
            for n in 1..=hi {
                let v = aggregate_throughput_milli(n, &params).unwrap();
                if v > best_v {
                    best_v = v;
                    best_n = n;
                }
            }
            assert!(
                scan.vertex_nodes.abs_diff(best_n) <= scan.step,
                "N0={} 有界顶点 {} 与全量顶点 {best_n} 相差超过 step={}",
                params.n0,
                scan.vertex_nodes,
                scan.step
            );
            assert_eq!(scan.peak_aggregate_milli, best_v);
        }
    }

    #[test]
    fn the_evaluation_budget_does_not_grow_with_the_cluster_size() {
        let small = bounded_vertex_scan(&params(), 1, 4_000, MAX_SCAN_SAMPLES).unwrap();
        let huge = bounded_vertex_scan(&params(), 1, 4_000_000, MAX_SCAN_SAMPLES).unwrap();
        assert!(small.evaluations <= evaluation_budget());
        assert!(huge.evaluations <= evaluation_budget());
        assert!(huge.step > small.step, "范围越大粗扫步长越大（先粗后细）");
        // 求值次数 = 粗扫次数 + 细化次数，两者都由 samples 与步长决定上界，
        // 与节点数无关；这里核对**逐项上界**（含整除舍入的余量），而不是要求两次相等。
        for scan in [&small, &huge] {
            let span = scan.hi - scan.lo;
            let coarse_bound = span / scan.step + 2;
            let refine_bound = (2 * scan.step + 1).min(MAX_REFINE_EVALUATIONS);
            assert!(
                scan.evaluations <= coarse_bound + refine_bound,
                "求值 {} 次超出上界 {}",
                scan.evaluations,
                coarse_bound + refine_bound
            );
        }
    }

    #[test]
    fn ten_thousand_nodes_are_evaluated_in_well_under_a_second() {
        let start = Instant::now();
        let report = tier_report(&params(), 100_000).unwrap();
        let elapsed = start.elapsed();
        assert_eq!(report.tiers[3].nodes, 10_000);
        assert!(
            elapsed.as_millis() < 1_000,
            "10k 档位评估耗时 {}ms，超过 1s",
            elapsed.as_millis()
        );
    }

    #[test]
    fn reports_are_deterministic_and_content_addressed() {
        let first = tier_report(&params(), 100_000).unwrap();
        let second = tier_report(&params(), 100_000).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.digest, second.digest);
        let json = first.to_json().unwrap();
        au4a_core::canonicalize(&json).unwrap();
        assert_eq!(json["digest"], serde_json::json!(first.digest));
    }

    #[test]
    fn invalid_scan_requests_are_refused() {
        let p = params();
        assert_eq!(
            bounded_vertex_scan(&p, 0, 100, 16).err(),
            Some(CoreError::InvalidKind)
        );
        assert_eq!(
            bounded_vertex_scan(&p, 100, 10, 16).err(),
            Some(CoreError::InvalidKind)
        );
        assert_eq!(
            bounded_vertex_scan(&p, 1, 100, 1).err(),
            Some(CoreError::InvalidKind)
        );
        assert_eq!(
            bounded_vertex_scan(&p, 1, 100, MAX_SCAN_SAMPLES + 1).err(),
            Some(CoreError::InvalidKind)
        );
        assert_eq!(tier_report(&p, 0).err(), Some(CoreError::ZeroAmount));
    }
}
