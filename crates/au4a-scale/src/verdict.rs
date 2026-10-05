//! v1.9.4 效果评估：容量顶点裁决。
//!
//! 裁决规则（先写清楚，再实现）：
//!
//! > 若群体吞吐在区间**内部**见顶，且峰值之后**编排开销占比上升**、**边际收益转负**，
//! > 则判定该顶点为**容量顶点**；若曲线在区间内单调（α=1）或峰值落在区间边界，则不作此判定。
//!
//! 三个原因码各自独立成立，裁决里逐条给出，因此使用者可以只采信其中一部分：
//!
//! * `AggregatePeaked` / `CompletionPeaked`：顶点两侧的吞吐/完成率都更低；
//! * `OverheadWorsened`：峰值后的开销占比严格更高；
//! * `MarginalTurnedNegative`：存在 `n ≥ 顶点` 使 `T(n+1) < T(n)`（「收益转负」）；
//! * `NetTurnedNegative`：存在 `n` 使 `G(n) < 0`（扣掉编排开销后净亏）。
//!
//! 全部整数运算、无墙钟、无 I/O：同一输入必然得到同一裁决。

use au4a_core::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cluster::{bounded_vertex_scan, MAX_SCAN_SAMPLES};
use crate::metrics::{
    aggregate_throughput_milli, completion_bp, marginal_gain_milli, net_throughput_milli,
    overhead_ratio_ppm, ScalingParams,
};

/// 裁决类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictKind {
    /// 区间内部存在容量顶点。
    VertexFound,
    /// 曲线在区间内单调（α=1 时没有内部极值）。
    MonotonicNoVertex,
    /// 区间太窄：峰值落在边界，无法判定「内部顶点」。
    InsufficientRange,
}

/// 结构化裁决理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictReason {
    AggregatePeaked,
    CompletionPeaked,
    OverheadWorsened,
    MarginalTurnedNegative,
    NetTurnedNegative,
    NoInteriorPeak,
    RangeTooNarrow,
}

impl VerdictKind {
    pub const ALL: [VerdictKind; 3] = [
        VerdictKind::VertexFound,
        VerdictKind::MonotonicNoVertex,
        VerdictKind::InsufficientRange,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            VerdictKind::VertexFound => "vertex_found",
            VerdictKind::MonotonicNoVertex => "monotonic_no_vertex",
            VerdictKind::InsufficientRange => "insufficient_range",
        }
    }
}

impl VerdictReason {
    pub const ALL: [VerdictReason; 7] = [
        VerdictReason::AggregatePeaked,
        VerdictReason::CompletionPeaked,
        VerdictReason::OverheadWorsened,
        VerdictReason::MarginalTurnedNegative,
        VerdictReason::NetTurnedNegative,
        VerdictReason::NoInteriorPeak,
        VerdictReason::RangeTooNarrow,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            VerdictReason::AggregatePeaked => "aggregate_peaked",
            VerdictReason::CompletionPeaked => "completion_peaked",
            VerdictReason::OverheadWorsened => "overhead_worsened",
            VerdictReason::MarginalTurnedNegative => "marginal_turned_negative",
            VerdictReason::NetTurnedNegative => "net_turned_negative",
            VerdictReason::NoInteriorPeak => "no_interior_peak",
            VerdictReason::RangeTooNarrow => "range_too_narrow",
        }
    }
}

/// 容量裁决结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapacityVerdict {
    pub kind: VerdictKind,
    pub lo: u64,
    pub hi: u64,
    /// 粗扫步长（顶点精度 = 该步长）。
    pub step: u64,
    pub vertex_nodes: u64,
    pub peak_aggregate_milli: i64,
    pub peak_completion_bp: i64,
    pub peak_net_milli: i64,
    /// 峰值处的开销占比（ppm：百万分之一；用于比较时保留分辨率）。
    pub overhead_ratio_at_peak_ppm: i64,
    /// 峰值之后（顶点 2 倍处，或区间右端）的开销占比（ppm）。
    pub overhead_ratio_after_peak_ppm: i64,
    /// 边际收益首次转负的节点数。
    pub marginal_negative_from: Option<u64>,
    /// 净收益首次转负的节点数。
    pub net_negative_from: Option<u64>,
    pub reasons: Vec<VerdictReason>,
    /// 本次裁决的求值次数（有界）。
    pub evaluations: u64,
}

impl CapacityVerdict {
    pub fn has(&self, reason: VerdictReason) -> bool {
        self.reasons.contains(&reason)
    }

    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }
}

/// 裁决：完成率峰值 + 峰值后编排开销恶化 + 边际收益转负 → 容量顶点。
pub fn adjudicate(
    params: &ScalingParams,
    demand_milli: i64,
    lo: u64,
    hi: u64,
) -> CoreResult<CapacityVerdict> {
    params.validate()?;
    if demand_milli <= 0 {
        return Err(CoreError::ZeroAmount);
    }
    if lo == 0 || hi <= lo {
        return Err(CoreError::InvalidKind);
    }

    let scan = bounded_vertex_scan(params, lo, hi, MAX_SCAN_SAMPLES)?;
    let vertex = scan.vertex_nodes;
    let mut reasons = Vec::new();

    let aggregate_at = |n: u64| aggregate_throughput_milli(n, params);
    let completion_at = |n: u64| completion_bp(n, params, demand_milli);

    let peak_aggregate_milli = aggregate_at(vertex)?;
    let peak_completion_bp = completion_at(vertex)?;
    let peak_net_milli = net_throughput_milli(vertex, params)?;
    let overhead_ratio_at_peak_ppm = overhead_ratio_ppm(vertex, params)?;

    // 峰值后参考点：顶点 2 倍处（若越界则取区间右端；若仍等于顶点则取 hi）。
    let after = vertex.saturating_mul(2).clamp(lo, hi).max(vertex);
    let overhead_ratio_after_peak_ppm = overhead_ratio_ppm(after, params)?;

    // 「收益转负」与「净收益转负」：从顶点向右有界扫描（与 N 无关的步数上界）。
    let span = hi.saturating_sub(vertex);
    let stride = (span / 2_048).max(1);
    let mut marginal_negative_from = None;
    let mut net_negative_from = None;
    let mut n = vertex;
    loop {
        if marginal_negative_from.is_none() && marginal_gain_milli(n, params)? < 0 {
            marginal_negative_from = Some(n);
        }
        if net_negative_from.is_none() && net_throughput_milli(n, params)? < 0 {
            net_negative_from = Some(n);
        }
        if n >= hi {
            break;
        }
        n = n.saturating_add(stride).min(hi);
    }

    let interior = vertex > lo + scan.step && vertex + scan.step < hi;
    let kind = if !params.has_interior_vertex() {
        reasons.push(VerdictReason::NoInteriorPeak);
        VerdictKind::MonotonicNoVertex
    } else if !interior {
        reasons.push(VerdictReason::RangeTooNarrow);
        VerdictKind::InsufficientRange
    } else {
        let before_aggregate = aggregate_at(vertex.saturating_sub(scan.step).max(lo))?;
        let after_aggregate = aggregate_at(vertex.saturating_add(scan.step).min(hi))?;
        if peak_aggregate_milli > before_aggregate && peak_aggregate_milli > after_aggregate {
            reasons.push(VerdictReason::AggregatePeaked);
        }
        let before_completion = completion_at(vertex.saturating_sub(scan.step).max(lo))?;
        let after_completion = completion_at(vertex.saturating_add(scan.step).min(hi))?;
        if peak_completion_bp >= before_completion && peak_completion_bp >= after_completion {
            reasons.push(VerdictReason::CompletionPeaked);
        }
        if overhead_ratio_after_peak_ppm > overhead_ratio_at_peak_ppm {
            reasons.push(VerdictReason::OverheadWorsened);
        }
        if marginal_negative_from.is_some() {
            reasons.push(VerdictReason::MarginalTurnedNegative);
        }
        if net_negative_from.is_some() {
            reasons.push(VerdictReason::NetTurnedNegative);
        }
        VerdictKind::VertexFound
    };

    Ok(CapacityVerdict {
        kind,
        lo,
        hi,
        step: scan.step,
        vertex_nodes: vertex,
        peak_aggregate_milli,
        peak_completion_bp,
        peak_net_milli,
        overhead_ratio_at_peak_ppm,
        overhead_ratio_after_peak_ppm,
        marginal_negative_from,
        net_negative_from,
        reasons,
        evaluations: scan.evaluations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::analytic_vertex_floor;

    #[test]
    fn a_peaked_model_is_adjudicated_as_a_capacity_vertex() {
        let params = ScalingParams::new(1_000, 2, 1_000_000, 1);
        let verdict = adjudicate(&params, 100_000, 1, 4_000).unwrap();
        assert_eq!(verdict.kind, VerdictKind::VertexFound);
        assert!(verdict.has(VerdictReason::AggregatePeaked));
        assert!(verdict.has(VerdictReason::CompletionPeaked));
        assert!(verdict.has(VerdictReason::OverheadWorsened));
        assert!(verdict.has(VerdictReason::MarginalTurnedNegative));
        // 与解析极值点（α=2 → n* = N0）相差不超过一个粗扫步长。
        let analytic = analytic_vertex_floor(params.n0, params.alpha);
        assert!(
            verdict.vertex_nodes.abs_diff(analytic) <= verdict.step,
            "顶点 {} 与解析 {analytic} 相差超过步长 {}",
            verdict.vertex_nodes,
            verdict.step
        );
        assert!(verdict.overhead_ratio_after_peak_ppm > verdict.overhead_ratio_at_peak_ppm);
        assert!(verdict.marginal_negative_from.unwrap() >= verdict.vertex_nodes);
        assert!(verdict.evaluations <= crate::cluster::evaluation_budget());
    }

    #[test]
    fn negative_returns_are_reported_exactly_when_they_happen() {
        // 每次交互成本 10 任务：小规模净赚，规模上去后编排开销吃掉全部产出（「收益转负」用例）。
        let expensive = ScalingParams::new(1_000, 2, 1_000_000, 10_000);
        let verdict = adjudicate(&expensive, 100_000, 1, 10_000).unwrap();
        assert!(
            verdict.has(VerdictReason::NetTurnedNegative),
            "reasons={:?}",
            verdict.reasons
        );
        let turned = verdict.net_negative_from.unwrap();
        assert!(net_throughput_milli(turned, &expensive).unwrap() < 0);
        // 曲线在起点是正的（「由正转负」而不是一开始就为负）。
        assert!(net_throughput_milli(1, &expensive).unwrap() > 0);
        assert!(turned > 1);

        // 编排成本低：净收益始终为正 → 不应出现该原因码。
        let cheap = ScalingParams::new(1_000, 2, 1_000_000, 0);
        let verdict = adjudicate(&cheap, 100_000, 1, 4_000).unwrap();
        assert!(!verdict.has(VerdictReason::NetTurnedNegative));
        assert_eq!(verdict.net_negative_from, None);
    }

    #[test]
    fn alpha_one_is_monotonic_and_gets_no_vertex() {
        let params = ScalingParams::new(1_000, 1, 1_000_000, 1);
        let verdict = adjudicate(&params, 100_000, 1, 4_000).unwrap();
        assert_eq!(verdict.kind, VerdictKind::MonotonicNoVertex);
        assert!(verdict.has(VerdictReason::NoInteriorPeak));
        assert!(!verdict.has(VerdictReason::AggregatePeaked));
    }

    #[test]
    fn a_range_that_does_not_contain_the_peak_is_insufficient() {
        let params = ScalingParams::new(10_000, 2, 1_000_000, 1);
        let verdict = adjudicate(&params, 100_000, 1, 20).unwrap();
        assert_eq!(verdict.kind, VerdictKind::InsufficientRange);
        assert!(verdict.has(VerdictReason::RangeTooNarrow));
    }

    #[test]
    fn adjudication_is_deterministic_and_serializable() {
        let params = ScalingParams::new(1_000, 2, 1_000_000, 1);
        let first = adjudicate(&params, 100_000, 1, 4_000).unwrap();
        let second = adjudicate(&params, 100_000, 1, 4_000).unwrap();
        assert_eq!(first, second);
        let json = first.to_json().unwrap();
        au4a_core::canonicalize(&json).unwrap();
        assert_eq!(json["kind"], serde_json::json!("vertex_found"));
        let reasons = json["reasons"].as_array().unwrap();
        assert!(reasons.iter().any(|r| r == &serde_json::json!("marginal_turned_negative")));
    }

    #[test]
    fn invalid_requests_are_refused() {
        let params = ScalingParams::new(1_000, 2, 1_000_000, 1);
        assert_eq!(adjudicate(&params, 0, 1, 100).err(), Some(CoreError::ZeroAmount));
        assert_eq!(adjudicate(&params, 100, 0, 100).err(), Some(CoreError::InvalidKind));
        assert_eq!(adjudicate(&params, 100, 100, 100).err(), Some(CoreError::InvalidKind));
        assert_eq!(adjudicate(&params, 100, 200, 100).err(), Some(CoreError::InvalidKind));
    }
}
