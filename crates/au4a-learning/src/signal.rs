//! v1.6.4 学习信号（Learning Signal）。
//!
//! spec 规定学习信号有四类：**任务完成质量 / 结算金额 / 信誉变化 / 违规记录**。
//! 这一版把它们折算成一个统一的整数向量，并明确每类信号的**来源**，避免「信号」变成含糊其辞：
//!
//! | 信号 | 来源 | 折算（万分比，10000 = 满格） |
//! |---|---|---|
//! | 任务完成质量 | `Outcome`（Success 10000 / Partial 5000 / Failure 0） | 直接取值 |
//! | 结算金额 | 经验里的 `reward`（微积分，来自内核账本结算路径） | `min(reward / reward_ref, 1) × 10000` |
//! | 信誉变化 | 本地信誉台账（v1.6.5；本版由调用方显式传入） | `clamp(delta / reputation_ref, -1, 1) × 10000` |
//! | 违规记录 | `ViolationLog` 的条数 | `min(count, 4) × 2500`，作为**扣分项** |
//!
//! 综合信号 = Σ(信号归一值 × 权重) / 10000，权重和为 10000，结果再钳制到 ±10000。
//! 全部是整数运算：没有浮点、没有「大约」、没有隐式饱和。

use au4a_core::{canonical_hash, CoreError, CoreResult, Credits, SelfCheck};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::experience::{Experience, ExperienceStore};
use crate::policy::Signals;
use crate::violation::ViolationLog;

/// 单条违规最多扣掉四分之一个「违规权重」：4 条违规即吃满该权重。
pub const VIOLATION_UNIT_BP: i64 = 2_500;

/// 信号权重（基点）。默认和为 10000。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalWeights {
    /// 任务完成质量权重。
    pub quality_bp: i64,
    /// 结算金额权重。
    pub settled_bp: i64,
    /// 信誉变化权重。
    pub reputation_bp: i64,
    /// 违规记录权重（作为扣分项）。
    pub violation_bp: i64,
    /// 结算金额归一化分母（微积分）。
    pub reward_ref: Credits,
    /// 信誉变化归一化分母（整数分）。
    pub reputation_ref: i64,
}

impl Default for SignalWeights {
    fn default() -> Self {
        Self {
            quality_bp: 5_000,
            settled_bp: 2_500,
            reputation_bp: 1_500,
            violation_bp: 1_000,
            reward_ref: Credits(100),
            reputation_ref: 100,
        }
    }
}

impl SignalWeights {
    pub fn sum_bp(&self) -> i64 {
        self.quality_bp
            .saturating_add(self.settled_bp)
            .saturating_add(self.reputation_bp)
            .saturating_add(self.violation_bp)
    }

    pub fn validate(&self) -> CoreResult<()> {
        if self.quality_bp < 0
            || self.settled_bp < 0
            || self.reputation_bp < 0
            || self.violation_bp < 0
        {
            return Err(CoreError::NegativeAmount);
        }
        if self.sum_bp() > 10_000 {
            // 权重和超过 10000 会让综合信号失去「±10000 满格」的含义，直接拒绝。
            return Err(CoreError::InvalidKind);
        }
        if self.reward_ref <= Credits::ZERO || self.reputation_ref <= 0 {
            return Err(CoreError::InvalidKind);
        }
        Ok(())
    }
}

/// 四类学习信号 + 综合信号。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearningSignal {
    /// 任务完成质量（万分比）。
    pub quality_bp: i64,
    /// 结算金额（微积分）。
    pub settled: Credits,
    /// 信誉变化（整数分）。
    pub reputation_delta: i64,
    /// 违规记录条数。
    pub violations: u32,
    /// 归一化后的四类分量（万分比，便于核对加权过程）。
    pub quality_norm_bp: i64,
    pub settled_norm_bp: i64,
    pub reputation_norm_bp: i64,
    pub violation_norm_bp: i64,
    /// 综合信号（万分比，[-10000, 10000]）。
    pub composite_bp: i64,
}

impl LearningSignal {
    /// 核心折算：四类原始值 → 归一值 → 加权综合。
    pub fn compute(
        quality_bp: i64,
        settled: Credits,
        reputation_delta: i64,
        violations: u32,
        weights: &SignalWeights,
    ) -> CoreResult<Self> {
        weights.validate()?;
        let quality_norm_bp = quality_bp.clamp(0, 10_000);
        let settled_norm_bp = if weights.reward_ref.get() > 0 {
            settled
                .get()
                .saturating_mul(10_000)
                .checked_div(weights.reward_ref.get())
                .unwrap_or(10_000)
                .clamp(0, 10_000)
        } else {
            0
        };
        let reputation_norm_bp = reputation_delta
            .saturating_mul(10_000)
            .checked_div(weights.reputation_ref)
            .unwrap_or(0)
            .clamp(-10_000, 10_000);
        let violation_norm_bp = (violations.min(4) as i64).saturating_mul(VIOLATION_UNIT_BP);

        let composite = weighted(quality_norm_bp, weights.quality_bp)?
            .saturating_add(weighted(settled_norm_bp, weights.settled_bp)?)
            .saturating_add(weighted(reputation_norm_bp, weights.reputation_bp)?)
            .saturating_sub(weighted(violation_norm_bp, weights.violation_bp)?)
            .clamp(-10_000, 10_000);

        Ok(Self {
            quality_bp,
            settled,
            reputation_delta,
            violations,
            quality_norm_bp,
            settled_norm_bp,
            reputation_norm_bp,
            violation_norm_bp,
            composite_bp: composite,
        })
    }

    /// 单条经验的信号：质量来自结局，结算金额来自该条经验，违规来自该条经验参与者的违规计数。
    pub fn from_experience(
        exp: &Experience,
        log: &ViolationLog,
        weights: &SignalWeights,
    ) -> CoreResult<Self> {
        let violations = exp
            .peer_agents
            .iter()
            .map(|p| log.count_of(p))
            .fold(0u32, |a, b| a.saturating_add(b));
        Self::compute(exp.quality_bp(), exp.reward, 0, violations, weights)
    }

    /// 整个经验库的聚合信号（学习循环里「这一轮」的信号）。
    ///
    /// `reputation_delta` 由调用方给出：v1.6.5 起来自本地信誉台账；在此之前如实传 0。
    pub fn from_store(
        store: &ExperienceStore,
        log: &ViolationLog,
        reputation_delta: i64,
        weights: &SignalWeights,
    ) -> CoreResult<Self> {
        if store.is_empty() {
            return Self::compute(0, Credits::ZERO, reputation_delta, log.total(), weights);
        }
        let mut quality_sum = 0i64;
        let mut settled = Credits::ZERO;
        for e in store.entries() {
            quality_sum = quality_sum.saturating_add(e.quality_bp());
            settled = settled.checked_add(e.reward)?;
        }
        let quality_bp = quality_sum / store.len() as i64;
        Self::compute(quality_bp, settled, reputation_delta, log.total(), weights)
    }

    /// 把信号接进行为调整的输入（`Signals`）。
    ///
    /// 质量/违规/信誉三类信号由本结构提供；与市场有关的**收益速率**与**报价历史**由调用方提供，
    /// 因为它们是流量（每 tick）而不是存量（总计），混在一起会让定价规则失去可比性。
    pub fn to_signals(
        &self,
        sample: usize,
        accept_rate_bp: Option<i64>,
        mean_reward: Credits,
        prev_mean_reward: Credits,
        prev_price_dir: i64,
    ) -> Signals {
        Signals {
            sample,
            confidence_bp: crate::feedback::confidence_of(sample),
            accept_rate_bp,
            success_bp: self.quality_bp,
            mean_reward,
            prev_mean_reward,
            prev_price_dir,
            violations: self.violations,
            reputation_delta: self.reputation_delta,
        }
    }

    pub fn to_value(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn digest(&self) -> CoreResult<String> {
        canonical_hash(&self.to_value()?)
    }

    /// 真实断言：折算精确、违规一定降低综合信号、权重非法被拒。
    pub fn self_check() -> Vec<SelfCheck> {
        let mut checks = Vec::new();
        let weights = SignalWeights::default();

        // 质量 5000bp、结算 50（参考 100）、信誉 +50（参考 100）、无违规
        // → 2500 + 1250 + 750 = 4500
        let base = LearningSignal::compute(5_000, Credits(50), 50, 0, &weights);
        let (ok, detail) = match &base {
            Ok(s) => (
                s.quality_norm_bp == 5_000
                    && s.settled_norm_bp == 5_000
                    && s.reputation_norm_bp == 5_000
                    && s.violation_norm_bp == 0
                    && s.composite_bp == 4_500,
                format!(
                    "norm=({},{},{},{}) composite={}bp",
                    s.quality_norm_bp,
                    s.settled_norm_bp,
                    s.reputation_norm_bp,
                    s.violation_norm_bp,
                    s.composite_bp
                ),
            ),
            Err(e) => (false, format!("计算失败: {e:?}")),
        };
        checks.push(crate::check("signal.exact_weighting", ok, detail));

        // 违规必须严格降低综合信号；一条违规 = 2500bp × 1000/10000 = 250bp
        let with_violation = LearningSignal::compute(5_000, Credits(50), 50, 1, &weights);
        let violation_ok = match (&base, &with_violation) {
            (Ok(b), Ok(v)) => {
                v.composite_bp == b.composite_bp - 250 && v.violation_norm_bp == 2_500
            }
            _ => false,
        };
        checks.push(crate::check(
            "signal.violation_penalises",
            violation_ok,
            "1 条违规 → 违规归一 2500bp、综合信号严格下降 250bp",
        ));

        // 结算金额封顶：500 微积分（参考 100）也只得 10000bp
        let capped = LearningSignal::compute(0, Credits(500), 0, 0, &weights)
            .map(|s| s.settled_norm_bp == 10_000)
            .unwrap_or(false);
        // 负信誉同样被钳制在 -10000bp
        let neg = LearningSignal::compute(0, Credits::ZERO, -1_000, 0, &weights)
            .map(|s| s.reputation_norm_bp == -10_000 && s.composite_bp == -1_500)
            .unwrap_or(false);
        checks.push(crate::check(
            "signal.clamped",
            capped && neg,
            format!(
                "结算归一封顶 10000bp：{capped}；负信誉钳制到 -10000bp 且综合为 -1500bp：{neg}"
            ),
        ));

        // 非法权重：负数 / 和超过 10000 / 分母为 0
        let rejected = SignalWeights {
            quality_bp: -1,
            ..SignalWeights::default()
        }
        .validate()
        .is_err()
            && SignalWeights {
                quality_bp: 9_000,
                ..SignalWeights::default()
            }
            .validate()
            .is_err()
            && SignalWeights {
                reward_ref: Credits::ZERO,
                ..SignalWeights::default()
            }
            .validate()
            .is_err();
        checks.push(crate::check(
            "signal.weights_validated",
            rejected,
            "负权重 / 权重和 >10000 / reward_ref=0 全部被拒绝",
        ));
        checks
    }
}

fn weighted(norm_bp: i64, weight_bp: i64) -> CoreResult<i64> {
    norm_bp
        .checked_mul(weight_bp)
        .map(|v| v / 10_000)
        .ok_or(CoreError::Overflow)
}

/// 模块级自检入口（`crate::self_check` 聚合它）。
pub fn self_check() -> Vec<SelfCheck> {
    LearningSignal::self_check()
}
