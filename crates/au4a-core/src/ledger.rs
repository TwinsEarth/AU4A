//! 整数守恒账本。
//!
//! 参考项目把账本做成浮点、且市场状态不可持久化——于是「守恒」只能靠文档承诺。
//! AU4A 把守恒做成可以在**任意时刻**断言的不变式：
//!
//! ```text
//! Σ(available) + Σ(locked) + slashed == minted
//! ```
//!
//! 所有金额是 `i64` 微积分（micro-credit），没有浮点，没有四舍五入，没有“差不多相等”。
//! `slashed` 是罚没销毁（不是转给谁），所以它出现在等式左边而不是被移项。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::did::Did;
use crate::error::{CoreError, CoreResult};

/// 微积分金额。**金额语义上非负**，运算检查溢出。
///
/// v2.3.0 澄清（原注释写「永远非负」，与代码不符）：
/// * [`Credits::new`] 拒绝负数；负数余额在账本里不可能出现；
/// * 但 [`Credits::checked_sub`] 是**带溢出检查的减法**，用于增量/差值时会合法地产生负值
///   （规模轨道的 `step_gain`、学习轨道的 `revenue_lift` 都依赖这个语义）；
/// * **金额路径**（转账/锁定/解押/罚没）请用 [`Credits::checked_sub_nonneg`]，它拒绝负结果。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Credits(pub i64);

impl Credits {
    pub const ZERO: Credits = Credits(0);

    pub fn new(v: i64) -> CoreResult<Self> {
        if v < 0 {
            Err(CoreError::NegativeAmount)
        } else {
            Ok(Credits(v))
        }
    }

    pub fn get(self) -> i64 {
        self.0
    }

    pub fn checked_add(self, other: Credits) -> CoreResult<Credits> {
        self.0
            .checked_add(other.0)
            .map(Credits)
            .ok_or(CoreError::Overflow)
    }

    pub fn checked_sub(self, other: Credits) -> CoreResult<Credits> {
        self.0
            .checked_sub(other.0)
            .map(Credits)
            .ok_or(CoreError::Overflow)
    }

    /// v2.3.0 新增：**金额语义的减法**——结果不得为负，否则 [`CoreError::NegativeAmount`]。
    ///
    /// 为什么不是把 `checked_sub` 改成这样：全仓 30 处调用里有若干**依赖负差值**
    /// （规模轨道的 `step_gain`、学习轨道的 `revenue_lift`、度量 diff），改语义会静默改变它们的行为。
    /// 因此新增一个显式 API 给金额路径用，旧的留给"差值/增量"。
    pub fn checked_sub_nonneg(self, other: Credits) -> CoreResult<Credits> {
        let v = self.0.checked_sub(other.0).ok_or(CoreError::Overflow)?;
        if v < 0 {
            return Err(CoreError::NegativeAmount);
        }
        Ok(Credits(v))
    }

    /// 按万分比计价：`rate_bp` 是基点（1 bp = 0.01%），向下取整，整数运算。
    pub fn scaled_bp(self, rate_bp: i64) -> CoreResult<Credits> {
        if rate_bp < 0 {
            return Err(CoreError::NegativeAmount);
        }
        self.0
            .checked_mul(rate_bp)
            .map(|v| Credits(v / 10_000))
            .ok_or(CoreError::Overflow)
    }
}

impl std::fmt::Display for Credits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 账户：可用余额 + 已锁定质押。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub available: Credits,
    pub locked: Credits,
}

impl Account {
    pub fn total(&self) -> CoreResult<Credits> {
        self.available.checked_add(self.locked)
    }
}

/// 只读账本投影——人类观察层「看收益」用的就是它。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerView {
    pub accounts: BTreeMap<String, Account>,
    pub minted: Credits,
    pub slashed: Credits,
    pub total: Credits,
}

/// 账本本体。
#[derive(Clone, Debug, Default)]
pub struct Ledger {
    accounts: BTreeMap<Did, Account>,
    minted: Credits,
    slashed: Credits,
}

impl Ledger {
    pub fn new() -> Self {
        Self::default()
    }

    fn account_mut(&mut self, who: &Did) -> &mut Account {
        self.accounts.entry(who.clone()).or_default()
    }

    /// 发行（创世或 Agent 自主增发）。人类只能提供资源，发行由网络规则决定。
    pub fn mint(&mut self, to: &Did, amount: Credits) -> CoreResult<()> {
        if amount.0 == 0 {
            return Err(CoreError::ZeroAmount);
        }
        let cur = self.account_mut(to).available;
        let next = cur.checked_add(amount)?;
        self.account_mut(to).available = next;
        self.minted = self.minted.checked_add(amount)?;
        Ok(())
    }

    /// Agent 之间的支付。人类不在此路径上。
    pub fn transfer(&mut self, from: &Did, to: &Did, amount: Credits) -> CoreResult<()> {
        if amount.0 == 0 {
            return Err(CoreError::ZeroAmount);
        }
        let from_balance = self.balance(from).available;
        if from_balance < amount {
            return Err(CoreError::InsufficientFunds);
        }
        let next_from = from_balance.checked_sub(amount)?;
        self.account_mut(from).available = next_from;
        let to_cur = self.account_mut(to).available;
        let next_to = to_cur.checked_add(amount)?;
        self.account_mut(to).available = next_to;
        self.check_conservation()
    }

    /// 锁定质押：可用 → 锁定，总量不变。
    pub fn lock(&mut self, who: &Did, amount: Credits) -> CoreResult<()> {
        if amount.0 == 0 {
            return Err(CoreError::ZeroAmount);
        }
        let acct = self.balance(who);
        if acct.available < amount {
            return Err(CoreError::InsufficientFunds);
        }
        self.account_mut(who).available = acct.available.checked_sub(amount)?;
        self.account_mut(who).locked = acct.locked.checked_add(amount)?;
        self.check_conservation()
    }

    /// 解除质押：锁定 → 可用。
    pub fn unlock(&mut self, who: &Did, amount: Credits) -> CoreResult<()> {
        if amount.0 == 0 {
            return Err(CoreError::ZeroAmount);
        }
        let acct = self.balance(who);
        if acct.locked < amount {
            return Err(CoreError::InsufficientFunds);
        }
        self.account_mut(who).locked = acct.locked.checked_sub(amount)?;
        self.account_mut(who).available = acct.available.checked_add(amount)?;
        self.check_conservation()
    }

    /// 罚没：从锁定余额销毁。销毁量进入 `slashed`，等式仍然成立。
    ///
    /// v2.3.0 修复（P1）：返回**实际销毁量**。此前返回 `Ok(())` 并静默钳制超出部分，
    /// 于是"要求罚没 999、实际只有 20"对调用方不可见——政策层会以为已罚 999，账实分叉。
    /// 钳制本身是刻意的（保留），但现在把真实数量交回调用方，可自行断言或记账。
    pub fn slash(&mut self, who: &Did, amount: Credits) -> CoreResult<Credits> {
        if amount.0 == 0 {
            return Err(CoreError::ZeroAmount);
        }
        let acct = self.balance(who);
        let take = if acct.locked < amount {
            acct.locked
        } else {
            amount
        };
        self.account_mut(who).locked = acct.locked.checked_sub_nonneg(take)?;
        self.slashed = self.slashed.checked_add(take)?;
        self.check_conservation()?;
        Ok(take)
    }

    pub fn balance(&self, who: &Did) -> Account {
        self.accounts.get(who).cloned().unwrap_or_default()
    }

    pub fn minted(&self) -> Credits {
        self.minted
    }

    pub fn slashed(&self) -> Credits {
        self.slashed
    }

    /// Σ(available) + Σ(locked)。
    pub fn total(&self) -> CoreResult<Credits> {
        let mut sum = Credits::ZERO;
        for acct in self.accounts.values() {
            sum = sum.checked_add(acct.total()?)?;
        }
        Ok(sum)
    }

    /// 守恒断言：`total + slashed == minted`。
    ///
    /// v2.3.0 修复（P1）：不成立时返回 [`CoreError::ConservationViolated`]，
    /// 不再误报为 `Overflow`——后者会把日志与告警引向"算术溢出"这个错误方向。
    pub fn check_conservation(&self) -> CoreResult<()> {
        let expected = self.minted.checked_sub(self.slashed)?;
        if self.total()? != expected {
            return Err(CoreError::ConservationViolated);
        }
        Ok(())
    }

    /// 人类观察层读的投影。只读、可序列化、无写路径。
    pub fn view(&self) -> LedgerView {
        LedgerView {
            accounts: self
                .accounts
                .iter()
                .map(|(k, v)| (k.as_str().to_string(), v.clone()))
                .collect(),
            minted: self.minted,
            slashed: self.slashed,
            total: self.total().unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::did::AgentKeys;

    fn dids(n: u8) -> Vec<Did> {
        (0..n)
            .map(|i| AgentKeys::from_seed(&[i; 32]).did())
            .collect()
    }

    #[test]
    fn conservation_holds_across_every_operation() {
        let d = dids(3);
        let mut l = Ledger::new();
        l.mint(&d[0], Credits(1_000)).unwrap();
        l.mint(&d[1], Credits(500)).unwrap();
        l.transfer(&d[0], &d[2], Credits(250)).unwrap();
        l.lock(&d[1], Credits(100)).unwrap();
        l.unlock(&d[1], Credits(40)).unwrap();
        l.slash(&d[1], Credits(30)).unwrap();
        l.check_conservation().unwrap();
        assert_eq!(l.total().unwrap(), Credits(1_470));
        assert_eq!(l.minted(), Credits(1_500));
        assert_eq!(l.slashed(), Credits(30));
    }

    #[test]
    fn overspend_is_refused_not_clamped() {
        let d = dids(2);
        let mut l = Ledger::new();
        l.mint(&d[0], Credits(10)).unwrap();
        assert_eq!(
            l.transfer(&d[0], &d[1], Credits(11)),
            Err(CoreError::InsufficientFunds)
        );
        assert_eq!(l.balance(&d[0]).available, Credits(10));
        l.check_conservation().unwrap();
    }

    #[test]
    fn slash_is_bounded_by_locked_balance() {
        let d = dids(1);
        let mut l = Ledger::new();
        l.mint(&d[0], Credits(100)).unwrap();
        l.lock(&d[0], Credits(20)).unwrap();
        l.slash(&d[0], Credits(999)).unwrap();
        assert_eq!(l.slashed(), Credits(20));
        assert_eq!(l.balance(&d[0]).locked, Credits::ZERO);
        assert_eq!(l.balance(&d[0]).available, Credits(80));
        l.check_conservation().unwrap();
    }

    #[test]
    fn zero_amount_calls_are_refused() {
        let d = dids(1);
        let mut l = Ledger::new();
        assert_eq!(l.mint(&d[0], Credits::ZERO), Err(CoreError::ZeroAmount));
    }

    #[test]
    fn basis_point_pricing_is_integer_math() {
        assert_eq!(Credits(1_000).scaled_bp(250).unwrap(), Credits(25));
        assert_eq!(Credits(999).scaled_bp(1).unwrap(), Credits(0));
        assert_eq!(Credits(1_000).scaled_bp(-1), Err(CoreError::NegativeAmount));
    }

    // ── v2.3.0 回归测试 ──────────────────────────────────────────────────────

    #[test]
    fn slash_reports_the_actual_amount_destroyed() {
        // P1 回归：修复前 `slash` 返回 `Ok(())` 并静默钳制，调用方看不到"只罚到 20"。
        let d = dids(1);
        let mut l = Ledger::new();
        l.mint(&d[0], Credits(100)).unwrap();
        l.lock(&d[0], Credits(20)).unwrap();
        let destroyed = l.slash(&d[0], Credits(999)).unwrap();
        assert_eq!(
            destroyed,
            Credits(20),
            "必须如实返回实际销毁量，而不是静默吞掉"
        );
        assert_eq!(l.slashed(), Credits(20));
        // 首次全罚光后，再罚只能得到 0（返回 0 而不是报错，语义明确）
        assert_eq!(l.slash(&d[0], Credits(5)).unwrap(), Credits::ZERO);
        assert_eq!(l.slashed(), Credits(20));
        l.check_conservation().unwrap();
    }

    #[test]
    fn checked_sub_is_signed_for_deltas_but_nonneg_for_money() {
        // v2.3.0：两种减法各有明确语义，不再靠注释含糊。
        assert_eq!(Credits(10).checked_sub(Credits(20)).unwrap(), Credits(-10)); // 差值可为负
        assert_eq!(
            Credits(10).checked_sub_nonneg(Credits(20)),
            Err(CoreError::NegativeAmount)
        );
        assert_eq!(
            Credits(20).checked_sub_nonneg(Credits(20)).unwrap(),
            Credits::ZERO
        );
        assert_eq!(
            Credits(30).checked_sub_nonneg(Credits(20)).unwrap(),
            Credits(10)
        );
        // 溢出仍然报 Overflow，与符号无关
        assert_eq!(
            Credits(i64::MIN).checked_sub_nonneg(Credits::ZERO),
            Err(CoreError::NegativeAmount)
        );
    }

    #[test]
    fn broken_conservation_reports_its_own_error_not_overflow() {
        // P1 回归：修复前守恒被破坏会报 `Overflow`，把日志/告警引向"算术溢出"这个错误方向。
        let d = dids(1);
        let mut l = Ledger::new();
        l.mint(&d[0], Credits(100)).unwrap();
        l.check_conservation().unwrap();
        // 直接破坏内部状态（同一模块的测试可以访问私有字段），制造真实的守恒破坏
        l.minted = Credits(999);
        assert_eq!(l.check_conservation(), Err(CoreError::ConservationViolated));
        assert_ne!(
            l.check_conservation(),
            Err(CoreError::Overflow),
            "守恒破坏不得再报成 Overflow"
        );
        assert_eq!(
            CoreError::ConservationViolated.to_string(),
            "conservation violated"
        );
        // 修回后恢复一致
        l.minted = Credits(100);
        l.check_conservation().unwrap();
    }
}
