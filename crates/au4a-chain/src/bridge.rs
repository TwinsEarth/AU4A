//! 双轨账本与 fail-closed 对账。
//!
//! 跨链结算有两条轨：**本地积分**（[`au4a_core::Ledger`]，唯一真相）与**链上表示**
//! （测试网里的资产总量）。桥接的语义必须两边同时记账：
//!
//! ```text
//! 桥出 bridge_out(X)：本地 可用 −X、锁定 +X（托管）   ｜ 链上表示 +X
//! 桥回 bridge_in(X) ：链上表示 −X                    ｜ 本地 锁定 −X → 可用 +X
//! ```
//!
//! 不变式（每次写路径后都断言）：
//!
//! 1. `Σ本地可用 + Σ本地锁定 + 罚没 == 发行`（账本自己的守恒，由内核保证）；
//! 2. `托管量 == 链上表示总量`（双轨一致）；
//! 3. **fail-closed**：两条轨不一致时，[`require_consistent`] 直接**拒绝**，
//!    [`reconcile`] 只报事实但绝不修改任何状态——**账本是真相**，
//!    链上侧的数字永远不会被用来「纠正」账本。

use au4a_core::{CoreError, CoreResult, Credits, Did, Ledger, RefusalCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 桥接方向。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeDirection {
    /// 本地积分 → 链上表示。
    Out,
    /// 链上表示 → 本地积分。
    In,
}

impl BridgeDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            BridgeDirection::Out => "out",
            BridgeDirection::In => "in",
        }
    }
}

/// 一次桥接事件（只读投影会用到）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeEvent {
    pub id: String,
    pub direction: BridgeDirection,
    pub who: String,
    pub amount: Credits,
    /// 链上表示的名字（例如 RGB 资产 id）。
    pub asset: String,
    pub rail: String,
}

/// 桥接台账：托管量（本地锁定）与链上表示总量。
#[derive(Clone, Debug, Default)]
pub struct BridgeBook {
    escrowed: Credits,
    chain_supply: Credits,
    events: Vec<BridgeEvent>,
}

impl BridgeBook {
    pub fn new() -> Self {
        Self::default()
    }

    /// 本地托管量（本地侧为桥接锁定的积分）。
    pub fn escrowed(&self) -> Credits {
        self.escrowed
    }

    /// 链上表示总量（测试网侧）。
    pub fn chain_supply(&self) -> Credits {
        self.chain_supply
    }

    pub fn events(&self) -> &[BridgeEvent] {
        &self.events
    }

    /// 桥出：本地锁定 X，链上表示 +X。
    pub fn bridge_out(
        &mut self,
        ledger: &mut Ledger,
        who: &Did,
        amount: Credits,
        asset: &str,
        rail: &str,
    ) -> CoreResult<BridgeEvent> {
        if amount == Credits::ZERO {
            return Err(CoreError::ZeroAmount);
        }
        if asset.is_empty() {
            return Err(CoreError::InvalidKind);
        }
        // 账本先动：可用 → 锁定（所有权未转移，只是托管）。
        ledger.lock(who, amount)?;
        ledger.check_conservation()?;
        self.escrowed = self.escrowed.checked_add(amount)?;
        self.chain_supply = self.chain_supply.checked_add(amount)?;
        let event = self.record(BridgeDirection::Out, who, amount, asset, rail)?;
        Ok(event)
    }

    /// 桥回：链上表示 −X，本地解锁并转给收款方。
    pub fn bridge_in(
        &mut self,
        ledger: &mut Ledger,
        who: &Did,
        to: &Did,
        amount: Credits,
        asset: &str,
        rail: &str,
    ) -> CoreResult<BridgeEvent> {
        if amount == Credits::ZERO {
            return Err(CoreError::ZeroAmount);
        }
        if asset.is_empty() {
            return Err(CoreError::InvalidKind);
        }
        // fail-closed：链上表示不足就拒绝，绝不透支「链上数字」。
        if self.chain_supply < amount || self.escrowed < amount {
            return Err(CoreError::InsufficientFunds);
        }
        self.chain_supply = self.chain_supply.checked_sub(amount)?;
        self.escrowed = self.escrowed.checked_sub(amount)?;
        ledger.unlock(who, amount)?;
        ledger.transfer(who, to, amount)?;
        ledger.check_conservation()?;
        let event = self.record(BridgeDirection::In, who, amount, asset, rail)?;
        Ok(event)
    }

    fn record(
        &mut self,
        direction: BridgeDirection,
        who: &Did,
        amount: Credits,
        asset: &str,
        rail: &str,
    ) -> CoreResult<BridgeEvent> {
        let index = self.events.len() as u64;
        let id = au4a_core::canonical_hash(&json!({
            "direction": direction,
            "who": who,
            "amount": amount,
            "asset": asset,
            "rail": rail,
            "index": index,
        }))?;
        let event = BridgeEvent {
            id,
            direction,
            who: who.as_str().to_string(),
            amount,
            asset: asset.to_string(),
            rail: rail.to_string(),
        };
        self.events.push(event.clone());
        Ok(event)
    }

    /// **故障注入（仅供审计与测试）**：把链上表示总量设成任意值，
    /// 用来验证 fail-closed 闸门真的会拦住不一致。正式路径从不调用它。
    pub fn force_chain_supply(&mut self, value: Credits) {
        self.chain_supply = value;
    }

    /// 强制一致性检查（fail-closed 的入口）：不一致就拒绝，让调用方停下来。
    ///
    /// `CoreError` 是冻结枚举（没有 `Conflict` 变体），因此双轨不一致统一映射为
    /// [`CoreError::InvalidKind`]（「当前状态不接受这个动作」），映射写在文档里、不新增错误类型。
    pub fn require_consistent(&self, ledger: &Ledger) -> CoreResult<Reconciliation> {
        let report = reconcile(ledger, self)?;
        if report.consistent {
            Ok(report)
        } else {
            Err(CoreError::InvalidKind)
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "escrowed": self.escrowed,
            "chain_supply": self.chain_supply,
            "events": self.events.len(),
            "note": "托管量必须等于链上表示总量（双轨守恒）",
        })
    }
}

/// 对账报告（只读事实，不改任何状态）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reconciliation {
    /// 账本实际锁定量（全部锁定，含质押/在途/托管）。
    pub ledger_locked: Credits,
    /// 账本守恒是否成立。
    pub ledger_conserved: bool,
    /// 本地桥接托管量。
    pub escrowed: Credits,
    /// 链上表示总量。
    pub chain_supply: Credits,
    /// 两条轨是否一致。
    pub consistent: bool,
    /// 是否处于 fail-closed（不一致即拒绝结算）。
    pub fail_closed: bool,
}

/// 只读对账：**账本是真相**——链上侧的数字只用来对比，不用来修改账本。
pub fn reconcile(ledger: &Ledger, book: &BridgeBook) -> CoreResult<Reconciliation> {
    let view = ledger.view();
    let mut locked = Credits::ZERO;
    for account in view.accounts.values() {
        locked = locked.checked_add(account.locked)?;
    }
    let ledger_conserved = ledger.check_conservation().is_ok();
    let consistent = book.escrowed == book.chain_supply;
    Ok(Reconciliation {
        ledger_locked: locked,
        ledger_conserved,
        escrowed: book.escrowed,
        chain_supply: book.chain_supply,
        consistent,
        fail_closed: true,
    })
}

/// 拒绝码说明：桥接违反双轨一致时用 [`RefusalCode::Conflict`]（状态冲突，不是竞争）。
pub const BRIDGE_INCONSISTENCY_CODE: RefusalCode = RefusalCode::Conflict;

#[cfg(test)]
mod tests {
    use super::*;
    use au4a_core::AgentKeys;

    fn did(seed: u8) -> Did {
        AgentKeys::from_seed(&[seed; 32]).did()
    }

    fn funded(seed: u8, amount: i64) -> (Did, Ledger) {
        let who = did(seed);
        let mut ledger = Ledger::new();
        ledger.mint(&who, Credits(amount)).unwrap();
        (who, ledger)
    }

    #[test]
    fn bridging_out_locks_locally_and_mints_the_same_amount_on_chain() {
        let (who, mut ledger) = funded(1, 1_000);
        let mut book = BridgeBook::new();
        let event = book
            .bridge_out(&mut ledger, &who, Credits(400), "rgb:usdt", "rgb")
            .unwrap();
        assert_eq!(event.amount, Credits(400));
        assert_eq!(event.direction, BridgeDirection::Out);
        assert_eq!(ledger.balance(&who).available, Credits(600));
        assert_eq!(ledger.balance(&who).locked, Credits(400));
        assert_eq!(book.escrowed(), Credits(400));
        assert_eq!(book.chain_supply(), Credits(400));
        ledger.check_conservation().unwrap();
        // 双轨一致 → fail-closed 闸门放行。
        let report = book.require_consistent(&ledger).unwrap();
        assert!(report.consistent && report.fail_closed && report.ledger_conserved);
        assert_eq!(report.ledger_locked, Credits(400));
    }

    #[test]
    fn bridging_back_burns_chain_supply_and_releases_the_escrow() {
        let (who, mut ledger) = funded(2, 1_000);
        let recipient = did(3);
        let mut book = BridgeBook::new();
        book.bridge_out(&mut ledger, &who, Credits(400), "rgb:usdt", "rgb")
            .unwrap();
        let event = book
            .bridge_in(&mut ledger, &who, &recipient, Credits(250), "rgb:usdt", "rgb")
            .unwrap();
        assert_eq!(event.direction, BridgeDirection::In);
        assert_eq!(book.escrowed(), Credits(150));
        assert_eq!(book.chain_supply(), Credits(150));
        // who：600 可用 +250 解锁 −250 转出 = 600，锁定 150；收款方 +250。
        assert_eq!(ledger.balance(&who).available, Credits(600));
        assert_eq!(ledger.balance(&who).locked, Credits(150));
        assert_eq!(ledger.balance(&recipient).available, Credits(250));
        ledger.check_conservation().unwrap();
        book.require_consistent(&ledger).unwrap();
        // 双轨总量始终等于初始发行量：600 + 150 + 250 = 1000。
        assert_eq!(
            Credits(
                ledger.balance(&who).available.get()
                    + ledger.balance(&who).locked.get()
                    + ledger.balance(&recipient).available.get()
            ),
            Credits(1_000)
        );
    }

    #[test]
    fn bridging_back_more_than_the_chain_supply_is_refused() {
        let (who, mut ledger) = funded(4, 1_000);
        let mut book = BridgeBook::new();
        book.bridge_out(&mut ledger, &who, Credits(100), "rgb:usdt", "rgb")
            .unwrap();
        assert_eq!(
            book.bridge_in(&mut ledger, &who, &did(5), Credits(101), "rgb:usdt", "rgb"),
            Err(CoreError::InsufficientFunds)
        );
        assert_eq!(book.chain_supply(), Credits(100));
        assert_eq!(ledger.balance(&who).locked, Credits(100));
        ledger.check_conservation().unwrap();
    }

    #[test]
    fn zero_amounts_and_empty_assets_are_refused() {
        let (who, mut ledger) = funded(6, 100);
        let mut book = BridgeBook::new();
        assert_eq!(
            book.bridge_out(&mut ledger, &who, Credits::ZERO, "rgb:usdt", "rgb"),
            Err(CoreError::ZeroAmount)
        );
        assert_eq!(
            book.bridge_out(&mut ledger, &who, Credits(10), "", "rgb"),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(book.events().len(), 0);
        ledger.check_conservation().unwrap();
    }

    #[test]
    fn an_inconsistent_book_fails_closed() {
        let (who, mut ledger) = funded(7, 1_000);
        let mut book = BridgeBook::new();
        book.bridge_out(&mut ledger, &who, Credits(300), "rgb:usdt", "rgb")
            .unwrap();
        // 模拟「链上侧多报了」：直接构造不一致的台账（真实中来自桥的 bug 或分叉）。
        let mut broken = book.clone();
        broken.force_chain_supply(Credits(999));
        let report = reconcile(&ledger, &broken).unwrap();
        assert!(!report.consistent);
        assert_eq!(report.escrowed, Credits(300));
        assert_eq!(report.chain_supply, Credits(999));
        // fail-closed：拒绝继续结算，而不是「相信链上数字」。
        assert_eq!(
            broken.require_consistent(&ledger),
            Err(CoreError::InvalidKind)
        );
        // 账本从未被链上数字改写。
        assert_eq!(ledger.balance(&who).locked, Credits(300));
        assert_eq!(BRIDGE_INCONSISTENCY_CODE, RefusalCode::Conflict);
    }

    #[test]
    fn events_are_content_addressed_and_ordered() {
        let (who, mut ledger) = funded(8, 1_000);
        let mut book = BridgeBook::new();
        let a = book
            .bridge_out(&mut ledger, &who, Credits(100), "rgb:usdt", "rgb")
            .unwrap();
        let b = book
            .bridge_out(&mut ledger, &who, Credits(100), "rgb:usdt", "rgb")
            .unwrap();
        assert_ne!(a.id, b.id, "同样金额的两笔桥接事件 id 必须不同");
        assert_eq!(book.events().len(), 2);
        let canonical = au4a_core::canonicalize(&book.to_json()).unwrap();
        assert!(!canonical.contains('.'));
    }
}
