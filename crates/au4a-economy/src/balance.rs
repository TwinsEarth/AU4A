//! v1.4.1 余额管理：Agent **自主**申报并执行自己的余额策略。
//!
//! 为什么把策略放在 Agent 手里：参考项目里账户由运营方开、额度由运营方批，于是 Agent 的
//! 支付能力实际上取决于人类。AU4A 反过来——每个 Agent 自己申报
//! 「运营底线 / 目标质押比例 / 单笔支出上限」，自己判定、自己执行；人类没有任何设定入口。
//!
//! 三条不变式，每一步账务之后都会断言：
//!
//! 1. 任何支出都不得穿过 `reserve_floor`（运营底线）；
//! 2. 任何支出都不得超过单笔上限 `max_spend_bp`（基点数，整数运算）；
//! 3. `Σ可用 + Σ锁定 + 罚没 == 发行`，由 [`Ledger::check_conservation`] 在每个写路径末尾强制。
//!
//! 判定与执行分离：[`check_spend`] 是纯函数（不改账本、可被观察层与审计复用），
//! [`spend`] 才是写路径，且写路径**先判定后动账**——拒绝时账本一个字节都不变。

use std::collections::BTreeMap;

use au4a_core::{Account, CoreError, CoreResult, Credits, Did, Ledger};
use serde::{Deserialize, Serialize};

/// 单个 Agent 自己申报的余额策略。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalancePolicy {
    /// 运营底线：可用余额低于它时只能收款，不能支出。
    pub reserve_floor: Credits,
    /// 目标质押比例（基点，占总资产的万分比）。
    pub stake_target_bp: i64,
    /// 单笔支出上限（基点，占当前可用余额的万分比）。
    pub max_spend_bp: i64,
}

impl BalancePolicy {
    /// 保守默认：留 50 微积分底线，20% 质押，单笔不超过可用余额一半。
    pub const DEFAULT: BalancePolicy = BalancePolicy {
        reserve_floor: Credits(50),
        stake_target_bp: 2_000,
        max_spend_bp: 5_000,
    };

    pub fn validate(&self) -> CoreResult<()> {
        validate_bp(self.stake_target_bp)?;
        validate_bp(self.max_spend_bp)
    }
}

impl Default for BalancePolicy {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 基点必须是比例：`0..=10_000`。
///
/// `CoreError` 是冻结枚举（没有 `OutOfRange` 变体），因此映射固定为：
/// 负数 → [`CoreError::NegativeAmount`]，越界 → [`CoreError::InvalidKind`]。
/// 这里不新增错误类型、不 panic、不静默截断——越界的策略必须在申报时就被拒绝。
fn validate_bp(bp: i64) -> CoreResult<()> {
    if bp < 0 {
        return Err(CoreError::NegativeAmount);
    }
    if bp > 10_000 {
        return Err(CoreError::InvalidKind);
    }
    Ok(())
}

/// 一次支出请求的判定结果（纯判定，不改账本）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpendVerdict {
    /// 允许支出。
    Allow,
    /// 金额为零：这是调用错误，不是经济决策。
    ZeroAmount,
    /// 会穿过运营底线。
    BelowReserve,
    /// 超过单笔上限。
    OverSingleCap,
}

impl SpendVerdict {
    pub fn allowed(self) -> bool {
        matches!(self, SpendVerdict::Allow)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SpendVerdict::Allow => "allow",
            SpendVerdict::ZeroAmount => "zero_amount",
            SpendVerdict::BelowReserve => "below_reserve",
            SpendVerdict::OverSingleCap => "over_single_cap",
        }
    }
}

/// 可用额度：`可用余额 - 运营底线`，不足则 0（不借、不透支）。
pub fn spendable(ledger: &Ledger, who: &Did, policy: &BalancePolicy) -> CoreResult<Credits> {
    policy.validate()?;
    let available = ledger.balance(who).available;
    if available > policy.reserve_floor {
        available.checked_sub(policy.reserve_floor)
    } else {
        Ok(Credits::ZERO)
    }
}

/// 目标锁定量：`总资产 × stake_target_bp`（向下取整）。
pub fn stake_target(ledger: &Ledger, who: &Did, policy: &BalancePolicy) -> CoreResult<Credits> {
    policy.validate()?;
    ledger.balance(who).total()?.scaled_bp(policy.stake_target_bp)
}

/// 距离质押目标还差多少（已达标返回 0）。
pub fn stake_deficit(ledger: &Ledger, who: &Did, policy: &BalancePolicy) -> CoreResult<Credits> {
    let target = stake_target(ledger, who, policy)?;
    let locked = ledger.balance(who).locked;
    if target > locked {
        target.checked_sub(locked)
    } else {
        Ok(Credits::ZERO)
    }
}

/// 支出判定表（纯函数）。顺序即优先级：零额 → 底线 → 单笔上限。
pub fn check_spend(
    ledger: &Ledger,
    who: &Did,
    amount: Credits,
    policy: &BalancePolicy,
) -> CoreResult<SpendVerdict> {
    policy.validate()?;
    if amount == Credits::ZERO {
        return Ok(SpendVerdict::ZeroAmount);
    }
    let available = ledger.balance(who).available;
    if available < policy.reserve_floor.checked_add(amount)? {
        return Ok(SpendVerdict::BelowReserve);
    }
    let single_cap = available.scaled_bp(policy.max_spend_bp)?;
    if amount > single_cap {
        return Ok(SpendVerdict::OverSingleCap);
    }
    Ok(SpendVerdict::Allow)
}

/// 支出：先判定，再动账本，最后断言守恒。
///
/// 拒绝时账本不发生任何变化（判定在 `transfer` 之前完成）。
pub fn spend(
    ledger: &mut Ledger,
    from: &Did,
    to: &Did,
    amount: Credits,
    policy: &BalancePolicy,
) -> CoreResult<Credits> {
    match check_spend(ledger, from, amount, policy)? {
        SpendVerdict::Allow => {}
        SpendVerdict::ZeroAmount => return Err(CoreError::ZeroAmount),
        SpendVerdict::BelowReserve | SpendVerdict::OverSingleCap => {
            return Err(CoreError::InsufficientFunds)
        }
    }
    ledger.transfer(from, to, amount)?;
    ledger.check_conservation()?;
    spendable(ledger, from, policy)
}

/// 自主补足质押：锁定量取 `min(质押缺口, 可用额度, 预算)`，不穿过运营底线。
///
/// 返回实际锁定金额；无缺口或无可用额度时返回 [`CoreError::ZeroAmount`]（不是静默成功）。
pub fn autostake(
    ledger: &mut Ledger,
    who: &Did,
    policy: &BalancePolicy,
    budget: Credits,
) -> CoreResult<Credits> {
    let deficit = stake_deficit(ledger, who, policy)?;
    let free = spendable(ledger, who, policy)?;
    let mut amount = if deficit < free { deficit } else { free };
    if budget < amount {
        amount = budget;
    }
    if amount == Credits::ZERO {
        return Err(CoreError::ZeroAmount);
    }
    ledger.lock(who, amount)?;
    ledger.check_conservation()?;
    Ok(amount)
}

/// 守恒闸门：每个写路径末尾调用，观察层也用它做只读断言。
pub fn assert_conserved(ledger: &Ledger) -> CoreResult<()> {
    ledger.check_conservation()
}

/// 余额变动（有符号微积分，供场景摘要与收益面板使用）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountDelta {
    pub available: i64,
    pub locked: i64,
    pub total: i64,
}

/// 计算两个账户快照之间的有符号差值。`saturating_sub` 保证极端数值下也不 panic。
pub fn delta(before: &Account, after: &Account) -> AccountDelta {
    AccountDelta {
        available: after.available.get().saturating_sub(before.available.get()),
        locked: after.locked.get().saturating_sub(before.locked.get()),
        total: after
            .total()
            .unwrap_or_default()
            .get()
            .saturating_sub(before.total().unwrap_or_default().get()),
    }
}

/// 单个 Agent 的余额报告（只读投影，收益面板复用）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalanceReport {
    pub did: String,
    pub available: Credits,
    pub locked: Credits,
    pub spendable: Credits,
    pub stake_deficit: Credits,
    pub reserve_floor: Credits,
    pub conserved: bool,
}

/// 策略簿：每个 Agent 申报自己的策略，之后只按自己的策略动账。
///
/// 这个结构里**没有**任何「替别人设定策略」的方法——这是刻意的：
/// 人类与运营方都不在写路径上。
#[derive(Clone, Debug, Default)]
pub struct BalanceManager {
    policies: BTreeMap<Did, BalancePolicy>,
}

impl BalanceManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Agent 自己申报策略。未申报就动账会被 [`CoreError::UnknownAgent`] 拒绝。
    pub fn declare(&mut self, who: &Did, policy: BalancePolicy) -> CoreResult<()> {
        policy.validate()?;
        self.policies.insert(who.clone(), policy);
        Ok(())
    }

    pub fn policy(&self, who: &Did) -> CoreResult<BalancePolicy> {
        self.policies.get(who).copied().ok_or(CoreError::UnknownAgent)
    }

    pub fn declared(&self) -> usize {
        self.policies.len()
    }

    pub fn check(&self, ledger: &Ledger, who: &Did, amount: Credits) -> CoreResult<SpendVerdict> {
        check_spend(ledger, who, amount, &self.policy(who)?)
    }

    pub fn spend(
        &self,
        ledger: &mut Ledger,
        from: &Did,
        to: &Did,
        amount: Credits,
    ) -> CoreResult<Credits> {
        spend(ledger, from, to, amount, &self.policy(from)?)
    }

    pub fn autostake(&self, ledger: &mut Ledger, who: &Did, budget: Credits) -> CoreResult<Credits> {
        autostake(ledger, who, &self.policy(who)?, budget)
    }

    pub fn report(&self, ledger: &Ledger, who: &Did) -> CoreResult<BalanceReport> {
        let policy = self.policy(who)?;
        let acct = ledger.balance(who);
        Ok(BalanceReport {
            did: who.as_str().to_string(),
            available: acct.available,
            locked: acct.locked,
            spendable: spendable(ledger, who, &policy)?,
            stake_deficit: stake_deficit(ledger, who, &policy)?,
            reserve_floor: policy.reserve_floor,
            conserved: ledger.check_conservation().is_ok(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use au4a_core::AgentKeys;

    fn did(seed: u8) -> Did {
        AgentKeys::from_seed(&[seed; 32]).did()
    }

    fn policy(reserve: i64, stake_bp: i64, spend_bp: i64) -> BalancePolicy {
        BalancePolicy {
            reserve_floor: Credits(reserve),
            stake_target_bp: stake_bp,
            max_spend_bp: spend_bp,
        }
    }

    #[test]
    fn the_agent_declares_its_own_policy() {
        let a = did(1);
        let mut book = BalanceManager::new();
        assert_eq!(book.policy(&a), Err(CoreError::UnknownAgent));
        book.declare(&a, BalancePolicy::DEFAULT).unwrap();
        assert_eq!(book.policy(&a).unwrap(), BalancePolicy::DEFAULT);
        assert_eq!(book.declared(), 1);
    }

    #[test]
    fn an_out_of_range_policy_is_refused_at_declaration() {
        let a = did(2);
        let mut book = BalanceManager::new();
        assert_eq!(
            book.declare(&a, policy(0, 10_001, 5_000)),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(
            book.declare(&a, policy(0, 0, -1)),
            Err(CoreError::NegativeAmount)
        );
        assert_eq!(book.declared(), 0);
    }

    #[test]
    fn an_allowed_spend_moves_exactly_the_amount() {
        let (a, b) = (did(3), did(4));
        let mut l = Ledger::new();
        l.mint(&a, Credits(1_000)).unwrap();
        let p = policy(100, 2_000, 5_000);
        assert_eq!(check_spend(&l, &a, Credits(300), &p).unwrap(), SpendVerdict::Allow);
        spend(&mut l, &a, &b, Credits(300), &p).unwrap();
        assert_eq!(l.balance(&a).available, Credits(700));
        assert_eq!(l.balance(&b).available, Credits(300));
        assert_conserved(&l).unwrap();
    }

    #[test]
    fn a_spend_that_would_break_the_reserve_floor_is_refused_and_changes_nothing() {
        let (a, b) = (did(5), did(6));
        let mut l = Ledger::new();
        l.mint(&a, Credits(400)).unwrap();
        let p = policy(100, 2_000, 10_000);
        // 400 - 301 = 99 < 100：穿过底线。
        assert_eq!(
            check_spend(&l, &a, Credits(301), &p).unwrap(),
            SpendVerdict::BelowReserve
        );
        assert_eq!(spend(&mut l, &a, &b, Credits(301), &p), Err(CoreError::InsufficientFunds));
        assert_eq!(l.balance(&a).available, Credits(400));
        assert_eq!(l.balance(&b).available, Credits::ZERO);
        assert_conserved(&l).unwrap();
    }

    #[test]
    fn the_single_transaction_cap_is_enforced() {
        let (a, b) = (did(7), did(8));
        let mut l = Ledger::new();
        l.mint(&a, Credits(10_000)).unwrap();
        let p = policy(0, 0, 1_000); // 单笔上限 = 可用余额的 10% = 1_000
        assert_eq!(
            check_spend(&l, &a, Credits(1_001), &p).unwrap(),
            SpendVerdict::OverSingleCap
        );
        assert_eq!(spend(&mut l, &a, &b, Credits(1_001), &p), Err(CoreError::InsufficientFunds));
        spend(&mut l, &a, &b, Credits(1_000), &p).unwrap();
        assert_eq!(l.balance(&b).available, Credits(1_000));
        assert_conserved(&l).unwrap();
    }

    #[test]
    fn zero_amount_is_a_call_error_not_a_decision() {
        let a = did(9);
        let mut l = Ledger::new();
        l.mint(&a, Credits(10)).unwrap();
        let p = BalancePolicy::DEFAULT;
        assert_eq!(check_spend(&l, &a, Credits::ZERO, &p).unwrap(), SpendVerdict::ZeroAmount);
        assert_eq!(spend(&mut l, &a, &did(10), Credits::ZERO, &p), Err(CoreError::ZeroAmount));
        assert_conserved(&l).unwrap();
    }

    #[test]
    fn autostake_locks_up_to_the_target_and_never_crosses_the_floor() {
        let a = did(11);
        let mut l = Ledger::new();
        l.mint(&a, Credits(1_000)).unwrap();
        let p = policy(100, 2_000, 5_000); // 目标锁定 = 1_000 的 20% = 200
        assert_eq!(stake_target(&l, &a, &p).unwrap(), Credits(200));
        assert_eq!(stake_deficit(&l, &a, &p).unwrap(), Credits(200));
        assert_eq!(autostake(&mut l, &a, &p, Credits(1_000)).unwrap(), Credits(200));
        assert_eq!(l.balance(&a).locked, Credits(200));
        assert_eq!(l.balance(&a).available, Credits(800));
        // 已达标：再补一次必须拒绝，而不是静默成功。
        assert_eq!(autostake(&mut l, &a, &p, Credits(1_000)), Err(CoreError::ZeroAmount));
        assert_conserved(&l).unwrap();
    }

    #[test]
    fn autostake_respects_budget_and_reserve_floor() {
        let a = did(12);
        let mut l = Ledger::new();
        l.mint(&a, Credits(200)).unwrap();
        let p = policy(150, 10_000, 10_000); // 缺口 200，可用额度只有 50
        assert_eq!(autostake(&mut l, &a, &p, Credits(1_000)).unwrap(), Credits(50));
        assert_eq!(l.balance(&a).locked, Credits(50));
        let small = did(13);
        l.mint(&small, Credits(200)).unwrap();
        assert_eq!(autostake(&mut l, &small, &p, Credits(30)).unwrap(), Credits(30));
        assert_eq!(l.balance(&small).locked, Credits(30));
        assert_conserved(&l).unwrap();
    }

    #[test]
    fn conservation_holds_across_a_scripted_sequence() {
        let (a, b, c) = (did(14), did(15), did(16));
        let mut l = Ledger::new();
        let mut book = BalanceManager::new();
        let p = policy(100, 2_000, 8_000);
        book.declare(&a, p).unwrap();
        l.mint(&a, Credits(1_000)).unwrap();
        assert_conserved(&l).unwrap();
        book.spend(&mut l, &a, &b, Credits(200)).unwrap();
        assert_conserved(&l).unwrap();
        book.autostake(&mut l, &a, Credits(500)).unwrap();
        assert_conserved(&l).unwrap();
        l.transfer(&b, &c, Credits(50)).unwrap();
        assert_conserved(&l).unwrap();
        l.lock(&c, Credits(20)).unwrap();
        l.slash(&c, Credits(999)).unwrap(); // 罚没受锁定余额约束
        assert_conserved(&l).unwrap();
        assert_eq!(l.slashed(), Credits(20));
        let report = book.report(&l, &a).unwrap();
        assert!(report.conserved);
        // 支付 200 后总资产 = 800；目标质押 = 800 × 20% = 160，锁定后可用 = 640。
        assert_eq!(report.available, Credits(640));
        assert_eq!(report.locked, Credits(160));
        assert_eq!(report.spendable, Credits(540));
    }

    #[test]
    fn delta_reports_signed_changes() {
        let before = Account {
            available: Credits(1_000),
            locked: Credits(100),
        };
        let after = Account {
            available: Credits(700),
            locked: Credits(400),
        };
        let d = delta(&before, &after);
        assert_eq!(d.available, -300);
        assert_eq!(d.locked, 300);
        assert_eq!(d.total, 0);
    }
}
