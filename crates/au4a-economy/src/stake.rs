//! v1.4.4 质押管理：Agent **自主**质押与解质押。
//!
//! 质押在 AU4A 里不是「运营方扣的保证金」，而是 Agent 自己押上的信用：它自己决定押多少、
//! 自己发起解质押，冷静期由**逻辑时钟**驱动（库代码不读墙钟）。
//!
//! 账务语义：
//!
//! * **质押**：可用 → 锁定（[`Ledger::lock`]），并登记 [`StakePosition`]；
//! * **解质押**：锁定 → 解绑中（账本上仍然锁定，仍受罚没约束），冷静期到点后
//!   [`StakeBook::release_matured`] 才把它解锁回可用；
//! * **罚没**：由 [`StakeBook::absorb_slash`] 与 `Ledger::slash` 配对，质押簿跟随账本削减；
//! * **准入线**：任何时刻质押都不得低于 `min_stake`，也不得超过总资产的 `max_stake_bp`。
//!
//! 质押簿与账本之间只有一条不变式（并被测试钉住）：**质押簿声称的锁定量永远不超过账本
//! 实际锁定量**。之所以是「不超过」而不是「等于」——因为链上兑换的在途预留也住在 `locked`
//! 里（v1.4.3）；簿记可以少记，绝不能多记（多记就等于凭空承诺不存在的质押）。

use std::collections::BTreeMap;

use au4a_core::{CoreError, CoreResult, Credits, Did, Ledger};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 质押条款（网络底线 + Agent 自己接受的条件）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StakeTerms {
    /// 准入下限：任何时刻质押不得低于它。
    pub min_stake: Credits,
    /// 解质押冷静期（逻辑时间片）。
    pub cooldown_ticks: u64,
    /// 单账户质押上限（占总资产的万分比）。
    pub max_stake_bp: i64,
}

impl StakeTerms {
    /// 默认条款：下限 10、冷静期 3 个时间片、质押不超过总资产的 50%。
    pub const DEFAULT: StakeTerms = StakeTerms {
        min_stake: Credits(10),
        cooldown_ticks: 3,
        max_stake_bp: 5_000,
    };

    pub fn validate(&self) -> CoreResult<()> {
        if self.max_stake_bp < 0 {
            return Err(CoreError::NegativeAmount);
        }
        if self.max_stake_bp > 10_000 {
            return Err(CoreError::InvalidKind);
        }
        Ok(())
    }
}

impl Default for StakeTerms {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 一笔解绑中的质押。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unbonding {
    pub amount: Credits,
    pub requested_at: u64,
    pub release_at: u64,
}

/// 一个 Agent 的质押头寸。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StakePosition {
    pub did: String,
    /// 仍在质押中的量（受罚没约束）。
    pub locked: Credits,
    /// 解绑中的量（同样受罚没约束，到点才回到可用）。
    pub unbonding: Vec<Unbonding>,
}

impl StakePosition {
    pub fn empty(did: &Did) -> Self {
        Self {
            did: did.as_str().to_string(),
            locked: Credits::ZERO,
            unbonding: Vec::new(),
        }
    }

    pub fn unbonding_total(&self) -> CoreResult<Credits> {
        let mut sum = Credits::ZERO;
        for u in &self.unbonding {
            sum = sum.checked_add(u.amount)?;
        }
        Ok(sum)
    }

    /// 质押簿意义上的承诺量 = 质押中 + 解绑中。
    pub fn committed(&self) -> CoreResult<Credits> {
        self.locked.checked_add(self.unbonding_total()?)
    }
}

/// 质押簿（确定性顺序：按 DID 字节序）。
#[derive(Clone, Debug, Default)]
pub struct StakeBook {
    positions: BTreeMap<Did, StakePosition>,
}

impl StakeBook {
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一个头寸；账本上已经有锁定余额时用它**镜像**，不重复锁定。
    pub fn adopt(
        &mut self,
        ledger: &Ledger,
        terms: &StakeTerms,
        who: &Did,
        amount: Credits,
    ) -> CoreResult<StakePosition> {
        terms.validate()?;
        if amount == Credits::ZERO {
            return Err(CoreError::ZeroAmount);
        }
        if amount < terms.min_stake {
            return Err(CoreError::InsufficientStake);
        }
        if ledger.balance(who).locked < amount {
            return Err(CoreError::InsufficientStake);
        }
        let pos = self
            .positions
            .entry(who.clone())
            .or_insert_with(|| StakePosition::empty(who));
        pos.locked = amount;
        Ok(pos.clone())
    }

    /// 自主质押：可用 → 锁定。低于准入线或超过份额上限一律拒绝（不静默截断）。
    pub fn stake(
        &mut self,
        ledger: &mut Ledger,
        terms: &StakeTerms,
        who: &Did,
        amount: Credits,
    ) -> CoreResult<StakePosition> {
        terms.validate()?;
        if amount == Credits::ZERO {
            return Err(CoreError::ZeroAmount);
        }
        let current = self.position(who);
        let next_locked = current.locked.checked_add(amount)?;
        if next_locked < terms.min_stake {
            return Err(CoreError::InsufficientStake);
        }
        let cap = ledger.balance(who).total()?.scaled_bp(terms.max_stake_bp)?;
        if next_locked > cap {
            return Err(CoreError::InsufficientStake);
        }
        ledger.lock(who, amount)?;
        ledger.check_conservation()?;
        let pos = self
            .positions
            .entry(who.clone())
            .or_insert_with(|| StakePosition::empty(who));
        pos.locked = next_locked;
        Ok(pos.clone())
    }

    /// 自主发起解质押：锁定 → 解绑中。账本此刻**不动**（仍然锁定，仍受罚没约束），
    /// 冷静期到点后由 [`StakeBook::release_matured`] 解锁。
    pub fn request_unstake(
        &mut self,
        terms: &StakeTerms,
        who: &Did,
        amount: Credits,
        now: u64,
    ) -> CoreResult<Unbonding> {
        terms.validate()?;
        if amount == Credits::ZERO {
            return Err(CoreError::ZeroAmount);
        }
        let pos = self.positions.get_mut(who).ok_or(CoreError::UnknownAgent)?;
        if amount > pos.locked {
            return Err(CoreError::InsufficientFunds);
        }
        let remaining = pos.locked.checked_sub(amount)?;
        if remaining < terms.min_stake {
            return Err(CoreError::InsufficientStake);
        }
        pos.locked = remaining;
        let entry = Unbonding {
            amount,
            requested_at: now,
            release_at: now.saturating_add(terms.cooldown_ticks),
        };
        pos.unbonding.push(entry.clone());
        Ok(entry)
    }

    /// 解锁冷静期已满的解绑量；未到点的一律留待下次（不提前释放）。
    pub fn release_matured(
        &mut self,
        ledger: &mut Ledger,
        who: &Did,
        now: u64,
    ) -> CoreResult<Credits> {
        let pos = self.positions.get_mut(who).ok_or(CoreError::UnknownAgent)?;
        let mut matured = Credits::ZERO;
        let mut pending = Vec::new();
        for entry in pos.unbonding.drain(..) {
            if entry.release_at <= now {
                matured = matured.checked_add(entry.amount)?;
            } else {
                pending.push(entry);
            }
        }
        pos.unbonding = pending;
        if matured == Credits::ZERO {
            return Ok(Credits::ZERO);
        }
        ledger.unlock(who, matured)?;
        ledger.check_conservation()?;
        Ok(matured)
    }

    /// 罚没后同步质押簿：把账本削减的量从「质押中」扣掉（不超过质押中余额）。
    ///
    /// 调用方必须配对执行 `Ledger::slash`，否则账本与簿记会漂移，而
    /// [`StakeBook::assert_consistent`] 会立刻抓到。
    pub fn absorb_slash(&mut self, who: &Did, amount: Credits) -> CoreResult<Credits> {
        let pos = self.positions.get_mut(who).ok_or(CoreError::UnknownAgent)?;
        let take = if pos.locked < amount { pos.locked } else { amount };
        pos.locked = pos.locked.checked_sub(take)?;
        Ok(take)
    }

    pub fn position(&self, who: &Did) -> StakePosition {
        self.positions
            .get(who)
            .cloned()
            .unwrap_or_else(|| StakePosition::empty(who))
    }

    pub fn registered(&self) -> usize {
        self.positions.len()
    }

    pub fn total_locked(&self) -> CoreResult<Credits> {
        let mut sum = Credits::ZERO;
        for pos in self.positions.values() {
            sum = sum.checked_add(pos.locked)?;
        }
        Ok(sum)
    }

    pub fn total_unbonding(&self) -> CoreResult<Credits> {
        let mut sum = Credits::ZERO;
        for pos in self.positions.values() {
            sum = sum.checked_add(pos.unbonding_total()?)?;
        }
        Ok(sum)
    }

    /// 不变式：账本守恒；且每个头寸的承诺量不超过账本实际锁定量。
    pub fn assert_consistent(&self, ledger: &Ledger) -> CoreResult<()> {
        ledger.check_conservation()?;
        for (did, pos) in &self.positions {
            if pos.committed()? > ledger.balance(did).locked {
                return Err(CoreError::InsufficientStake);
            }
        }
        Ok(())
    }

    /// 只读 JSON 投影（监控面板数据源的一部分）。
    pub fn to_json(&self) -> Value {
        let positions: Vec<&StakePosition> = self.positions.values().collect();
        json!({
            "positions": positions,
            "registered": self.positions.len(),
            "total_locked": self.total_locked().unwrap_or_default(),
            "total_unbonding": self.total_unbonding().unwrap_or_default(),
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

    fn terms(min: i64, cooldown: u64, cap_bp: i64) -> StakeTerms {
        StakeTerms {
            min_stake: Credits(min),
            cooldown_ticks: cooldown,
            max_stake_bp: cap_bp,
        }
    }

    #[test]
    fn staking_locks_funds_and_records_a_position() {
        let a = did(1);
        let mut ledger = Ledger::new();
        ledger.mint(&a, Credits(1_000)).unwrap();
        let mut book = StakeBook::new();
        let pos = book
            .stake(&mut ledger, &StakeTerms::DEFAULT, &a, Credits(200))
            .unwrap();
        assert_eq!(pos.locked, Credits(200));
        assert_eq!(ledger.balance(&a).available, Credits(800));
        assert_eq!(ledger.balance(&a).locked, Credits(200));
        book.assert_consistent(&ledger).unwrap();
        assert_eq!(book.total_locked().unwrap(), Credits(200));
    }

    #[test]
    fn below_the_admission_line_is_refused() {
        let a = did(2);
        let mut ledger = Ledger::new();
        ledger.mint(&a, Credits(1_000)).unwrap();
        let mut book = StakeBook::new();
        let refuse = terms(100, 3, 5_000);
        assert_eq!(
            book.stake(&mut ledger, &refuse, &a, Credits(99)),
            Err(CoreError::InsufficientStake)
        );
        assert_eq!(
            book.stake(&mut ledger, &refuse, &a, Credits::ZERO),
            Err(CoreError::ZeroAmount)
        );
        assert_eq!(book.registered(), 0);
        ledger.check_conservation().unwrap();
    }

    #[test]
    fn above_the_share_cap_is_refused() {
        let a = did(3);
        let mut ledger = Ledger::new();
        ledger.mint(&a, Credits(1_000)).unwrap();
        let mut book = StakeBook::new();
        let cap = terms(10, 3, 5_000); // 上限 = 总资产 1000 × 50% = 500
        assert_eq!(
            book.stake(&mut ledger, &cap, &a, Credits(501)),
            Err(CoreError::InsufficientStake)
        );
        book.stake(&mut ledger, &cap, &a, Credits(500)).unwrap();
        assert_eq!(ledger.balance(&a).locked, Credits(500));
        book.assert_consistent(&ledger).unwrap();
    }

    #[test]
    fn unstaking_moves_lock_to_unbonding_without_moving_the_ledger() {
        let a = did(4);
        let mut ledger = Ledger::new();
        ledger.mint(&a, Credits(1_000)).unwrap();
        let mut book = StakeBook::new();
        let t = terms(100, 5, 5_000);
        book.stake(&mut ledger, &t, &a, Credits(400)).unwrap();
        let entry = book.request_unstake(&t, &a, Credits(150), 10).unwrap();
        assert_eq!(entry.requested_at, 10);
        assert_eq!(entry.release_at, 15);
        // 账本没动：解绑中的质押仍然锁定、仍然可被罚没。
        assert_eq!(ledger.balance(&a).locked, Credits(400));
        assert_eq!(ledger.balance(&a).available, Credits(600));
        let pos = book.position(&a);
        assert_eq!(pos.locked, Credits(250));
        assert_eq!(pos.unbonding_total().unwrap(), Credits(150));
        assert_eq!(pos.committed().unwrap(), Credits(400));
        book.assert_consistent(&ledger).unwrap();
    }

    #[test]
    fn unstaking_below_the_line_or_without_a_position_is_refused() {
        let (a, stranger) = (did(5), did(6));
        let mut ledger = Ledger::new();
        ledger.mint(&a, Credits(1_000)).unwrap();
        let mut book = StakeBook::new();
        let t = terms(100, 5, 5_000);
        book.stake(&mut ledger, &t, &a, Credits(200)).unwrap();
        // 200 - 150 = 50 < 100：跌破准入线。
        assert_eq!(
            book.request_unstake(&t, &a, Credits(150), 1),
            Err(CoreError::InsufficientStake)
        );
        // 解质押量超过质押中余额。
        assert_eq!(
            book.request_unstake(&t, &a, Credits(250), 1),
            Err(CoreError::InsufficientFunds)
        );
        // 没有头寸的 Agent。
        assert_eq!(
            book.request_unstake(&t, &stranger, Credits(10), 1),
            Err(CoreError::UnknownAgent)
        );
        assert_eq!(book.position(&a).locked, Credits(200));
    }

    #[test]
    fn the_cooldown_is_exact_at_the_boundary() {
        let a = did(7);
        let mut ledger = Ledger::new();
        ledger.mint(&a, Credits(1_000)).unwrap();
        let mut book = StakeBook::new();
        let t = terms(100, 5, 5_000);
        book.stake(&mut ledger, &t, &a, Credits(300)).unwrap();
        book.request_unstake(&t, &a, Credits(100), 20).unwrap(); // release_at = 25
        // 未到点：不解锁。
        assert_eq!(book.release_matured(&mut ledger, &a, 24).unwrap(), Credits::ZERO);
        assert_eq!(ledger.balance(&a).locked, Credits(300));
        // 到点：精确解锁 100。
        assert_eq!(book.release_matured(&mut ledger, &a, 25).unwrap(), Credits(100));
        assert_eq!(ledger.balance(&a).locked, Credits(200));
        assert_eq!(ledger.balance(&a).available, Credits(800));
        assert_eq!(book.position(&a).unbonding.len(), 0);
        book.assert_consistent(&ledger).unwrap();
    }

    #[test]
    fn slashing_is_bounded_and_the_book_follows() {
        let a = did(8);
        let mut ledger = Ledger::new();
        ledger.mint(&a, Credits(1_000)).unwrap();
        let mut book = StakeBook::new();
        book.stake(&mut ledger, &StakeTerms::DEFAULT, &a, Credits(300)).unwrap();
        // 罚没 10_000 被锁定余额约束为 300（账本层），簿记同步削减。
        ledger.slash(&a, Credits(10_000)).unwrap();
        let absorbed = book.absorb_slash(&a, Credits(10_000)).unwrap();
        assert_eq!(absorbed, Credits(300));
        assert_eq!(book.position(&a).locked, Credits::ZERO);
        assert_eq!(ledger.slashed(), Credits(300));
        assert_eq!(ledger.balance(&a).available, Credits(700));
        book.assert_consistent(&ledger).unwrap();
        // 没有头寸的账户不能同步罚没。
        assert_eq!(book.absorb_slash(&did(9), Credits(1)), Err(CoreError::UnknownAgent));
    }

    #[test]
    fn the_book_may_never_claim_more_than_the_ledger_holds() {
        let a = did(10);
        let mut ledger = Ledger::new();
        ledger.mint(&a, Credits(1_000)).unwrap();
        let mut book = StakeBook::new();
        book.stake(&mut ledger, &StakeTerms::DEFAULT, &a, Credits(200)).unwrap();
        book.assert_consistent(&ledger).unwrap();
        // 账本被外部改动（簿记没有跟随）→ 不变式必须抓到。
        ledger.unlock(&a, Credits(200)).unwrap();
        assert_eq!(
            book.assert_consistent(&ledger),
            Err(CoreError::InsufficientStake)
        );
    }

    #[test]
    fn adopting_existing_stake_does_not_double_lock() {
        let a = did(11);
        let mut ledger = Ledger::new();
        ledger.mint(&a, Credits(1_000)).unwrap();
        ledger.lock(&a, Credits(150)).unwrap(); // 例如内核注册时自带的质押
        let mut book = StakeBook::new();
        let pos = book.adopt(&ledger, &StakeTerms::DEFAULT, &a, Credits(150)).unwrap();
        assert_eq!(pos.locked, Credits(150));
        assert_eq!(ledger.balance(&a).locked, Credits(150)); // 没有重复锁定
        assert_eq!(ledger.balance(&a).available, Credits(850));
        book.assert_consistent(&ledger).unwrap();
        // 认领超过实际锁定量 → 拒绝。
        assert_eq!(
            book.adopt(&ledger, &StakeTerms::DEFAULT, &a, Credits(151)),
            Err(CoreError::InsufficientStake)
        );
    }

    #[test]
    fn totals_and_json_projection_are_integer_only() {
        let (a, b) = (did(12), did(13));
        let mut ledger = Ledger::new();
        ledger.mint(&a, Credits(1_000)).unwrap();
        ledger.mint(&b, Credits(1_000)).unwrap();
        let mut book = StakeBook::new();
        let t = terms(100, 2, 8_000);
        book.stake(&mut ledger, &t, &a, Credits(400)).unwrap();
        book.stake(&mut ledger, &t, &b, Credits(200)).unwrap();
        book.request_unstake(&t, &b, Credits(100), 1).unwrap();
        assert_eq!(book.registered(), 2);
        assert_eq!(book.total_locked().unwrap(), Credits(500));
        assert_eq!(book.total_unbonding().unwrap(), Credits(100));
        let value = book.to_json();
        let canonical = au4a_core::canonicalize(&value).unwrap();
        assert!(!canonical.contains('.'));
        assert_eq!(value["registered"], json!(2));
        assert_eq!(value["total_locked"], json!(500));
        book.assert_consistent(&ledger).unwrap();
    }

    #[test]
    fn degenerate_terms_are_refused() {
        let a = did(14);
        let mut ledger = Ledger::new();
        ledger.mint(&a, Credits(1_000)).unwrap();
        let mut book = StakeBook::new();
        assert_eq!(
            book.stake(&mut ledger, &terms(10, 3, 10_001), &a, Credits(10)),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(
            book.stake(&mut ledger, &terms(10, 3, -1), &a, Credits(10)),
            Err(CoreError::NegativeAmount)
        );
        ledger.check_conservation().unwrap();
    }
}
