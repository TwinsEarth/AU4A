//! v1.9.1 缩放定律度量：节点数 / 交互复杂度 / 算力预算 ↔ 群体智能水平。
//!
//! 本轨道度量的是**网络结构**对群体产出的影响，全部用整数定点数表达
//! （基元层的规范 JSON 禁止浮点，所以比率一律用基点 bp，吞吐一律用毫任务 milli）。
//!
//! # 模型（数学表达先写清楚，代码只是它的实现）
//!
//! 记节点数 `n`，模型参数 `(N0, α, p0, c)`：
//!
//! * **交互复杂度**（需要协调的对数）：`E(n) = n·(n−1)/2`
//! * **单节点有效吞吐**（协调开销吃掉一部分算力）：
//!   `p(n) = p0 · N0^α / (N0^α + n^α)`
//! * **群体吞吐**：`T(n) = n · p(n)`
//! * **编排开销**：`O(n) = c · E(n) / 1000`
//! * **净收益**：`G(n) = T(n) − O(n)`
//! * **完成率**：`C(n) = min(10000, 10000·T(n)/D)`（`D` 是需求，毫任务）
//! * **开销占比**：`R(n) = 10000·O(n)/T(n)`
//! * **边际收益**：`ΔT(n) = T(n+1) − T(n)`
//!
//! α = 1 时 `T(n)` 单调递增（只饱和，不衰退）；**α > 1 时存在内部极值点**：
//! `dT/dn = 0 ⟺ (α−1)·n^α = N0^α`，即 `n* = N0 / (α−1)^(1/α)`。
//! 极值点之后单节点效率的衰减快过节点数的增长，`T(n)` 转而下降——这就是
//! 「容量顶点」的数学来源，也是本轨道要裁决的东西（v1.9.4）。
//!
//! 单位约定（写进类型名的后缀，避免单位混用）：
//! * `_milli`：千分之一任务/epoch（`1000 milli = 1 任务`）
//! * `_bp`：万分之一（`10000 bp = 1.0`）
//!
//! # 整数取整的实际影响（必须知道，否则会误读曲线）
//!
//! `p(n)` 向下取整，所以当 `p0` 很小（例如 1000 milli）时，相邻 n 的 `T(n)`
//! 可能在峰后**短暂持平**：每节点少 1 milli 乘以 n 造成的差可以被取整吃掉。
//! 因此：判断峰值/拐点时用 `p0 ≥ 10^6 milli` 或对曲线做局部平均；
//! 度量本身的误差上界是 **每节点 1 milli**（`p(n)` 的一次截断）。

use au4a_core::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 模型参数。全部是整数：`N0` 是协调规模，`α` 是编排开销指数。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScalingParams {
    /// 协调规模：单节点效率衰减到一半时的节点数量级。
    pub n0: u64,
    /// 编排开销指数（1..=3）。只有 α > 1 才可能出现容量顶点。
    pub alpha: u32,
    /// 单节点理想吞吐（毫任务/epoch）。
    pub p0_milli: i64,
    /// 每次交互的编排成本（毫任务）。
    pub interaction_cost_milli: i64,
}

impl ScalingParams {
    pub const MIN_ALPHA: u32 = 1;
    pub const MAX_ALPHA: u32 = 3;

    pub fn new(n0: u64, alpha: u32, p0_milli: i64, interaction_cost_milli: i64) -> Self {
        Self {
            n0,
            alpha,
            p0_milli,
            interaction_cost_milli,
        }
    }

    /// 准入检查：非法参数会让度量给出无意义（甚至负数）的值，必须显式拒绝。
    pub fn validate(&self) -> CoreResult<()> {
        if self.n0 == 0 {
            return Err(CoreError::InvalidKind);
        }
        if !(Self::MIN_ALPHA..=Self::MAX_ALPHA).contains(&self.alpha) {
            return Err(CoreError::InvalidKind);
        }
        if self.p0_milli <= 0 {
            return Err(CoreError::NegativeAmount);
        }
        if self.interaction_cost_milli < 0 {
            return Err(CoreError::NegativeAmount);
        }
        Ok(())
    }

    /// 模型是否可能拥有内部容量顶点（数学结论：α > 1）。
    pub fn has_interior_vertex(&self) -> bool {
        self.alpha > 1
    }

    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn from_json(value: &Value) -> CoreResult<Self> {
        serde_json::from_value(value.clone()).map_err(|_| CoreError::Encoding)
    }
}

impl Default for ScalingParams {
    fn default() -> Self {
        // 默认参数取自「本地实验的基线场景」，不是任何外部系统的实测标定。
        Self::new(100, 2, 1_000, 1)
    }
}

fn pow_u128(base: u64, exp: u32) -> u128 {
    let mut acc: u128 = 1;
    for _ in 0..exp {
        acc = acc.saturating_mul(base as u128);
    }
    acc
}

fn clamp_i64(value: u128) -> CoreResult<i64> {
    if value > i64::MAX as u128 {
        Err(CoreError::Overflow)
    } else {
        Ok(value as i64)
    }
}

/// `N0^α`：模型的分母基准。
pub fn coordination_base(n0: u64, alpha: u32) -> u128 {
    pow_u128(n0, alpha)
}

/// 交互复杂度 `E(n) = n(n−1)/2`（需要协调的对数）。
pub fn interaction_complexity(n: u64) -> u128 {
    let n = n as u128;
    n.saturating_mul(n.saturating_sub(1)) / 2
}

/// `p(n) = p0 · N0^α / (N0^α + n^α)`（毫任务/节点/epoch，向下取整）。
pub fn effective_per_node_milli(n: u64, params: &ScalingParams) -> CoreResult<i64> {
    let base = coordination_base(params.n0, params.alpha);
    let n_alpha = pow_u128(n, params.alpha);
    let denominator = base.saturating_add(n_alpha);
    if denominator == 0 {
        return Err(CoreError::Overflow);
    }
    let numerator = (params.p0_milli.max(0) as u128).saturating_mul(base);
    clamp_i64(numerator / denominator)
}

/// `T(n) = n · p(n)`（毫任务/epoch）。
pub fn aggregate_throughput_milli(n: u64, params: &ScalingParams) -> CoreResult<i64> {
    let per_node = effective_per_node_milli(n, params)?.max(0) as u128;
    clamp_i64((n as u128).saturating_mul(per_node))
}

/// `O(n) = c · E(n) / 1000`（毫任务/epoch）。
pub fn orchestration_overhead_milli(n: u64, params: &ScalingParams) -> CoreResult<i64> {
    let cost = params.interaction_cost_milli.max(0) as u128;
    clamp_i64(interaction_complexity(n).saturating_mul(cost) / 1_000)
}

/// `G(n) = T(n) − O(n)`：可以为负——「收益转负」是本轨道要测的真实结论。
pub fn net_throughput_milli(n: u64, params: &ScalingParams) -> CoreResult<i64> {
    let aggregate = aggregate_throughput_milli(n, params)?;
    let overhead = orchestration_overhead_milli(n, params)?;
    aggregate.checked_sub(overhead).ok_or(CoreError::Overflow)
}

/// `C(n) = min(10000, 10000·T(n)/D)`（基点；`demand_milli > 0`）。
pub fn completion_bp(n: u64, params: &ScalingParams, demand_milli: i64) -> CoreResult<i64> {
    if demand_milli <= 0 {
        return Err(CoreError::ZeroAmount);
    }
    let aggregate = aggregate_throughput_milli(n, params)?.max(0) as u128;
    let rate = aggregate.saturating_mul(10_000) / (demand_milli as u128);
    Ok(if rate >= 10_000 { 10_000 } else { rate as i64 })
}

/// `R(n) = 10000·O(n)/T(n)`（基点）。`T(n) == 0` 时定义为 10000（全部是开销）。
pub fn overhead_ratio_bp(n: u64, params: &ScalingParams) -> CoreResult<i64> {
    let aggregate = aggregate_throughput_milli(n, params)?.max(0) as u128;
    let overhead = orchestration_overhead_milli(n, params)?.max(0) as u128;
    if aggregate == 0 {
        return Ok(10_000);
    }
    clamp_i64(overhead.saturating_mul(10_000) / aggregate)
}

/// `R_ppm(n) = 1000000·O(n)/T(n)`（百万分之一）。
///
/// **为什么需要它**：当吞吐远大于开销时，基点分辨率会把占比截断成 0
/// （例如 `O/T = 1e-5` → 0 bp），于是「峰值后开销恶化」这类**比较**会失效。
/// 因此：**报告**用 bp（人类可读），**比较**用 ppm（保留分辨率）。
pub fn overhead_ratio_ppm(n: u64, params: &ScalingParams) -> CoreResult<i64> {
    let aggregate = aggregate_throughput_milli(n, params)?.max(0) as u128;
    let overhead = orchestration_overhead_milli(n, params)?.max(0) as u128;
    if aggregate == 0 {
        return Ok(1_000_000);
    }
    clamp_i64(overhead.saturating_mul(1_000_000) / aggregate)
}

/// `ΔT(n) = T(n+1) − T(n)`：正数表示还在变大，负数表示已经过了顶点。
pub fn marginal_gain_milli(n: u64, params: &ScalingParams) -> CoreResult<i64> {
    let here = aggregate_throughput_milli(n, params)?;
    let next = aggregate_throughput_milli(n.saturating_add(1), params)?;
    next.checked_sub(here).ok_or(CoreError::Overflow)
}

/// 解析极值点（向下取整）：最大的 `n` 满足 `(α−1)·n^α ≤ N0^α`。
///
/// `α == 1` 时不存在内部极值，返回 0（调用方应据 `has_interior_vertex` 判断）。
pub fn analytic_vertex_floor(n0: u64, alpha: u32) -> u64 {
    if alpha <= 1 || n0 == 0 {
        return 0;
    }
    let base = coordination_base(n0, alpha);
    let factor = (alpha - 1) as u128;
    // 在 [1, n0] 内二分：α > 1 时极值点必然落在 N0 之内。
    let mut low = 1u64;
    let mut high = n0;
    let mut best = 0u64;
    while low <= high {
        let mid = low + (high - low) / 2;
        if factor.saturating_mul(pow_u128(mid, alpha)) <= base {
            best = mid;
            low = mid.saturating_add(1);
        } else {
            high = mid.saturating_sub(1);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> ScalingParams {
        ScalingParams::new(100, 2, 1_000, 1)
    }

    #[test]
    fn per_node_and_aggregate_match_hand_computed_values() {
        let p = params();
        // p(100) = 1000·10000/(10000+10000) = 500；T(100) = 100·500 = 50000
        assert_eq!(effective_per_node_milli(100, &p).unwrap(), 500);
        assert_eq!(aggregate_throughput_milli(100, &p).unwrap(), 50_000);
        // p(10) = 1000·10000/(10000+100) = 990（向下取整）；T(10) = 9900
        assert_eq!(effective_per_node_milli(10, &p).unwrap(), 990);
        assert_eq!(aggregate_throughput_milli(10, &p).unwrap(), 9_900);
        // p(N0) 恰好是理想吞吐的一半：这是 N0 的定义。
        assert_eq!(effective_per_node_milli(p.n0, &p).unwrap(), p.p0_milli / 2);
    }

    #[test]
    fn metrics_agree_with_an_independent_u128_computation() {
        // 「数学表达 + 代码实现 + 数值测试三者一致」：测试里换一种写法重算一遍。
        let p = params();
        for n in [1u64, 7, 50, 100, 250, 1_000] {
            let base = (p.n0 as u128).pow(p.alpha);
            let expected_per_node = (p.p0_milli as u128 * base) / (base + (n as u128).pow(p.alpha));
            let expected_aggregate = n as u128 * expected_per_node;
            assert_eq!(
                effective_per_node_milli(n, &p).unwrap() as u128,
                expected_per_node,
                "n={n}"
            );
            assert_eq!(
                aggregate_throughput_milli(n, &p).unwrap() as u128,
                expected_aggregate,
                "n={n}"
            );
        }
    }

    #[test]
    fn alpha_two_has_a_peak_while_alpha_one_does_not() {
        let peaked = ScalingParams::new(100, 2, 1_000, 1);
        assert!(peaked.has_interior_vertex());
        assert!(
            aggregate_throughput_milli(100, &peaked).unwrap()
                > aggregate_throughput_milli(1_000, &peaked).unwrap()
        );

        // α=1：严格单调（用较大 p0 让向下取整的影响可忽略，见模块文档的取整说明）。
        let flat = ScalingParams::new(100, 1, 1_000_000, 1);
        assert!(!flat.has_interior_vertex());
        let mut previous = 0;
        for n in [10u64, 100, 1_000, 10_000] {
            let current = aggregate_throughput_milli(n, &flat).unwrap();
            assert!(current > previous, "α=1 必须单调递增：n={n}");
            previous = current;
        }
    }

    #[test]
    fn the_analytic_vertex_agrees_with_the_discrete_argmax_within_one_step() {
        // α=2 时解析极值点就是 N0（(α−1)^(1/α) = 1）。
        let p = ScalingParams::new(1_000, 2, 1_000_000, 1);
        assert_eq!(analytic_vertex_floor(p.n0, p.alpha), 1_000);
        let discrete = discrete_argmax(&p, p.n0 * 2);
        assert!(
            discrete.abs_diff(1_000) <= 1,
            "离散顶点 {discrete} 与解析顶点 1000 相差超过 1"
        );

        // α=3 时解析极值点 N0/2^(1/3) ≈ 793.7 → 向下取整 793。
        let cubed = ScalingParams::new(1_000, 3, 1_000_000, 1);
        let analytic = analytic_vertex_floor(cubed.n0, cubed.alpha);
        assert_eq!(analytic, 793);
        let discrete = discrete_argmax(&cubed, cubed.n0 * 2);
        assert!(
            discrete.abs_diff(analytic) <= 1,
            "离散顶点 {discrete} 与解析顶点 {analytic} 相差超过 1"
        );
    }

    fn discrete_argmax(params: &ScalingParams, limit: u64) -> u64 {
        let mut best_n = 1u64;
        let mut best_t = i64::MIN;
        for n in 1..=limit {
            let t = aggregate_throughput_milli(n, params).unwrap_or(i64::MIN);
            if t > best_t {
                best_t = t;
                best_n = n;
            }
        }
        best_n
    }

    #[test]
    fn overhead_grows_quadratically_and_can_turn_the_net_negative() {
        let p = ScalingParams::new(10_000, 2, 1_000, 1);
        // E(2n)/E(n) 在 n 较大时接近 4（小 n 上取整误差大，所以用 1000/2000 比较）。
        let small = orchestration_overhead_milli(1_000, &p).unwrap();
        let doubled = orchestration_overhead_milli(2_000, &p).unwrap();
        let ratio_bp = doubled * 10_000 / small;
        // 比值用基点表达：4.0 倍 == 40000 bp（注意单位是 bp，不是「倍」）。
        assert!((38_000..=42_000).contains(&ratio_bp), "ratio_bp={ratio_bp}");

        // 编排成本足够高时净收益转负——这是容量裁决要用的信号。
        let expensive = ScalingParams::new(1_000, 2, 1_000, 1_000);
        let net = net_throughput_milli(10_000, &expensive).unwrap();
        assert!(net < 0, "净收益应为负：{net}");
        assert!(overhead_ratio_bp(10_000, &expensive).unwrap() > 10_000);
    }

    #[test]
    fn completion_saturates_and_requires_a_positive_demand() {
        let p = params();
        assert_eq!(completion_bp(10, &p, 9_900).unwrap(), 10_000);
        assert_eq!(completion_bp(10, &p, 19_800).unwrap(), 5_000);
        assert_eq!(completion_bp(10, &p, 0), Err(CoreError::ZeroAmount));
    }

    #[test]
    fn marginal_gain_is_positive_before_and_negative_after_the_vertex() {
        // p0 取大一些，让「每节点取整」的相对误差可忽略（见模块文档的取整说明）。
        let p = ScalingParams::new(1_000, 2, 1_000_000, 1);
        assert!(marginal_gain_milli(100, &p).unwrap() > 0);
        assert!(marginal_gain_milli(5_000, &p).unwrap() < 0);
    }

    #[test]
    fn parameters_are_validated_and_serialize_without_floats() {
        assert_eq!(
            ScalingParams::new(0, 2, 1_000, 1).validate(),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(
            ScalingParams::new(10, 0, 1_000, 1).validate(),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(
            ScalingParams::new(10, 4, 1_000, 1).validate(),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(
            ScalingParams::new(10, 2, 0, 1).validate(),
            Err(CoreError::NegativeAmount)
        );
        let json = params().to_json().unwrap();
        au4a_core::canonicalize(&json).unwrap();
        assert_eq!(ScalingParams::from_json(&json).unwrap(), params());
        assert_eq!(analytic_vertex_floor(0, 2), 0);
        assert_eq!(analytic_vertex_floor(100, 1), 0);
    }
}
