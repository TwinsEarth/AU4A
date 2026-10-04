//! v1.9.6 分析工具：从观测数据反推模型参数，并给出残差报告。
//!
//! 方法（整数网格搜索，不用浮点、不用迭代优化器）：
//!
//! 1. 在 `(N0, α)` 网格上枚举候选；
//! 2. 对每个候选，用**最小档位**的观测反解 `p0`：
//!    `p0 = T_obs(n_min) · (N0^α + n_min^α) / (n_min · N0^α)`；
//! 3. 用该 `(N0, α, p0)` 预测每个观测点的 `T(n)`，取**最大相对偏差**（ppm）作为损失；
//! 4. 取损失最小的候选；并列时取较小的 `N0`、再取较小的 `α`（确定性打破平局）。
//!
//! 这是一个诚实的拟合：它只说「哪个候选最不坏」，并把**残差**一并交出来。
//! 样本不足（< 2 条）或搜索空间为空时直接拒绝——不给没有依据的结论。

use au4a_core::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};

use crate::collect::DataBundle;
use crate::metrics::{aggregate_throughput_milli, coordination_base, ScalingParams};

/// 搜索空间。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FitSearch {
    pub n0_min: u64,
    pub n0_max: u64,
    pub n0_step: u64,
    pub alphas: Vec<u32>,
}

impl FitSearch {
    pub fn around(n0: u64, span: u64, step: u64, alphas: Vec<u32>) -> Self {
        Self {
            n0_min: n0.saturating_sub(span).max(1),
            n0_max: n0.saturating_add(span).max(1),
            n0_step: step.max(1),
            alphas,
        }
    }

    pub fn candidates(&self) -> CoreResult<u64> {
        if self.n0_min == 0
            || self.n0_max < self.n0_min
            || self.n0_step == 0
            || self.alphas.is_empty()
        {
            return Err(CoreError::InvalidKind);
        }
        let count = (self.n0_max - self.n0_min) / self.n0_step + 1;
        Ok(count.saturating_mul(self.alphas.len() as u64))
    }
}

/// 拟合报告。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FitReport {
    pub n0_estimate: u64,
    pub alpha_estimate: u32,
    pub p0_estimate_milli: i64,
    /// 最大相对偏差（ppm）。
    pub max_residual_ppm: i64,
    /// 平均相对偏差（ppm）。
    pub mean_residual_ppm: i64,
    pub samples: usize,
    pub evaluated_candidates: u64,
}

impl FitReport {
    pub fn params(&self) -> ScalingParams {
        ScalingParams::new(
            self.n0_estimate,
            self.alpha_estimate,
            self.p0_estimate_milli,
            0,
        )
    }

    pub fn to_json(&self) -> CoreResult<serde_json::Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }
}

/// 相对偏差（ppm）：`(预测 − 观测)·10^6 / 观测`。观测为 0 时返回 0（不制造无穷大）。
pub fn residual_ppm(observed: i64, predicted: i64) -> i64 {
    if observed == 0 {
        return 0;
    }
    let diff = predicted.saturating_sub(observed) as i128;
    ((diff * 1_000_000) / observed as i128)
        .clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

/// 给定参数下的逐点残差（ppm，顺序与 bundle 记录一致）。
pub fn residuals(bundle: &DataBundle, params: &ScalingParams) -> CoreResult<Vec<i64>> {
    params.validate()?;
    let mut out = Vec::with_capacity(bundle.len());
    for record in &bundle.records {
        let predicted = aggregate_throughput_milli(record.nodes, params)?;
        out.push(residual_ppm(record.aggregate_milli, predicted));
    }
    Ok(out)
}

fn candidate_p0(
    n0: u64,
    alpha: u32,
    nodes: u64,
    observed_aggregate: i64,
) -> CoreResult<i64> {
    if nodes == 0 || observed_aggregate <= 0 {
        return Err(CoreError::InvalidKind);
    }
    let base = coordination_base(n0, alpha);
    let n_alpha = coordination_base(nodes, alpha);
    // p0 = T_obs · (N0^α + n^α) / (n · N0^α)
    let numerator = (observed_aggregate as u128)
        .saturating_mul(base.saturating_add(n_alpha));
    let denominator = (nodes as u128).saturating_mul(base);
    if denominator == 0 {
        return Err(CoreError::Overflow);
    }
    let p0 = numerator / denominator;
    if p0 == 0 || p0 > i64::MAX as u128 {
        return Err(CoreError::Overflow);
    }
    Ok(p0 as i64)
}

/// 网格搜索拟合 `(N0, α, p0)`。
pub fn fit(bundle: &DataBundle, search: &FitSearch) -> CoreResult<FitReport> {
    if bundle.len() < 2 {
        // 一条记录可以拟合出无数个参数，等于没有依据。
        return Err(CoreError::InvalidKind);
    }
    let total = search.candidates()?;
    let anchor = bundle
        .records
        .iter()
        .min_by_key(|record| record.nodes)
        .ok_or(CoreError::InvalidKind)?;

    let mut best: Option<(i64, u64, u32, i64, Vec<i64>)> = None;
    let mut n0 = search.n0_min;
    while n0 <= search.n0_max {
        for alpha in &search.alphas {
            let params_probe = ScalingParams::new(n0, *alpha, 1, 0);
            if params_probe.validate().is_err() {
                continue;
            }
            let p0 = match candidate_p0(n0, *alpha, anchor.nodes, anchor.aggregate_milli) {
                Ok(value) => value,
                Err(_) => {
                    n0 += search.n0_step;
                    continue;
                }
            };
            let params = ScalingParams::new(n0, *alpha, p0, 0);
            let mut residual_list = Vec::with_capacity(bundle.len());
            let mut worst = 0i64;
            let mut ok = true;
            for record in &bundle.records {
                match aggregate_throughput_milli(record.nodes, &params) {
                    Ok(predicted) => {
                        let value = residual_ppm(record.aggregate_milli, predicted);
                        worst = worst.max(value.abs());
                        residual_list.push(value);
                    }
                    Err(_) => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                continue;
            }
            let better = match &best {
                None => true,
                Some((best_worst, best_n0, best_alpha, _, _)) => {
                    worst < *best_worst
                        || (worst == *best_worst
                            && (n0 < *best_n0 || (n0 == *best_n0 && *alpha < *best_alpha)))
                }
            };
            if better {
                best = Some((worst, n0, *alpha, p0, residual_list));
            }
        }
        n0 = n0.saturating_add(search.n0_step);
    }

    let (max_residual_ppm, n0_estimate, alpha_estimate, p0_estimate_milli, residual_list) =
        best.ok_or(CoreError::InvalidKind)?;
    let sum: i128 = residual_list.iter().map(|value| value.abs() as i128).sum();
    let mean = if residual_list.is_empty() {
        0
    } else {
        (sum / residual_list.len() as i128) as i64
    };
    Ok(FitReport {
        n0_estimate,
        alpha_estimate,
        p0_estimate_milli,
        max_residual_ppm,
        mean_residual_ppm: mean,
        samples: bundle.len(),
        evaluated_candidates: total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::{collect, sweep, ScenarioSpec};

    fn truth_bundle() -> DataBundle {
        let specs = vec![ScenarioSpec::new(
            "truth",
            vec![10, 100, 500, 1_000, 2_000, 5_000],
            ScalingParams::new(1_000, 2, 1_000_000, 0),
            100_000,
        )];
        collect(&specs).unwrap()
    }

    #[test]
    fn the_fit_recovers_the_ground_truth_parameters() {
        let bundle = truth_bundle();
        let search = FitSearch::around(1_000, 400, 10, vec![1, 2, 3]);
        let report = fit(&bundle, &search).unwrap();
        assert_eq!(report.alpha_estimate, 2, "α 必须精确命中真值");
        let delta = report.n0_estimate.abs_diff(1_000);
        assert!(delta <= 50, "N0 估计 {} 偏离真值过多", report.n0_estimate);
        assert!(
            report.max_residual_ppm <= 10_000,
            "最大残差 {} ppm 过大",
            report.max_residual_ppm
        );
        assert_eq!(report.samples, 6);
        assert!(report.evaluated_candidates > 0);
    }

    #[test]
    fn residuals_are_zero_at_the_truth_and_grow_for_wrong_parameters() {
        let bundle = truth_bundle();
        let truth = ScalingParams::new(1_000, 2, 1_000_000, 0);
        let at_truth = residuals(&bundle, &truth).unwrap();
        for value in &at_truth {
            assert!(value.abs() <= 1_000, "真值附近残差应接近 0，得到 {value} ppm");
        }
        let wrong = ScalingParams::new(2_000, 2, 1_000_000, 0);
        let at_wrong = residuals(&bundle, &wrong).unwrap();
        let worst_truth = at_truth.iter().map(|v| v.abs()).max().unwrap();
        let worst_wrong = at_wrong.iter().map(|v| v.abs()).max().unwrap();
        assert!(worst_wrong > worst_truth, "错误参数的残差必须更大（拟合有区分度）");
    }

    #[test]
    fn the_fit_is_deterministic_and_breaks_ties_deterministically() {
        let bundle = truth_bundle();
        let search = FitSearch::around(1_000, 100, 25, vec![2]);
        let first = fit(&bundle, &search).unwrap();
        let second = fit(&bundle, &search).unwrap();
        assert_eq!(first, second);
        let json = first.to_json().unwrap();
        au4a_core::canonicalize(&json).unwrap();
    }

    #[test]
    fn fitting_refuses_thin_or_invalid_input() {
        let single = collect(&[ScenarioSpec::new(
            "one",
            vec![100],
            ScalingParams::new(1_000, 2, 1_000_000, 0),
            100_000,
        )])
        .unwrap();
        let search = FitSearch::around(1_000, 100, 50, vec![2]);
        assert_eq!(fit(&single, &search).err(), Some(CoreError::InvalidKind));

        let bundle = sweep(&[10, 100], &[1_000], &[2], 1_000_000, 0, 100_000).unwrap();
        let empty_search = FitSearch {
            n0_min: 0,
            n0_max: 10,
            n0_step: 1,
            alphas: vec![2],
        };
        assert_eq!(fit(&bundle, &empty_search).err(), Some(CoreError::InvalidKind));
    }

    #[test]
    fn a_sweep_can_be_fitted_back_to_the_swept_parameter() {
        let bundle = sweep(&[10, 100, 1_000, 10_000], &[500], &[2], 1_000_000, 0, 100_000).unwrap();
        let search = FitSearch::around(500, 300, 10, vec![2]);
        let report = fit(&bundle, &search).unwrap();
        assert!(report.n0_estimate.abs_diff(500) <= 30, "N0={}", report.n0_estimate);
        assert_eq!(report.alpha_estimate, 2);
    }
}
