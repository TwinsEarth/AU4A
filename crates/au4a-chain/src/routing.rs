//! v1.8.5 结算路由：金额 / 时效 / 费用阈值决定走哪条轨（**确定性测试网原型**）。
//!
//! 四条轨：`internal`（不上链）、`rgb`、`taproot`、`x402`。决策表**第一优先级是 fail-closed**：
//! 只要双轨对账不一致（本地托管 ≠ 链上表示），就一律**缓办**——不猜、不硬走、不改账本。
//! 这与 v1.4.3 的兑换路由是同一套思路，但这里的「轨」是真实的适配器语义：
//! 选中链上轨之后，账务走 [`crate::bridge::BridgeBook::bridge_out`]（本地锁定 + 链上表示）。
//!
//! ```text
//! 0. amount == 0                        → 拒绝 ZeroAmount
//! 1. 双轨不一致（fail-closed）           → defer / reconciliation_failed
//! 2. amount < 最低链上金额               → internal / below_onchain_minimum
//! 3. 没有合格轨（金额/时效/费用/最终性）  → defer / rail_unavailable | fee_above_threshold | deadline_too_tight
//! 4. 有偏好轨且合格                      → onchain / preferred_rail
//! 5. 其余                                → onchain / cheapest_rail（按 费用 → 最终性 → 名字 排序）
//! ```
//!
//! 证据等级恒为 `cpu-proto`。

use au4a_core::{CoreError, CoreResult, Credits, Did, EvidenceGrade, Ledger, RefusalCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::bridge::{BridgeBook, BridgeEvent, Reconciliation};
use crate::testnet::ONCHAIN_GRADE;

/// 结算轨。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rail {
    /// 本地积分内结算，不上链。
    Internal,
    /// BTC 侧 RGB。
    Rgb,
    /// BTC 侧 Taproot Assets。
    Taproot,
    /// ETH 侧 x402。
    X402,
}

impl Rail {
    pub const ALL: [Rail; 4] = [Rail::Internal, Rail::Rgb, Rail::Taproot, Rail::X402];

    pub fn as_str(self) -> &'static str {
        match self {
            Rail::Internal => "internal",
            Rail::Rgb => "rgb",
            Rail::Taproot => "taproot",
            Rail::X402 => "x402",
        }
    }

    pub fn is_onchain(self) -> bool {
        !matches!(self, Rail::Internal)
    }
}

/// 时效要求。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Urgency {
    Standard,
    Expedited,
}

/// 路由动作。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteAction {
    Internal,
    Onchain,
    Defer,
}

impl RouteAction {
    pub fn as_str(self) -> &'static str {
        match self {
            RouteAction::Internal => "internal",
            RouteAction::Onchain => "onchain",
            RouteAction::Defer => "defer",
        }
    }
}

/// 决策理由码。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionReason {
    ReconciliationFailed,
    BelowOnchainMinimum,
    RailUnavailable,
    FeeAboveThreshold,
    DeadlineTooTight,
    PreferredRail,
    CheapestRail,
}

impl DecisionReason {
    pub fn as_str(self) -> &'static str {
        match self {
            DecisionReason::ReconciliationFailed => "reconciliation_failed",
            DecisionReason::BelowOnchainMinimum => "below_onchain_minimum",
            DecisionReason::RailUnavailable => "rail_unavailable",
            DecisionReason::FeeAboveThreshold => "fee_above_threshold",
            DecisionReason::DeadlineTooTight => "deadline_too_tight",
            DecisionReason::PreferredRail => "preferred_rail",
            DecisionReason::CheapestRail => "cheapest_rail",
        }
    }
}

/// 一条轨的条件（费用基点、金额区间、时效与最终性要求）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RailTerms {
    pub rail: Rail,
    pub fee_bp: i64,
    pub min_amount: Credits,
    pub max_amount: Credits,
    pub min_slack_ticks: u64,
    pub finality_ticks: u64,
}

impl RailTerms {
    pub fn validate(&self) -> CoreResult<()> {
        if self.fee_bp < 0 {
            return Err(CoreError::NegativeAmount);
        }
        if self.fee_bp > 10_000 {
            return Err(CoreError::InvalidKind);
        }
        if self.min_amount > self.max_amount {
            return Err(CoreError::InvalidKind);
        }
        Ok(())
    }

    pub fn fee_for(&self, amount: Credits) -> CoreResult<Credits> {
        amount.scaled_bp(self.fee_bp)
    }

    pub fn accepts_amount(&self, amount: Credits) -> bool {
        amount >= self.min_amount && amount <= self.max_amount
    }
}

/// 路由表。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingTable {
    pub rails: Vec<RailTerms>,
    /// 费用阈值：费率高于它的轨一律不合格。
    pub max_fee_bp: i64,
    /// 最终性阈值：等待时间超过它的轨一律不合格。
    pub max_finality_ticks: u64,
    /// 低于这个金额一律走内部。
    pub internal_threshold: Credits,
}

impl RoutingTable {
    /// 默认表：RGB 80bp/最终性 6、Taproot 120bp/最终性 6、x402 60bp/最终性 12；
    /// 费率上限 150bp、最终性上限 12、50 微积分以下不上链。
    pub fn default_table() -> Self {
        Self {
            rails: vec![
                RailTerms {
                    rail: Rail::Rgb,
                    fee_bp: 80,
                    min_amount: Credits(200),
                    max_amount: Credits(1_000_000),
                    min_slack_ticks: 20,
                    finality_ticks: 6,
                },
                RailTerms {
                    rail: Rail::Taproot,
                    fee_bp: 120,
                    min_amount: Credits(100),
                    max_amount: Credits(1_000_000),
                    min_slack_ticks: 30,
                    finality_ticks: 6,
                },
                RailTerms {
                    rail: Rail::X402,
                    fee_bp: 60,
                    min_amount: Credits(50),
                    max_amount: Credits(100_000),
                    min_slack_ticks: 10,
                    finality_ticks: 12,
                },
            ],
            max_fee_bp: 150,
            max_finality_ticks: 12,
            internal_threshold: Credits(50),
        }
    }

    pub fn validate(&self) -> CoreResult<()> {
        for terms in &self.rails {
            terms.validate()?;
        }
        Ok(())
    }

    pub fn terms_for(&self, rail: Rail) -> Option<&RailTerms> {
        self.rails.iter().find(|t| t.rail == rail)
    }
}

impl Default for RoutingTable {
    fn default() -> Self {
        Self::default_table()
    }
}

/// 一次结算请求。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementRequest {
    pub payer: Did,
    pub payee: Did,
    pub amount: Credits,
    pub urgency: Urgency,
    pub slack_ticks: u64,
    pub preferred: Option<Rail>,
}

impl SettlementRequest {
    pub fn new(payer: &Did, payee: &Did, amount: i64) -> CoreResult<Self> {
        Ok(Self {
            payer: payer.clone(),
            payee: payee.clone(),
            amount: Credits::new(amount)?,
            urgency: Urgency::Standard,
            slack_ticks: 60,
            preferred: None,
        })
    }

    pub fn with_slack(mut self, slack_ticks: u64) -> Self {
        self.slack_ticks = slack_ticks;
        self
    }

    pub fn with_preference(mut self, rail: Rail) -> Self {
        self.preferred = Some(rail);
        self
    }
}

/// 路由决策。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteDecision {
    pub action: RouteAction,
    pub rail: Rail,
    pub reason: DecisionReason,
    pub amount: Credits,
    pub fee: Credits,
    pub net: Credits,
    pub eta_ticks: u64,
    pub required_slack_ticks: u64,
    pub grade: EvidenceGrade,
    /// 决策时双轨是否一致（不一致时一律缓办）。
    pub reconciliation_ok: bool,
    pub fail_closed: bool,
}

impl RouteDecision {
    pub fn to_json(&self) -> Value {
        json!({
            "action": self.action.as_str(),
            "rail": self.rail.as_str(),
            "reason": self.reason.as_str(),
            "amount": self.amount,
            "fee": self.fee,
            "net": self.net,
            "eta_ticks": self.eta_ticks,
            "required_slack_ticks": self.required_slack_ticks,
            "grade": self.grade.as_str(),
            "reconciliation_ok": self.reconciliation_ok,
            "fail_closed": self.fail_closed,
        })
    }
}

/// 决策表（纯函数）。`reconciliation_ok` 由调用方用 [`crate::bridge::reconcile`] 算出。
pub fn route(
    request: &SettlementRequest,
    table: &RoutingTable,
    reconciliation_ok: bool,
) -> CoreResult<RouteDecision> {
    table.validate()?;
    if request.amount == Credits::ZERO {
        return Err(CoreError::ZeroAmount);
    }
    let required_slack = match request.urgency {
        Urgency::Standard => 0u64,
        Urgency::Expedited => 30u64,
    };
    let defer = |reason: DecisionReason, rail: Rail, eta: u64| RouteDecision {
        action: RouteAction::Defer,
        rail,
        reason,
        amount: request.amount,
        fee: Credits::ZERO,
        net: request.amount,
        eta_ticks: eta,
        required_slack_ticks: required_slack,
        grade: ONCHAIN_GRADE,
        reconciliation_ok,
        fail_closed: true,
    };

    // 1. fail-closed：双轨不一致 → 一律缓办（不猜、不动账）。
    if !reconciliation_ok {
        return Ok(defer(DecisionReason::ReconciliationFailed, Rail::Internal, 0));
    }
    // 2. 小额不上链。
    if request.amount < table.internal_threshold {
        return Ok(RouteDecision {
            action: RouteAction::Internal,
            rail: Rail::Internal,
            reason: DecisionReason::BelowOnchainMinimum,
            amount: request.amount,
            fee: Credits::ZERO,
            net: request.amount,
            eta_ticks: 0,
            required_slack_ticks: required_slack,
            grade: ONCHAIN_GRADE,
            reconciliation_ok,
            fail_closed: true,
        });
    }
    let slack_need = |terms: &RailTerms| terms.min_slack_ticks.max(required_slack);
    let eligible: Vec<&RailTerms> = table
        .rails
        .iter()
        .filter(|t| t.accepts_amount(request.amount))
        .filter(|t| t.fee_bp <= table.max_fee_bp)
        .filter(|t| t.finality_ticks <= table.max_finality_ticks)
        .filter(|t| request.slack_ticks >= slack_need(t))
        .collect();
    if eligible.is_empty() {
        // 给出最具体的原因：金额区间 → 费用 → 时效。
        let by_amount: Vec<&RailTerms> = table
            .rails
            .iter()
            .filter(|t| t.accepts_amount(request.amount))
            .collect();
        if by_amount.is_empty() {
            return Ok(defer(DecisionReason::RailUnavailable, Rail::Internal, 0));
        }
        let by_fee: Vec<&RailTerms> = by_amount
            .iter()
            .copied()
            .filter(|t| t.fee_bp <= table.max_fee_bp)
            .collect();
        if by_fee.is_empty() {
            return Ok(defer(DecisionReason::FeeAboveThreshold, by_amount[0].rail, 0));
        }
        let best = by_fee[0];
        let mut decision = defer(DecisionReason::DeadlineTooTight, best.rail, 0);
        decision.required_slack_ticks = slack_need(best);
        return Ok(decision);
    }
    // 3. 偏好轨（若合格）。
    if let Some(preferred) = request.preferred {
        if let Some(terms) = eligible.iter().find(|t| t.rail == preferred) {
            return Ok(pick(terms, DecisionReason::PreferredRail, request, required_slack));
        }
    }
    // 4. 最便宜轨：费用 → 最终性 → 名字（确定性）。
    let mut sorted = eligible;
    sorted.sort_by(|a, b| {
        (a.fee_bp, a.finality_ticks, a.rail.as_str()).cmp(&(b.fee_bp, b.finality_ticks, b.rail.as_str()))
    });
    let best = sorted.first().copied().ok_or(CoreError::InvalidKind)?;
    Ok(pick(best, DecisionReason::CheapestRail, request, required_slack))
}

fn pick(
    terms: &RailTerms,
    reason: DecisionReason,
    request: &SettlementRequest,
    required_slack: u64,
) -> RouteDecision {
    let fee = terms.fee_for(request.amount).unwrap_or_default();
    let net = request.amount.checked_sub(fee).unwrap_or_default();
    RouteDecision {
        action: RouteAction::Onchain,
        rail: terms.rail,
        reason,
        amount: request.amount,
        fee,
        net,
        eta_ticks: terms.finality_ticks,
        required_slack_ticks: terms.min_slack_ticks.max(required_slack),
        grade: ONCHAIN_GRADE,
        reconciliation_ok: true,
        fail_closed: true,
    }
}

/// 结算执行结果。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettlementOutcome {
    pub decision: RouteDecision,
    /// 实际动账金额（链上轨为托管量，内部轨为转账量）。
    pub moved: Credits,
    pub deferred: bool,
    pub bridge_event: Option<BridgeEvent>,
    pub reconciliation: Reconciliation,
}

/// 结算：**先 fail-closed 对账，再按决策动账，最后再对账一次**。
///
/// * 不一致 → 直接 `Err`（`CoreError::InvalidKind`），账本不动；
/// * `Onchain` → `bridge_out`（本地锁定 + 链上表示），随后再次 `require_consistent`；
/// * `Internal` → 本地转账；
/// * `Defer` → 不动账，返回 `deferred = true`（调用方可重试）。
pub fn settle(
    ledger: &mut Ledger,
    book: &mut BridgeBook,
    request: &SettlementRequest,
    table: &RoutingTable,
) -> CoreResult<SettlementOutcome> {
    let before = book.require_consistent(ledger)?;
    let decision = route(request, table, before.consistent)?;
    match decision.action {
        RouteAction::Defer => Ok(SettlementOutcome {
            decision,
            moved: Credits::ZERO,
            deferred: true,
            bridge_event: None,
            reconciliation: before,
        }),
        RouteAction::Internal => {
            let moved = decision.amount;
            ledger.transfer(&request.payer, &request.payee, moved)?;
            ledger.check_conservation()?;
            let after = book.require_consistent(ledger)?;
            Ok(SettlementOutcome {
                decision,
                moved,
                deferred: false,
                bridge_event: None,
                reconciliation: after,
            })
        }
        RouteAction::Onchain => {
            let moved = decision.amount;
            let event = book.bridge_out(
                ledger,
                &request.payer,
                moved,
                &format!("rail:{}", decision.rail.as_str()),
                decision.rail.as_str(),
            )?;
            let after = book.require_consistent(ledger)?;
            Ok(SettlementOutcome {
                decision,
                moved,
                deferred: false,
                bridge_event: Some(event),
                reconciliation: after,
            })
        }
    }
}

/// 与 [`Rail`] 对应的拒绝码说明：轨道级不支持时用 [`RefusalCode::Unsupported`]。
pub const UNSUPPORTED_RAIL_CODE: RefusalCode = RefusalCode::Unsupported;

#[cfg(test)]
mod tests {
    use super::*;
    use au4a_core::AgentKeys;

    fn did(seed: u8) -> Did {
        AgentKeys::from_seed(&[seed; 32]).did()
    }

    fn request(amount: i64, slack: u64) -> SettlementRequest {
        SettlementRequest::new(&did(1), &did(2), amount)
            .unwrap()
            .with_slack(slack)
    }

    #[test]
    fn the_fail_closed_gate_comes_before_everything_else() {
        let table = RoutingTable::default_table();
        // 一笔完全合格的请求，但双轨不一致 → 缓办，理由 reconciliation_failed。
        let decision = route(&request(1_000, 100), &table, false).unwrap();
        assert_eq!(decision.action, RouteAction::Defer);
        assert_eq!(decision.reason, DecisionReason::ReconciliationFailed);
        assert_eq!(decision.fee, Credits::ZERO);
        assert_eq!(decision.net, Credits(1_000));
        assert!(!decision.reconciliation_ok);
        assert!(decision.fail_closed);
    }

    #[test]
    fn tiny_amounts_stay_internal() {
        let table = RoutingTable::default_table();
        let decision = route(&request(49, 100), &table, true).unwrap();
        assert_eq!(decision.action, RouteAction::Internal);
        assert_eq!(decision.reason, DecisionReason::BelowOnchainMinimum);
        assert_eq!(decision.fee, Credits::ZERO);
        assert_eq!(decision.net, Credits(49));
    }

    #[test]
    fn the_cheapest_eligible_rail_wins_with_a_deterministic_tie_break() {
        let table = RoutingTable::default_table();
        // x402：60bp、最终性 12、金额 ≥50 → 1_000 时最便宜。
        let decision = route(&request(1_000, 100), &table, true).unwrap();
        assert_eq!(decision.action, RouteAction::Onchain);
        assert_eq!(decision.rail, Rail::X402);
        assert_eq!(decision.reason, DecisionReason::CheapestRail);
        assert_eq!(decision.fee, Credits(6)); // 1000 × 60 / 10000
        assert_eq!(decision.net, Credits(994));
        assert_eq!(decision.eta_ticks, 12);
        // 金额 60、时效刚好 10 → x402 仍可；RGB 需要 200 起。
        let small = route(&request(60, 10), &table, true).unwrap();
        assert_eq!(small.rail, Rail::X402);
    }

    #[test]
    fn a_preferred_rail_wins_when_it_is_eligible() {
        let table = RoutingTable::default_table();
        let preferred = request(1_000, 100).with_preference(Rail::Rgb);
        let decision = route(&preferred, &table, true).unwrap();
        assert_eq!(decision.rail, Rail::Rgb);
        assert_eq!(decision.reason, DecisionReason::PreferredRail);
        assert_eq!(decision.fee, Credits(8)); // 1000 × 80 / 10000
        // 偏好轨不合格时退回最便宜轨，而不是失败。
        let too_small = request(60, 100).with_preference(Rail::Rgb);
        let fallback = route(&too_small, &table, true).unwrap();
        assert_eq!(fallback.rail, Rail::X402);
        assert_eq!(fallback.reason, DecisionReason::CheapestRail);
    }

    #[test]
    fn deadline_fee_and_range_reasons_are_specific() {
        let table = RoutingTable::default_table();
        // 时效 9：x402 需要 10、Taproot 30、RGB 20 → 全不合格。
        let tight = route(&request(1_000, 9), &table, true).unwrap();
        assert_eq!(tight.action, RouteAction::Defer);
        assert_eq!(tight.reason, DecisionReason::DeadlineTooTight);
        // 费率阈值 50bp：x402(60) 被挡，RGB(80)/Taproot(120) 也被挡。
        let strict = RoutingTable {
            max_fee_bp: 50,
            ..RoutingTable::default_table()
        };
        let fee = route(&request(1_000, 100), &strict, true).unwrap();
        assert_eq!(fee.reason, DecisionReason::FeeAboveThreshold);
        // 金额超过所有轨的区间。
        let huge = route(&request(2_000_000, 100), &table, true).unwrap();
        assert_eq!(huge.reason, DecisionReason::RailUnavailable);
        // 零额是调用错误。
        assert_eq!(
            route(&request(0, 100), &table, true),
            Err(CoreError::ZeroAmount)
        );
    }

    #[test]
    fn expedited_requests_need_more_slack() {
        let table = RoutingTable::default_table();
        let mut expedited = request(1_000, 25);
        expedited.urgency = Urgency::Expedited;
        let decision = route(&expedited, &table, true).unwrap();
        assert_eq!(decision.action, RouteAction::Defer);
        assert_eq!(decision.reason, DecisionReason::DeadlineTooTight);
        assert_eq!(decision.required_slack_ticks, 30);
        assert_eq!(decision.eta_ticks, 0);
    }

    #[test]
    fn settling_onchain_locks_locally_and_keeps_both_tracks_equal() {
        let payer = did(3);
        let mut ledger = Ledger::new();
        ledger.mint(&payer, Credits(5_000)).unwrap();
        let mut book = BridgeBook::new();
        let table = RoutingTable::default_table();
        let mut req = request(1_000, 100);
        req.payer = payer.clone();
        let outcome = settle(&mut ledger, &mut book, &req, &table).unwrap();
        assert!(!outcome.deferred);
        assert_eq!(outcome.decision.rail, Rail::X402);
        assert_eq!(outcome.moved, Credits(1_000));
        assert_eq!(ledger.balance(&payer).available, Credits(4_000));
        assert_eq!(ledger.balance(&payer).locked, Credits(1_000));
        assert_eq!(book.escrowed(), Credits(1_000));
        assert_eq!(book.chain_supply(), Credits(1_000));
        assert!(outcome.reconciliation.consistent);
        ledger.check_conservation().unwrap();
        // 桥回之后双轨归零，本地余额恢复。
        let recipient = did(4);
        book.bridge_in(&mut ledger, &payer, &recipient, Credits(1_000), "rail:x402", "x402")
            .unwrap();
        ledger.check_conservation().unwrap();
        book.require_consistent(&ledger).unwrap();
        assert_eq!(ledger.balance(&recipient).available, Credits(1_000));
    }

    #[test]
    fn settling_internally_moves_credits_without_touching_the_chain_track() {
        let payer = did(5);
        let payee = did(6);
        let mut ledger = Ledger::new();
        ledger.mint(&payer, Credits(500)).unwrap();
        let mut book = BridgeBook::new();
        let table = RoutingTable::default_table();
        let req = SettlementRequest::new(&payer, &payee, 20).unwrap();
        let outcome = settle(&mut ledger, &mut book, &req, &table).unwrap();
        assert_eq!(outcome.decision.action, RouteAction::Internal);
        assert_eq!(outcome.moved, Credits(20));
        assert_eq!(book.chain_supply(), Credits::ZERO);
        assert_eq!(ledger.balance(&payee).available, Credits(20));
        ledger.check_conservation().unwrap();
    }

    #[test]
    fn an_inconsistent_book_blocks_settlement_and_moves_nothing() {
        let payer = did(7);
        let mut ledger = Ledger::new();
        ledger.mint(&payer, Credits(5_000)).unwrap();
        let mut book = BridgeBook::new();
        book.bridge_out(&mut ledger, &payer, Credits(100), "rail:rgb", "rgb")
            .unwrap();
        book.force_chain_supply(Credits(9_999)); // 故障注入：链上侧多报
        let table = RoutingTable::default_table();
        let mut req = request(1_000, 100);
        req.payer = payer.clone();
        assert_eq!(
            settle(&mut ledger, &mut book, &req, &table),
            Err(CoreError::InvalidKind)
        );
        // 账本没有任何新动账（只有之前那 100 托管）。
        assert_eq!(ledger.balance(&payer).locked, Credits(100));
        assert_eq!(ledger.balance(&payer).available, Credits(4_900));
        ledger.check_conservation().unwrap();
    }

    #[test]
    fn deferred_settlements_leave_the_ledger_untouched() {
        let payer = did(8);
        let mut ledger = Ledger::new();
        ledger.mint(&payer, Credits(5_000)).unwrap();
        let mut book = BridgeBook::new();
        let table = RoutingTable::default_table();
        let mut req = request(1_000, 5);
        req.payer = payer.clone();
        let before = ledger.view();
        let outcome = settle(&mut ledger, &mut book, &req, &table).unwrap();
        assert!(outcome.deferred);
        assert_eq!(outcome.moved, Credits::ZERO);
        assert_eq!(ledger.view(), before);
        assert_eq!(outcome.decision.grade, EvidenceGrade::CpuProto);
    }

    #[test]
    fn decisions_are_reproducible_and_float_free() {
        let table = RoutingTable::default_table();
        for amount in [60i64, 200, 1_000, 99_999] {
            for slack in [10u64, 25, 60, 600] {
                let a = route(&request(amount, slack), &table, true).unwrap();
                let b = route(&request(amount, slack), &table, true).unwrap();
                assert_eq!(a, b, "amount={amount} slack={slack}");
                let value = a.to_json();
                au4a_core::canonicalize(&value).unwrap();
                assert_eq!(value["grade"], json!("cpu-proto"));
            }
        }
        // 表参数非法时拒绝。
        let bad = RoutingTable {
            rails: vec![RailTerms {
                rail: Rail::Rgb,
                fee_bp: 10_001,
                min_amount: Credits(1),
                max_amount: Credits(2),
                min_slack_ticks: 0,
                finality_ticks: 0,
            }],
            ..RoutingTable::default_table()
        };
        assert_eq!(
            route(&request(100, 100), &bad, true),
            Err(CoreError::InvalidKind)
        );
    }

    #[test]
    fn rail_names_and_refusal_code_are_stable() {
        assert_eq!(UNSUPPORTED_RAIL_CODE, RefusalCode::Unsupported);
        let names: Vec<&str> = Rail::ALL.iter().map(|r| r.as_str()).collect();
        assert_eq!(names, vec!["internal", "rgb", "taproot", "x402"]);
        assert!(!Rail::Internal.is_onchain());
        assert!(Rail::Rgb.is_onchain());
        assert_eq!(Rail::ALL.len(), 4);
    }
}
