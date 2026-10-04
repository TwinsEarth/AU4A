//! v1.4.2 定价策略：Agent **自主**定价。
//!
//! 定价是 Agent 对自己的能力报出的价格，输入只有三样（全部来自它自己的状态）：
//!
//! * **信誉分** `reputation_bp`：越高越便宜（不可转让的信誉要能换到市场份额）；
//! * **能力稀缺度** `scarcity_bp`：越稀缺越贵（稀缺技能有溢价）；
//! * **当前负载** `load_bp`：越忙越贵（用价格做拥塞控制，而不是靠人类调度）。
//!
//! 为什么全是整数基点：浮点会破坏可复现性——同一份输入在不同机器上可能给出不同价格，
//! 于是「Agent 的定价可被验证」就成了空话。这里的每一步都是 `i64` 整数运算：
//!
//! ```text
//! multiplier_bp = clamp(10000 + 负载溢价 + 稀缺溢价 - 信誉折扣, min_bp, max_bp)
//! unit_price    = max(base_price × multiplier_bp / 10000, 1)
//! ```
//!
//! [`quote`] 是**纯函数**：同样的输入永远给出同样的报价单（有测试断言具体数值）。
//! 单调性也被测试钉住：负载 ↑ 价格 ↑、信誉 ↑ 价格 ↓、稀缺度 ↑ 价格 ↑。
//! 价格下限为 1 微积分，因此报价永远不会塌缩成 0（0 额结算在账本层是非法调用）。

use au4a_core::{CoreError, CoreResult, Credits};
use serde::{Deserialize, Serialize};

/// 定价输入：完全来自 Agent 自己的状态，人类不参与。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceInputs {
    /// 基准单价（微积分）。
    pub base_price: Credits,
    /// 信誉分（基点，0..=10_000）。
    pub reputation_bp: i64,
    /// 能力稀缺度（基点，0..=10_000）。
    pub scarcity_bp: i64,
    /// 当前负载（基点，0..=10_000）。
    pub load_bp: i64,
}

impl PriceInputs {
    /// 全部为 0 的状态：基准价、无信誉折扣、无稀缺溢价、空闲。
    pub fn idle(base_price: Credits) -> Self {
        Self {
            base_price,
            reputation_bp: 0,
            scarcity_bp: 0,
            load_bp: 0,
        }
    }
}

/// 定价系数：一次定死，跨版本可调但同一版本内固定，保证可复现。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceKnobs {
    /// 信誉满分时最多降多少（基点）。
    pub reputation_discount_max_bp: i64,
    /// 稀缺度满分时最多加多少（基点）。
    pub scarcity_premium_max_bp: i64,
    /// 满载时最多加多少（基点）。
    pub load_premium_max_bp: i64,
    /// 总乘数下限（基点）。
    pub min_multiplier_bp: i64,
    /// 总乘数上限（基点）。
    pub max_multiplier_bp: i64,
}

impl PriceKnobs {
    /// 默认系数：信誉最多降 20%，稀缺最多加 30%，负载最多加 50%，总乘数夹在 [50%, 200%]。
    pub const DEFAULT: PriceKnobs = PriceKnobs {
        reputation_discount_max_bp: 2_000,
        scarcity_premium_max_bp: 3_000,
        load_premium_max_bp: 5_000,
        min_multiplier_bp: 5_000,
        max_multiplier_bp: 20_000,
    };

    pub fn validate(&self) -> CoreResult<()> {
        validate_ratio_bp(self.reputation_discount_max_bp)?;
        validate_ratio_bp(self.scarcity_premium_max_bp)?;
        validate_ratio_bp(self.load_premium_max_bp)?;
        validate_multiplier_bp(self.min_multiplier_bp)?;
        validate_multiplier_bp(self.max_multiplier_bp)?;
        if self.min_multiplier_bp > self.max_multiplier_bp {
            return Err(CoreError::InvalidKind);
        }
        Ok(())
    }
}

impl Default for PriceKnobs {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 比例型基点必须是 `0..=10_000`（`CoreError` 冻结，无 `OutOfRange` 变体）。
fn validate_ratio_bp(bp: i64) -> CoreResult<()> {
    if bp < 0 {
        return Err(CoreError::NegativeAmount);
    }
    if bp > 10_000 {
        return Err(CoreError::InvalidKind);
    }
    Ok(())
}

/// 乘数型基点允许超过 100%（最高 1000%），但不得为负。
fn validate_multiplier_bp(bp: i64) -> CoreResult<()> {
    if bp < 0 {
        return Err(CoreError::NegativeAmount);
    }
    if bp > 100_000 {
        return Err(CoreError::InvalidKind);
    }
    Ok(())
}

/// 报价单的三个可审计分量（全部是基点，可被观察层逐项复核）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceComponents {
    pub reputation_discount_bp: i64,
    pub scarcity_premium_bp: i64,
    pub load_premium_bp: i64,
}

/// 一份报价单：输入、分量、总乘数、成交单价。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceQuote {
    pub inputs: PriceInputs,
    pub components: PriceComponents,
    pub multiplier_bp: i64,
    pub unit_price: Credits,
}

impl PriceQuote {
    /// `units` 个单位的成交额（整数乘法，溢出即拒绝）。
    pub fn total_for(&self, units: u64) -> CoreResult<Credits> {
        let units = i64::try_from(units).map_err(|_| CoreError::Overflow)?;
        if units == 0 {
            return Err(CoreError::ZeroAmount);
        }
        self.unit_price
            .get()
            .checked_mul(units)
            .map(Credits)
            .ok_or(CoreError::Overflow)
    }

    /// 报价单的内容寻址（报价可被引用、可被信封承载）。
    pub fn fingerprint(&self) -> CoreResult<String> {
        let value = serde_json::to_value(self).map_err(|_| CoreError::Encoding)?;
        au4a_core::canonical_hash(&value)
    }
}

fn clamp(value: i64, lo: i64, hi: i64) -> i64 {
    if value < lo {
        lo
    } else if value > hi {
        hi
    } else {
        value
    }
}

/// 纯函数定价：同输入 → 同输出。
pub fn quote(inputs: &PriceInputs, knobs: &PriceKnobs) -> CoreResult<PriceQuote> {
    knobs.validate()?;
    if inputs.base_price == Credits::ZERO {
        return Err(CoreError::ZeroAmount);
    }
    validate_ratio_bp(inputs.reputation_bp)?;
    validate_ratio_bp(inputs.scarcity_bp)?;
    validate_ratio_bp(inputs.load_bp)?;

    let components = PriceComponents {
        reputation_discount_bp: Credits(inputs.reputation_bp)
            .scaled_bp(knobs.reputation_discount_max_bp)?
            .get(),
        scarcity_premium_bp: Credits(inputs.scarcity_bp)
            .scaled_bp(knobs.scarcity_premium_max_bp)?
            .get(),
        load_premium_bp: Credits(inputs.load_bp)
            .scaled_bp(knobs.load_premium_max_bp)?
            .get(),
    };
    let raw_bp = 10_000i64
        .checked_add(components.load_premium_bp)
        .and_then(|v| v.checked_add(components.scarcity_premium_bp))
        .and_then(|v| v.checked_sub(components.reputation_discount_bp))
        .ok_or(CoreError::Overflow)?;
    let multiplier_bp = clamp(raw_bp, knobs.min_multiplier_bp, knobs.max_multiplier_bp);
    let computed = inputs.base_price.scaled_bp(multiplier_bp)?;
    // 价格下限 1：不允许报价塌缩为 0，否则结算层会把它当成非法调用。
    let unit_price = if computed == Credits::ZERO {
        Credits(1)
    } else {
        computed
    };
    Ok(PriceQuote {
        inputs: *inputs,
        components,
        multiplier_bp,
        unit_price,
    })
}

/// 只要单价的便捷入口。
pub fn unit_price(inputs: &PriceInputs, knobs: &PriceKnobs) -> CoreResult<Credits> {
    Ok(quote(inputs, knobs)?.unit_price)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(base: i64, rep: i64, scarce: i64, load: i64) -> PriceInputs {
        PriceInputs {
            base_price: Credits(base),
            reputation_bp: rep,
            scarcity_bp: scarce,
            load_bp: load,
        }
    }

    #[test]
    fn concrete_numbers_are_pinned() {
        let q = quote(&inputs(1_000_000, 8_000, 2_000, 4_000), &PriceKnobs::DEFAULT).unwrap();
        // 折扣 = 8000×2000/10000 = 1600；稀缺 = 2000×3000/10000 = 600；负载 = 4000×5000/10000 = 2000
        assert_eq!(q.components.reputation_discount_bp, 1_600);
        assert_eq!(q.components.scarcity_premium_bp, 600);
        assert_eq!(q.components.load_premium_bp, 2_000);
        // 乘数 = 10000 + 2000 + 600 - 1600 = 11000 → 单价 = 1000000 × 11000 / 10000 = 1100000
        assert_eq!(q.multiplier_bp, 11_000);
        assert_eq!(q.unit_price, Credits(1_100_000));
    }

    #[test]
    fn the_same_input_always_gives_the_same_quote() {
        let i = inputs(777_000, 3_333, 6_666, 1_234);
        let a = quote(&i, &PriceKnobs::DEFAULT).unwrap();
        let b = quote(&i, &PriceKnobs::DEFAULT).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.fingerprint().unwrap(), b.fingerprint().unwrap());
    }

    #[test]
    fn the_quote_is_integer_only_and_canonicalizable() {
        let q = quote(&inputs(999_999, 1_111, 2_222, 3_333), &PriceKnobs::DEFAULT).unwrap();
        let value = serde_json::to_value(q).unwrap();
        // 规范 JSON 拒绝浮点：能编码成功就证明报价里没有浮点。
        let canonical = au4a_core::canonicalize(&value).unwrap();
        assert!(canonical.contains("\"unit_price\":"));
        assert!(!canonical.contains('.'));
    }

    #[test]
    fn load_up_price_up_never_down() {
        let knobs = PriceKnobs::DEFAULT;
        let mut prev = quote(&inputs(1_000_000, 0, 0, 0), &knobs).unwrap().unit_price;
        for load in 1..=10_000i64 {
            let price = quote(&inputs(1_000_000, 0, 0, load), &knobs).unwrap().unit_price;
            assert!(price >= prev, "load={load} 时价格下降：{prev} → {price}");
            prev = price;
        }
        // 网格上严格递增（每 500bp 负载至少让价格上升一次）
        let mut grid = Vec::new();
        for load in (0..=10_000i64).step_by(500) {
            grid.push(quote(&inputs(1_000_000, 0, 0, load), &knobs).unwrap().unit_price);
        }
        for w in grid.windows(2) {
            assert!(w[0] < w[1], "负载网格上未严格递增：{:?}", w);
        }
    }

    #[test]
    fn reputation_up_price_down_never_up() {
        let knobs = PriceKnobs::DEFAULT;
        let mut prev = quote(&inputs(1_000_000, 0, 0, 0), &knobs).unwrap().unit_price;
        for rep in 1..=10_000i64 {
            let price = quote(&inputs(1_000_000, rep, 0, 0), &knobs).unwrap().unit_price;
            assert!(price <= prev, "reputation={rep} 时价格上升：{prev} → {price}");
            prev = price;
        }
        let mut grid = Vec::new();
        for rep in (0..=10_000i64).step_by(500) {
            grid.push(quote(&inputs(1_000_000, rep, 0, 0), &knobs).unwrap().unit_price);
        }
        for w in grid.windows(2) {
            assert!(w[0] > w[1], "信誉网格上未严格递减：{:?}", w);
        }
    }

    #[test]
    fn scarcity_up_price_up() {
        let knobs = PriceKnobs::DEFAULT;
        let mut prev = quote(&inputs(1_000_000, 0, 0, 0), &knobs).unwrap().unit_price;
        for scarce in (500..=10_000i64).step_by(500) {
            let price = quote(&inputs(1_000_000, 0, scarce, 0), &knobs).unwrap().unit_price;
            assert!(price > prev, "稀缺度 {scarce} 未抬价：{prev} → {price}");
            prev = price;
        }
    }

    #[test]
    fn clamps_bound_the_multiplier_on_both_ends() {
        let knobs = PriceKnobs {
            min_multiplier_bp: 9_500,
            max_multiplier_bp: 12_000,
            ..PriceKnobs::DEFAULT
        };
        let cheap = quote(&inputs(1_000_000, 10_000, 0, 0), &knobs).unwrap();
        assert_eq!(cheap.multiplier_bp, 9_500);
        let dear = quote(&inputs(1_000_000, 0, 10_000, 10_000), &knobs).unwrap();
        assert_eq!(dear.multiplier_bp, 12_000);
    }

    #[test]
    fn a_price_never_collapses_to_zero() {
        let q = quote(&inputs(1, 10_000, 0, 0), &PriceKnobs::DEFAULT).unwrap();
        assert_eq!(q.multiplier_bp, 8_000);
        assert_eq!(q.unit_price, Credits(1));
    }

    #[test]
    fn degenerate_inputs_are_refused() {
        let knobs = PriceKnobs::DEFAULT;
        assert_eq!(
            quote(&inputs(0, 0, 0, 0), &knobs),
            Err(CoreError::ZeroAmount)
        );
        assert_eq!(
            quote(&inputs(100, 10_001, 0, 0), &knobs),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(
            quote(&inputs(100, 0, -1, 0), &knobs),
            Err(CoreError::NegativeAmount)
        );
        let bad = PriceKnobs {
            min_multiplier_bp: 9_000,
            max_multiplier_bp: 8_000,
            ..PriceKnobs::DEFAULT
        };
        assert_eq!(quote(&inputs(100, 0, 0, 0), &bad), Err(CoreError::InvalidKind));
    }

    #[test]
    fn multi_unit_total_is_integer_multiplication() {
        let q = quote(&inputs(1_000_000, 8_000, 2_000, 4_000), &PriceKnobs::DEFAULT).unwrap();
        assert_eq!(q.total_for(3).unwrap(), Credits(3_300_000));
        assert_eq!(q.total_for(0), Err(CoreError::ZeroAmount));
    }
}
