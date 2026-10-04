//! v1.4.3 自动兑换：系统积分 ↔ BTC/ETH 的**路由决策**。
//!
//! 这一版只做两件事，而且必须把边界画清楚：
//!
//! 1. **决策**：给定金额、时效要求（剩余时间片）、费用阈值，从决策表里选出一条路由；
//! 2. **账务**：选择链上路由时把金额**预留**（可用 → 锁定，仍在守恒等式内），
//!    登记一条「等待 v1.8 执行」的意图；选择内部路由时走内部转账；选择缓办时**不动账本**。
//!
//! **绝不伪造链上成功**。真实链上执行属于 v1.8（Cross-Chain Settlement）。这条边界在类型
//! 层面就封死了：[`ChainExecution`] 只有三个变体——`PendingV18` / `NotApplicable` /
//! `NotRequested`，**没有** `Executed`；[`ChainExecution::executed`] 永远返回 `false`；
//! [`IntentStatus`] 只有 `AwaitingChainExecution` 一个变体。本轨道无法表达「已经上链成功」。
//!
//! 决策表（顺序即优先级，全部整数基点运算）：
//!
//! | # | 条件 | 动作 | 理由码 | 账务 |
//! |---|---|---|---|---|
//! | 0 | `amount == 0` | 拒绝 | — | 不动 |
//! | 1 | Agent 显式偏好内部结算 | 内部 | `preferred_venue` | 内部转账 |
//! | 2 | `amount < onchain_min_amount` | 内部 | `below_onchain_minimum` | 内部转账 |
//! | 3 | `slack_ticks < urgency 所需时间片` | 缓办 | `deadline_too_tight` | 不动 |
//! | 4 | `venue_fee_bp > max_fee_bp` | 缓办 | `fee_above_threshold` | 不动 |
//! | 5 | 其余 | 上链（v1.8 执行） | `cheapest_venue` / `preferred_venue` | 预留（锁定） |

use au4a_core::{CoreError, CoreResult, Credits, Did, Ledger};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 兑换场所：系统积分内部 / BTC / ETH。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Venue {
    Internal,
    Bitcoin,
    Ethereum,
}

impl Venue {
    pub const ALL: [Venue; 3] = [Venue::Internal, Venue::Bitcoin, Venue::Ethereum];

    pub fn as_str(self) -> &'static str {
        match self {
            Venue::Internal => "internal",
            Venue::Bitcoin => "btc",
            Venue::Ethereum => "eth",
        }
    }

    /// 是否需要 v1.8 的真实链上执行。
    pub fn is_onchain(self) -> bool {
        !matches!(self, Venue::Internal)
    }
}

/// 时效要求：普通 / 加急（加急需要更多剩余时间片才敢上链）。
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
    /// 留在系统积分内结算，不上链。
    KeepInternal,
    /// 走链上路由（v1.8 执行；本轨道只预留与登记）。
    RouteOnchain,
    /// 缓办：条件不满足，账本不动。
    Defer,
}

impl RouteAction {
    pub fn as_str(self) -> &'static str {
        match self {
            RouteAction::KeepInternal => "keep_internal",
            RouteAction::RouteOnchain => "route_onchain",
            RouteAction::Defer => "defer",
        }
    }
}

/// 决策理由码（观察层可直接展示，人类不能改写）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionReason {
    BelowOnchainMinimum,
    DeadlineTooTight,
    FeeAboveThreshold,
    PreferredVenue,
    CheapestVenue,
}

impl DecisionReason {
    pub fn as_str(self) -> &'static str {
        match self {
            DecisionReason::BelowOnchainMinimum => "below_onchain_minimum",
            DecisionReason::DeadlineTooTight => "deadline_too_tight",
            DecisionReason::FeeAboveThreshold => "fee_above_threshold",
            DecisionReason::PreferredVenue => "preferred_venue",
            DecisionReason::CheapestVenue => "cheapest_venue",
        }
    }
}

/// 链上执行状态。**没有** `Executed`——本轨道在类型层面无法声称链上成功。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainExecution {
    /// 已预留，等待 v1.8 的真实链上执行。
    PendingV18,
    /// 不需要链上执行（内部结算）。
    NotApplicable,
    /// 决策为缓办，没有发起任何链上动作。
    NotRequested,
}

impl ChainExecution {
    pub const ALL: [ChainExecution; 3] = [
        ChainExecution::PendingV18,
        ChainExecution::NotApplicable,
        ChainExecution::NotRequested,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ChainExecution::PendingV18 => "pending_v1.8",
            ChainExecution::NotApplicable => "not_applicable",
            ChainExecution::NotRequested => "not_requested",
        }
    }

    /// 恒为 `false`：本轨道从不报告链上成功（真实执行在 v1.8）。
    pub const fn executed(self) -> bool {
        false
    }
}

/// 兑换意图的状态。只有「等待 v1.8 执行」一种——不存在「已成交」。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentStatus {
    AwaitingChainExecution,
}

impl IntentStatus {
    pub const ALL: [IntentStatus; 1] = [IntentStatus::AwaitingChainExecution];

    pub fn as_str(self) -> &'static str {
        match self {
            IntentStatus::AwaitingChainExecution => "awaiting_chain_execution",
        }
    }
}

/// 路由决策表参数（Agent 自主选择参数；网络可以给出默认值）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteTable {
    /// BTC 通道预估费率（基点）。
    pub btc_fee_bp: i64,
    /// ETH 通道预估费率（基点）。
    pub eth_fee_bp: i64,
    /// 低于这个金额不上链（小额上链不划算）。
    pub onchain_min_amount: Credits,
    /// 费用阈值：预估费率高于它就缓办。
    pub max_fee_bp: i64,
    /// 普通时效需要的最少剩余时间片。
    pub standard_min_slack_ticks: u64,
    /// 加急时效需要的最少剩余时间片。
    pub expedited_min_slack_ticks: u64,
}

impl RouteTable {
    /// 默认表：BTC 120bp、ETH 80bp，200 微积分以下不上链，费率上限 150bp，
    /// 普通需要 ≥20 个时间片、加急需要 ≥60 个。
    pub const DEFAULT: RouteTable = RouteTable {
        btc_fee_bp: 120,
        eth_fee_bp: 80,
        onchain_min_amount: Credits(200),
        max_fee_bp: 150,
        standard_min_slack_ticks: 20,
        expedited_min_slack_ticks: 60,
    };

    pub fn validate(&self) -> CoreResult<()> {
        validate_bp(self.btc_fee_bp)?;
        validate_bp(self.eth_fee_bp)?;
        validate_bp(self.max_fee_bp)?;
        Ok(())
    }

    pub fn fee_bp(&self, venue: Venue) -> i64 {
        match venue {
            Venue::Internal => 0,
            Venue::Bitcoin => self.btc_fee_bp,
            Venue::Ethereum => self.eth_fee_bp,
        }
    }

    pub fn required_slack_ticks(&self, urgency: Urgency) -> u64 {
        match urgency {
            Urgency::Standard => self.standard_min_slack_ticks,
            Urgency::Expedited => self.expedited_min_slack_ticks,
        }
    }
}

impl Default for RouteTable {
    fn default() -> Self {
        Self::DEFAULT
    }
}

fn validate_bp(bp: i64) -> CoreResult<()> {
    if bp < 0 {
        return Err(CoreError::NegativeAmount);
    }
    if bp > 10_000 {
        return Err(CoreError::InvalidKind);
    }
    Ok(())
}

/// 一次兑换请求（由 Agent 自己发起）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExchangeRequest {
    pub from: Did,
    pub amount: Credits,
    pub urgency: Urgency,
    /// 剩余时间片（逻辑时钟；库代码不读墙钟）。
    pub slack_ticks: u64,
    /// Agent 显式偏好（可选）。
    pub preferred: Option<Venue>,
}

/// 决策结果：路由 + 理由 + 预测费用与到账 + 链上执行状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutePlan {
    pub request: ExchangeRequest,
    pub action: RouteAction,
    pub venue: Venue,
    pub reason: DecisionReason,
    /// 预估费用（整数，向下取整）。
    pub fee: Credits,
    /// 预计到账 = `amount - fee`。
    pub net: Credits,
    pub chain_execution: ChainExecution,
    pub required_slack_ticks: u64,
}

impl RoutePlan {
    pub fn from(&self) -> &Did {
        &self.request.from
    }

    pub fn amount(&self) -> Credits {
        self.request.amount
    }

    pub fn fingerprint(&self) -> CoreResult<String> {
        let value = serde_json::to_value(self).map_err(|_| CoreError::Encoding)?;
        au4a_core::canonical_hash(&value)
    }
}

/// 预估费用：`amount × venue_fee_bp / 10000`（整数，向下取整）。
pub fn fee_for(amount: Credits, venue: Venue, table: &RouteTable) -> CoreResult<Credits> {
    table.validate()?;
    amount.scaled_bp(table.fee_bp(venue))
}

/// 决策表实现（纯函数：同输入 → 同输出）。
pub fn route(request: &ExchangeRequest, table: &RouteTable) -> CoreResult<RoutePlan> {
    table.validate()?;
    if request.amount == Credits::ZERO {
        return Err(CoreError::ZeroAmount);
    }
    let required = table.required_slack_ticks(request.urgency);
    let plan = |action: RouteAction,
                venue: Venue,
                reason: DecisionReason,
                fee: Credits,
                net: Credits,
                chain_execution: ChainExecution| RoutePlan {
        request: request.clone(),
        action,
        venue,
        reason,
        fee,
        net,
        chain_execution,
        required_slack_ticks: required,
    };

    // 1. Agent 显式要求留在系统积分内。
    if request.preferred == Some(Venue::Internal) {
        return Ok(plan(
            RouteAction::KeepInternal,
            Venue::Internal,
            DecisionReason::PreferredVenue,
            Credits::ZERO,
            request.amount,
            ChainExecution::NotApplicable,
        ));
    }
    // 2. 小额不上链。
    if request.amount < table.onchain_min_amount {
        return Ok(plan(
            RouteAction::KeepInternal,
            Venue::Internal,
            DecisionReason::BelowOnchainMinimum,
            Credits::ZERO,
            request.amount,
            ChainExecution::NotApplicable,
        ));
    }
    let venue = match request.preferred {
        Some(v) => v,
        None => cheapest_venue(table),
    };
    let fee = fee_for(request.amount, venue, table)?;
    let net = request.amount.checked_sub(fee)?;
    // 3. 时效不够：缓办，账本不动。
    if request.slack_ticks < required {
        return Ok(plan(
            RouteAction::Defer,
            venue,
            DecisionReason::DeadlineTooTight,
            fee,
            net,
            ChainExecution::NotRequested,
        ));
    }
    // 4. 费用超过阈值：缓办，账本不动。
    if table.fee_bp(venue) > table.max_fee_bp {
        return Ok(plan(
            RouteAction::Defer,
            venue,
            DecisionReason::FeeAboveThreshold,
            fee,
            net,
            ChainExecution::NotRequested,
        ));
    }
    // 5. 上链（v1.8 执行）。
    let reason = if request.preferred.is_some() {
        DecisionReason::PreferredVenue
    } else {
        DecisionReason::CheapestVenue
    };
    Ok(plan(
        RouteAction::RouteOnchain,
        venue,
        reason,
        fee,
        net,
        ChainExecution::PendingV18,
    ))
}

/// 最便宜的链上通道；费率相同时按场所名升序，保证确定性。
fn cheapest_venue(table: &RouteTable) -> Venue {
    let mut best = Venue::Bitcoin;
    for v in [Venue::Bitcoin, Venue::Ethereum] {
        if (table.fee_bp(v), v.as_str()) < (table.fee_bp(best), best.as_str()) {
            best = v;
        }
    }
    best
}

/// 兑换意图：链上路由的账务凭证。`status` 只有「等待 v1.8 执行」。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExchangeIntent {
    pub id: String,
    pub from: Did,
    pub amount: Credits,
    pub fee: Credits,
    pub net: Credits,
    pub venue: String,
    pub status: IntentStatus,
    pub chain_execution: ChainExecution,
    pub at: u64,
    pub fingerprint: String,
}

impl ExchangeIntent {
    /// 恒为 `false`（真实链上执行在 v1.8）。
    pub const fn on_chain_success(&self) -> bool {
        false
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// 预留被撤回的记录（例如计划作废）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub id: String,
    pub amount: Credits,
    pub at: u64,
}

/// 兑换台账：单调追加，顺序确定（供监控只读投影使用）。
#[derive(Clone, Debug, Default)]
pub struct ExchangeBook {
    escrowed: Vec<ExchangeIntent>,
    released: Vec<Release>,
}

impl ExchangeBook {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, intent: ExchangeIntent) {
        self.escrowed.push(intent);
    }

    pub fn mark_released(&mut self, id: &str, amount: Credits, at: u64) {
        self.released.push(Release {
            id: id.to_string(),
            amount,
            at,
        });
    }

    pub fn intents(&self) -> &[ExchangeIntent] {
        &self.escrowed
    }

    pub fn releases(&self) -> &[Release] {
        &self.released
    }

    /// 仍在等待链上执行的意图数。
    pub fn pending(&self) -> usize {
        self.escrowed.len().saturating_sub(self.released.len())
    }

    /// 已登记意图的总额（不代表已成交）。
    pub fn escrowed_total(&self) -> CoreResult<Credits> {
        let mut sum = Credits::ZERO;
        for i in &self.escrowed {
            sum = sum.checked_add(i.amount)?;
        }
        Ok(sum)
    }

    /// 只读 JSON 投影。
    pub fn to_json(&self) -> Value {
        json!({
            "intents": self.escrowed.iter().map(ExchangeIntent::to_json).collect::<Vec<_>>(),
            "released": self.released,
            "pending": self.pending(),
            "chain_executed": false,
            "note": "链上执行属 v1.8；本轨道只登记等待执行的意图",
        })
    }
}

/// 为链上路由做账务预留：`amount` 从可用转入锁定（在途），守恒不变。
///
/// 只有 [`RouteAction::RouteOnchain`] 的计划才能预留；其余动作一律拒绝，
/// 因此「缓办 / 内部结算」在账本上不可能留下痕迹。
pub fn escrow_for_route(
    ledger: &mut Ledger,
    plan: &RoutePlan,
    at: u64,
) -> CoreResult<ExchangeIntent> {
    if plan.action != RouteAction::RouteOnchain {
        return Err(CoreError::InvalidKind);
    }
    ledger.lock(plan.from(), plan.amount())?;
    ledger.check_conservation()?;
    let payload = json!({
        "from": plan.from(),
        "amount": plan.amount(),
        "fee": plan.fee,
        "net": plan.net,
        "venue": plan.venue.as_str(),
        "at": at,
    });
    let id = au4a_core::canonical_hash(&payload)?;
    let intent = ExchangeIntent {
        fingerprint: id.clone(),
        id,
        from: plan.from().clone(),
        amount: plan.amount(),
        fee: plan.fee,
        net: plan.net,
        venue: plan.venue.as_str().to_string(),
        status: IntentStatus::AwaitingChainExecution,
        chain_execution: ChainExecution::PendingV18,
        at,
    };
    Ok(intent)
}

/// 撤回预留：锁定 → 可用（例如计划作废或 v1.8 明确拒绝执行时）。
pub fn cancel_intent(ledger: &mut Ledger, intent: &ExchangeIntent) -> CoreResult<Credits> {
    ledger.unlock(&intent.from, intent.amount)?;
    ledger.check_conservation()?;
    Ok(intent.amount)
}

/// 内部结算：把「留在系统积分内」的决策真正落账（只允许内部动作）。
pub fn apply_internal(ledger: &mut Ledger, plan: &RoutePlan, to: &Did) -> CoreResult<Credits> {
    if plan.action != RouteAction::KeepInternal {
        return Err(CoreError::InvalidKind);
    }
    ledger.transfer(plan.from(), to, plan.amount())?;
    ledger.check_conservation()?;
    Ok(plan.amount())
}

#[cfg(test)]
mod tests {
    use super::*;
    use au4a_core::AgentKeys;

    fn did(seed: u8) -> Did {
        AgentKeys::from_seed(&[seed; 32]).did()
    }

    fn request(
        amount: i64,
        urgency: Urgency,
        slack: u64,
        preferred: Option<Venue>,
    ) -> ExchangeRequest {
        ExchangeRequest {
            from: did(1),
            amount: Credits(amount),
            urgency,
            slack_ticks: slack,
            preferred,
        }
    }

    #[test]
    fn small_amounts_stay_off_chain() {
        let plan = route(&request(199, Urgency::Standard, 100, None), &RouteTable::DEFAULT).unwrap();
        assert_eq!(plan.action, RouteAction::KeepInternal);
        assert_eq!(plan.venue, Venue::Internal);
        assert_eq!(plan.reason, DecisionReason::BelowOnchainMinimum);
        assert_eq!(plan.fee, Credits::ZERO);
        assert_eq!(plan.net, Credits(199));
        assert_eq!(plan.chain_execution, ChainExecution::NotApplicable);
    }

    #[test]
    fn the_cheapest_venue_wins_with_integer_fee_math() {
        let plan =
            route(&request(1_000, Urgency::Standard, 100, None), &RouteTable::DEFAULT).unwrap();
        assert_eq!(plan.action, RouteAction::RouteOnchain);
        assert_eq!(plan.venue, Venue::Ethereum); // 80bp < 120bp
        assert_eq!(plan.reason, DecisionReason::CheapestVenue);
        assert_eq!(plan.fee, Credits(8)); // 1000 × 80 / 10000
        assert_eq!(plan.net, Credits(992));
        assert_eq!(plan.chain_execution, ChainExecution::PendingV18);
        assert!(!plan.chain_execution.executed());
    }

    #[test]
    fn equal_fees_tie_break_by_venue_name_deterministically() {
        let table = RouteTable {
            btc_fee_bp: 90,
            eth_fee_bp: 90,
            ..RouteTable::DEFAULT
        };
        let plan = route(&request(1_000, Urgency::Standard, 100, None), &table).unwrap();
        assert_eq!(plan.venue, Venue::Bitcoin);
        assert_eq!(plan.reason, DecisionReason::CheapestVenue);
    }

    #[test]
    fn a_preferred_venue_overrides_the_cheapest_one() {
        let plan = route(
            &request(1_000, Urgency::Standard, 100, Some(Venue::Bitcoin)),
            &RouteTable::DEFAULT,
        )
        .unwrap();
        assert_eq!(plan.venue, Venue::Bitcoin);
        assert_eq!(plan.reason, DecisionReason::PreferredVenue);
        assert_eq!(plan.fee, Credits(12));
        assert_eq!(plan.net, Credits(988));
    }

    #[test]
    fn preferring_the_internal_venue_keeps_everything_off_chain() {
        let plan = route(
            &request(5_000, Urgency::Standard, 100, Some(Venue::Internal)),
            &RouteTable::DEFAULT,
        )
        .unwrap();
        assert_eq!(plan.action, RouteAction::KeepInternal);
        assert_eq!(plan.reason, DecisionReason::PreferredVenue);
        assert_eq!(plan.chain_execution, ChainExecution::NotApplicable);
    }

    #[test]
    fn a_tight_deadline_defers_without_touching_the_ledger() {
        let plan =
            route(&request(1_000, Urgency::Standard, 19, None), &RouteTable::DEFAULT).unwrap();
        assert_eq!(plan.action, RouteAction::Defer);
        assert_eq!(plan.reason, DecisionReason::DeadlineTooTight);
        assert_eq!(plan.required_slack_ticks, 20);
        assert_eq!(plan.chain_execution, ChainExecution::NotRequested);
        let expedited =
            route(&request(1_000, Urgency::Expedited, 30, None), &RouteTable::DEFAULT).unwrap();
        assert_eq!(expedited.action, RouteAction::Defer);
        assert_eq!(expedited.required_slack_ticks, 60);

        // 缓办的计划不允许预留：账本不可能被一条没通过的决策改动。
        let mut ledger = Ledger::new();
        ledger.mint(&did(1), Credits(10_000)).unwrap();
        assert_eq!(
            escrow_for_route(&mut ledger, &plan, 5),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(ledger.balance(&did(1)).available, Credits(10_000));
        ledger.check_conservation().unwrap();
    }

    #[test]
    fn a_fee_above_threshold_defers() {
        let table = RouteTable {
            btc_fee_bp: 900,
            eth_fee_bp: 800,
            max_fee_bp: 150,
            ..RouteTable::DEFAULT
        };
        let plan = route(&request(1_000, Urgency::Standard, 100, None), &table).unwrap();
        assert_eq!(plan.action, RouteAction::Defer);
        assert_eq!(plan.reason, DecisionReason::FeeAboveThreshold);
        assert_eq!(plan.venue, Venue::Ethereum);
        assert_eq!(plan.fee, Credits(80));
        assert_eq!(plan.net, Credits(920));
    }

    #[test]
    fn escrow_locks_in_flight_and_conserves() {
        let who = did(2);
        let mut ledger = Ledger::new();
        ledger.mint(&who, Credits(10_000)).unwrap();
        let plan = route(
            &ExchangeRequest {
                from: who.clone(),
                amount: Credits(2_000),
                urgency: Urgency::Standard,
                slack_ticks: 100,
                preferred: None,
            },
            &RouteTable::DEFAULT,
        )
        .unwrap();
        let intent = escrow_for_route(&mut ledger, &plan, 7).unwrap();
        assert_eq!(ledger.balance(&who).available, Credits(8_000));
        assert_eq!(ledger.balance(&who).locked, Credits(2_000));
        ledger.check_conservation().unwrap();
        assert_eq!(intent.status, IntentStatus::AwaitingChainExecution);
        assert_eq!(intent.chain_execution, ChainExecution::PendingV18);
        assert!(!intent.on_chain_success());
        assert_eq!(intent.venue, "eth");
        assert_eq!(intent.fee, Credits(16));
        assert_eq!(intent.net, Credits(1_984));
        assert_eq!(intent.id, intent.fingerprint);
    }

    #[test]
    fn this_track_cannot_express_chain_success() {
        // 变体集合里没有 executed；执行状态恒为 false。
        let names: Vec<&str> = ChainExecution::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(
            names,
            vec!["pending_v1.8", "not_applicable", "not_requested"]
        );
        assert!(!names.contains(&"executed"));
        for c in ChainExecution::ALL {
            assert!(!c.executed());
        }
        assert_eq!(IntentStatus::ALL.len(), 1);
        assert_eq!(IntentStatus::ALL[0].as_str(), "awaiting_chain_execution");
    }

    #[test]
    fn cancelling_a_reservation_restores_the_balance_and_conserves() {
        let who = did(3);
        let mut ledger = Ledger::new();
        ledger.mint(&who, Credits(5_000)).unwrap();
        let plan = route(
            &ExchangeRequest {
                from: who.clone(),
                amount: Credits(1_000),
                urgency: Urgency::Standard,
                slack_ticks: 50,
                preferred: None,
            },
            &RouteTable::DEFAULT,
        )
        .unwrap();
        let intent = escrow_for_route(&mut ledger, &plan, 9).unwrap();
        let mut book = ExchangeBook::new();
        book.record(intent.clone());
        assert_eq!(book.pending(), 1);
        assert_eq!(book.escrowed_total().unwrap(), Credits(1_000));
        let back = cancel_intent(&mut ledger, &intent).unwrap();
        assert_eq!(back, Credits(1_000));
        book.mark_released(&intent.id, back, 11);
        assert_eq!(book.pending(), 0);
        assert_eq!(ledger.balance(&who).available, Credits(5_000));
        assert_eq!(ledger.balance(&who).locked, Credits::ZERO);
        ledger.check_conservation().unwrap();
        let json = book.to_json();
        assert_eq!(json["chain_executed"], json!(false));
    }

    #[test]
    fn internal_settlement_only_applies_to_internal_plans() {
        let (a, b) = (did(4), did(5));
        let mut ledger = Ledger::new();
        ledger.mint(&a, Credits(1_000)).unwrap();
        let ask = |amount: i64| ExchangeRequest {
            from: a.clone(),
            amount: Credits(amount),
            urgency: Urgency::Standard,
            slack_ticks: 100,
            preferred: None,
        };
        let internal = route(&ask(150), &RouteTable::DEFAULT).unwrap();
        assert_eq!(
            apply_internal(&mut ledger, &internal, &b).unwrap(),
            Credits(150)
        );
        assert_eq!(ledger.balance(&b).available, Credits(150));
        assert_eq!(ledger.balance(&a).available, Credits(850));
        ledger.check_conservation().unwrap();

        let onchain = route(&ask(1_000), &RouteTable::DEFAULT).unwrap();
        assert_eq!(
            apply_internal(&mut ledger, &onchain, &b),
            Err(CoreError::InvalidKind)
        );
        ledger.check_conservation().unwrap();
    }

    #[test]
    fn degenerate_requests_and_tables_are_refused() {
        assert_eq!(
            route(&request(0, Urgency::Standard, 100, None), &RouteTable::DEFAULT),
            Err(CoreError::ZeroAmount)
        );
        let bad = RouteTable {
            btc_fee_bp: 10_001,
            ..RouteTable::DEFAULT
        };
        assert_eq!(
            route(&request(1_000, Urgency::Standard, 100, None), &bad),
            Err(CoreError::InvalidKind)
        );
        let negative = RouteTable {
            eth_fee_bp: -1,
            ..RouteTable::DEFAULT
        };
        assert_eq!(
            route(&request(1_000, Urgency::Standard, 100, None), &negative),
            Err(CoreError::NegativeAmount)
        );
    }

    #[test]
    fn the_plan_is_content_addressed_and_float_free() {
        let plan =
            route(&request(1_234, Urgency::Expedited, 99, None), &RouteTable::DEFAULT).unwrap();
        let value = serde_json::to_value(plan.clone()).unwrap();
        let canonical = au4a_core::canonicalize(&value).unwrap();
        assert!(!canonical.contains('.'));
        assert_eq!(plan.fingerprint().unwrap(), plan.fingerprint().unwrap());
        // 预留凭证的 id 只依赖决策输入：同样的请求 → 同样的 id。
        let mut l1 = Ledger::new();
        l1.mint(plan.from(), Credits(10_000)).unwrap();
        let i1 = escrow_for_route(&mut l1, &plan, 42).unwrap();
        let mut l2 = Ledger::new();
        l2.mint(plan.from(), Credits(10_000)).unwrap();
        let i2 = escrow_for_route(&mut l2, &plan, 42).unwrap();
        assert_eq!(i1.id, i2.id);
    }
}
