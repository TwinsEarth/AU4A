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

/// 微积分金额。永远非负，运算检查溢出。
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
    pub fn slash(&mut self, who: &Did, amount: Credits) -> CoreResult<()> {
        if amount.0 == 0 {
            return Err(CoreError::ZeroAmount);
        }
        let acct = self.balance(who);
        let take = if acct.locked < amount {
            acct.locked
        } else {
            amount
        };
        self.account_mut(who).locked = acct.locked.checked_sub(take)?;
        self.slashed = self.slashed.checked_add(take)?;
        self.check_conservation()
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
    pub fn check_conservation(&self) -> CoreResult<()> {
        let expected = self.minted.checked_sub(self.slashed)?;
        if self.total()? != expected {
            return Err(CoreError::Overflow);
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
}
