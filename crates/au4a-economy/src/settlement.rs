//! v1.4.6 结算路由 + 收益归属资源提供者。
//!
//! 这一版把「钱怎么走」讲清楚，并且把人类的角色钉死在**只收收益**：
//!
//! * **结算路由决策表**：证据等级 / `cpu-proto` 上限 / 收款方是否有未结争议，三者决定
//!   `direct`（直接结算，过内核证据闸门）、`escrowed`（托管：先锁定，等争议了结）、
//!   `withheld`（拒付：证据不可结算）；
//! * **多资源提供者分成**：按基点权重用**最大余数法**整数拆分，Σ分账严格等于总额，
//!   不产生浮点尾差、不凭空多出或少掉 1 微积分；
//! * **收益归属**：[`Beneficiary::human_operator`] 的 [`DecisionRights`] 只能是
//!   `IncomeOnly`——人类操作者能收到 [`Receipt`]，但结构上没有任何提案 / 定价 / 投票 / 仲裁入口；
//! * 所有真实转账都走 `Kernel::settle`（证据闸门 + 守恒），托管走 `Ledger::lock`/`unlock`。

use au4a_core::{CoreError, CoreResult, Credits, Did, EvidenceGrade, Ledger};
use au4a_kernel::Kernel;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 资源提供者类别（收益归属给谁）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRole {
    /// 算力提供者。
    Compute,
    /// 数据提供者。
    Data,
    /// 技能 / 服务提供者。
    Skill,
}

impl ProviderRole {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderRole::Compute => "compute",
            ProviderRole::Data => "data",
            ProviderRole::Skill => "skill",
        }
    }
}

/// 收益主体的类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BeneficiaryKind {
    /// Agent 自己（有完整决策权）。
    Agent,
    /// 人类操作者（只收收益，不决策）。
    HumanOperator,
}

/// 决策权：这是「人类只观察」在类型上的表达。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionRights {
    /// Agent 主体：定价、质押、投票都由它自己发起。
    Agent,
    /// 收益主体：**只有**收收益这一件事。
    IncomeOnly,
}

/// 一个收益主体。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Beneficiary {
    pub did: Did,
    pub kind: BeneficiaryKind,
}

impl Beneficiary {
    pub fn agent(did: Did) -> Self {
        Self {
            did,
            kind: BeneficiaryKind::Agent,
        }
    }

    /// 人类操作者：只收收益。结构上没有配套的写方法——人类不能发起任何经济动作。
    pub fn human_operator(did: Did) -> Self {
        Self {
            did,
            kind: BeneficiaryKind::HumanOperator,
        }
    }

    pub fn decision_rights(&self) -> DecisionRights {
        match self.kind {
            BeneficiaryKind::Agent => DecisionRights::Agent,
            BeneficiaryKind::HumanOperator => DecisionRights::IncomeOnly,
        }
    }

    /// 人类操作者不能替 Agent 做经济决策；Agent 主体可以。
    pub fn may_decide(&self) -> bool {
        self.decision_rights() == DecisionRights::Agent
    }
}

/// 一条分成权重。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevenueShare {
    pub beneficiary: Did,
    pub share_bp: i64,
    pub role: ProviderRole,
    pub kind: BeneficiaryKind,
}

/// 一笔到账凭证（收益面板的最小数据单元）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub beneficiary: String,
    pub amount: Credits,
    pub share_bp: i64,
    pub role: ProviderRole,
    pub kind: BeneficiaryKind,
    pub at: u64,
}

/// 结算路由。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettlementRoute {
    /// 直接结算（过证据闸门）。
    Direct,
    /// 托管：先锁定，争议了结后再放款或退回。
    Escrowed,
    /// 拒付：证据不可结算。
    Withheld,
}

impl SettlementRoute {
    pub fn as_str(self) -> &'static str {
        match self {
            SettlementRoute::Direct => "direct",
            SettlementRoute::Escrowed => "escrowed",
            SettlementRoute::Withheld => "withheld",
        }
    }
}

/// 结算策略（网络底线 + Agent 自己的选择）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementPolicy {
    /// `cpu-proto` 证据的结算上限。
    pub cpu_proto_cap: Credits,
    /// 收款方有未结争议时是否托管。
    pub escrow_on_open_dispute: bool,
}

impl SettlementPolicy {
    pub const DEFAULT: SettlementPolicy = SettlementPolicy {
        cpu_proto_cap: Credits(100),
        escrow_on_open_dispute: true,
    };
}

impl Default for SettlementPolicy {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 一次结算请求。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementRequest {
    pub payer: Did,
    pub payee: Did,
    pub total: Credits,
    pub evidence: EvidenceGrade,
    /// 收款方当前是否有未结争议。
    pub dispute_open: bool,
    pub at: u64,
}

/// 路由决策结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementDecision {
    pub route: SettlementRoute,
    pub reason: String,
    pub grade: EvidenceGrade,
    pub amount: Credits,
}

/// 结算结果：付款 / 托管 / 拒付三选一（拒付与托管都不会产生收益）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettlementOutcome {
    Paid(Vec<Receipt>),
    Escrowed(SettlementDecision),
    Withheld(SettlementDecision),
}

/// 结算路由决策表（纯函数，顺序即优先级）。
///
/// | # | 条件 | 路由 | 理由码 |
/// |---|---|---|---|
/// | 0 | `total == 0` | 拒绝 `ZeroAmount` | — |
/// | 1 | 证据 `Unverified` | `withheld` | `evidence_unverified` |
/// | 2 | `cpu-proto` 且超上限 | `withheld` | `cpu_proto_cap` |
/// | 3 | 收款方有未结争议且策略要求托管 | `escrowed` | `payee_under_dispute` |
/// | 4 | 其余 | `direct` | `evidence_ok` |
pub fn route(
    request: &SettlementRequest,
    policy: &SettlementPolicy,
) -> CoreResult<SettlementDecision> {
    if request.total == Credits::ZERO {
        return Err(CoreError::ZeroAmount);
    }
    let decision = |route: SettlementRoute, reason: &str| SettlementDecision {
        route,
        reason: reason.to_string(),
        grade: request.evidence,
        amount: request.total,
    };
    if request.evidence == EvidenceGrade::Unverified {
        return Ok(decision(SettlementRoute::Withheld, "evidence_unverified"));
    }
    if request.evidence == EvidenceGrade::CpuProto && request.total > policy.cpu_proto_cap {
        return Ok(decision(SettlementRoute::Withheld, "cpu_proto_cap"));
    }
    if request.dispute_open && policy.escrow_on_open_dispute {
        return Ok(decision(SettlementRoute::Escrowed, "payee_under_dispute"));
    }
    Ok(decision(SettlementRoute::Direct, "evidence_ok"))
}

/// 最大余数法整数拆分：Σ结果 == `total`，且每一项先取整再按余数分配，确定性、无浮点。
///
/// 权重之和必须恰好是 10 000bp（一个整体），否则拒绝——不允许「分不完」这种模糊状态。
pub fn split_weights(total: Credits, weights_bp: &[i64]) -> CoreResult<Vec<Credits>> {
    if total == Credits::ZERO {
        return Err(CoreError::ZeroAmount);
    }
    if weights_bp.is_empty() {
        return Err(CoreError::InvalidKind);
    }
    let mut sum_w = 0i64;
    for w in weights_bp {
        if *w < 0 {
            return Err(CoreError::NegativeAmount);
        }
        if *w > 10_000 {
            return Err(CoreError::InvalidKind);
        }
        sum_w = sum_w.checked_add(*w).ok_or(CoreError::Overflow)?;
    }
    if sum_w != 10_000 {
        return Err(CoreError::InvalidKind);
    }

    let mut amounts = Vec::with_capacity(weights_bp.len());
    let mut remainders: Vec<(i64, usize)> = Vec::with_capacity(weights_bp.len());
    let mut allocated = 0i64;
    for (i, w) in weights_bp.iter().enumerate() {
        let product = total.get().checked_mul(*w).ok_or(CoreError::Overflow)?;
        let base = product / 10_000;
        amounts.push(base);
        allocated = allocated.checked_add(base).ok_or(CoreError::Overflow)?;
        remainders.push((product % 10_000, i));
    }
    let mut leftover = total.get().checked_sub(allocated).ok_or(CoreError::Overflow)?;
    // 余数大的先拿；余数相同按下标升序（确定性）。
    remainders.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut cursor = 0usize;
    while leftover > 0 && !remainders.is_empty() {
        let idx = remainders[cursor % remainders.len()].1;
        amounts[idx] = amounts[idx].checked_add(1).ok_or(CoreError::Overflow)?;
        leftover -= 1;
        cursor += 1;
    }
    amounts.into_iter().map(Credits::new).collect()
}

/// 直接结算（单笔）：走 `Kernel::settle`，证据闸门与守恒都在内核里强制。
pub fn settle_direct(
    kernel: &mut Kernel,
    from: &Did,
    to: &Did,
    amount: Credits,
    evidence: EvidenceGrade,
) -> CoreResult<()> {
    kernel.settle(from, to, amount, evidence)
}

/// 托管：把付款方的可用余额锁成在途（所有权未变，守恒不变）。
pub fn execute_escrow(ledger: &mut Ledger, from: &Did, amount: Credits) -> CoreResult<Credits> {
    ledger.lock(from, amount)?;
    ledger.check_conservation()?;
    Ok(amount)
}

/// 放款：解除托管并把钱真正转给收款方（争议判定付款方应支付时调用）。
pub fn release_escrow(
    ledger: &mut Ledger,
    from: &Did,
    to: &Did,
    amount: Credits,
) -> CoreResult<()> {
    ledger.unlock(from, amount)?;
    ledger.transfer(from, to, amount)?;
    ledger.check_conservation()
}

/// 退回：解除托管把钱还给付款方（争议判定不应支付时调用）。
pub fn refund_escrow(ledger: &mut Ledger, from: &Did, amount: Credits) -> CoreResult<Credits> {
    ledger.unlock(from, amount)?;
    ledger.check_conservation()?;
    Ok(amount)
}

/// 按权重把总额付给多个资源提供者：路由到 `direct` 才真正动账，其余返回对应结果。
///
/// 权重为 0 的提供者不产生凭证、也不会触发零额转账。
pub fn pay_split(
    kernel: &mut Kernel,
    request: &SettlementRequest,
    shares: &[RevenueShare],
    policy: &SettlementPolicy,
) -> CoreResult<SettlementOutcome> {
    if shares.is_empty() {
        return Err(CoreError::InvalidKind);
    }
    let decision = route(request, policy)?;
    match decision.route {
        SettlementRoute::Withheld => {
            kernel.refuse(
                &request.payer,
                au4a_core::RefusalCode::PolicyDenied,
                format!("withheld: {}", decision.reason),
            );
            return Ok(SettlementOutcome::Withheld(decision));
        }
        SettlementRoute::Escrowed => return Ok(SettlementOutcome::Escrowed(decision)),
        SettlementRoute::Direct => {}
    }
    let weights: Vec<i64> = shares.iter().map(|s| s.share_bp).collect();
    let amounts = split_weights(request.total, &weights)?;
    let mut receipts = Vec::new();
    for (share, amount) in shares.iter().zip(amounts) {
        if amount == Credits::ZERO {
            continue; // 0 权重：不产生凭证，也不做零额转账（那是非法调用）
        }
        settle_direct(
            kernel,
            &request.payer,
            &share.beneficiary,
            amount,
            request.evidence,
        )?;
        receipts.push(Receipt {
            beneficiary: share.beneficiary.as_str().to_string(),
            amount,
            share_bp: share.share_bp,
            role: share.role,
            kind: share.kind,
            at: request.at,
        });
    }
    Ok(SettlementOutcome::Paid(receipts))
}

/// 被拒付的记录。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WithheldRecord {
    pub payer: String,
    pub payee: String,
    pub amount: Credits,
    pub reason: String,
    pub at: u64,
}

/// 托管记录。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EscrowRecord {
    pub payer: String,
    pub amount: Credits,
    pub at: u64,
    /// 是否已经放款或退回。
    pub closed: bool,
}

/// 收益台账：只追加，顺序确定；收益面板的数据源之一。
#[derive(Clone, Debug, Default)]
pub struct RevenueBook {
    receipts: Vec<Receipt>,
    withheld: Vec<WithheldRecord>,
    escrows: Vec<EscrowRecord>,
}

impl RevenueBook {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_receipt(&mut self, receipt: Receipt) {
        self.receipts.push(receipt);
    }

    pub fn record_withheld(&mut self, record: WithheldRecord) {
        self.withheld.push(record);
    }

    pub fn record_escrow(&mut self, record: EscrowRecord) {
        self.escrows.push(record);
    }

    pub fn close_last_escrow(&mut self) {
        if let Some(last) = self.escrows.last_mut() {
            last.closed = true;
        }
    }

    pub fn receipts(&self) -> &[Receipt] {
        &self.receipts
    }

    pub fn withheld(&self) -> &[WithheldRecord] {
        &self.withheld
    }

    pub fn escrows(&self) -> &[EscrowRecord] {
        &self.escrows
    }

    /// 某个受益人的累计到账收益。
    pub fn earned(&self, who: &Did) -> CoreResult<Credits> {
        let key = who.as_str();
        let mut sum = Credits::ZERO;
        for r in &self.receipts {
            if r.beneficiary == key {
                sum = sum.checked_add(r.amount)?;
            }
        }
        Ok(sum)
    }

    pub fn total_earned(&self) -> CoreResult<Credits> {
        let mut sum = Credits::ZERO;
        for r in &self.receipts {
            sum = sum.checked_add(r.amount)?;
        }
        Ok(sum)
    }

    /// 只读 JSON 投影（监控面板数据源）。
    pub fn to_json(&self) -> Value {
        json!({
            "receipts": self.receipts,
            "withheld": self.withheld,
            "escrows": self.escrows,
            "total_earned": self.total_earned().unwrap_or_default(),
            "receipt_count": self.receipts.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use au4a_core::AgentKeys;
    use au4a_kernel::KernelConfig;

    fn did(seed: u8) -> Did {
        AgentKeys::from_seed(&[seed; 32]).did()
    }

    fn request(payer: &Did, payee: &Did, total: i64, evidence: EvidenceGrade, disputed: bool) -> SettlementRequest {
        SettlementRequest {
            payer: payer.clone(),
            payee: payee.clone(),
            total: Credits(total),
            evidence,
            dispute_open: disputed,
            at: 1,
        }
    }

    fn share(who: &Did, bp: i64, role: ProviderRole) -> RevenueShare {
        RevenueShare {
            beneficiary: who.clone(),
            share_bp: bp,
            role,
            kind: BeneficiaryKind::Agent,
        }
    }

    #[test]
    fn the_decision_table_is_pinned() {
        let (payer, payee) = (did(1), did(2));
        let policy = SettlementPolicy::DEFAULT;
        assert_eq!(
            route(&request(&payer, &payee, 0, EvidenceGrade::Verified, false), &policy),
            Err(CoreError::ZeroAmount)
        );
        let withheld = route(
            &request(&payer, &payee, 10, EvidenceGrade::Unverified, false),
            &policy,
        )
        .unwrap();
        assert_eq!(withheld.route, SettlementRoute::Withheld);
        assert_eq!(withheld.reason, "evidence_unverified");
        let capped = route(
            &request(&payer, &payee, 101, EvidenceGrade::CpuProto, false),
            &policy,
        )
        .unwrap();
        assert_eq!(capped.route, SettlementRoute::Withheld);
        assert_eq!(capped.reason, "cpu_proto_cap");
        let escrowed = route(
            &request(&payer, &payee, 50, EvidenceGrade::Verified, true),
            &policy,
        )
        .unwrap();
        assert_eq!(escrowed.route, SettlementRoute::Escrowed);
        assert_eq!(escrowed.reason, "payee_under_dispute");
        let direct = route(
            &request(&payer, &payee, 50, EvidenceGrade::CpuProto, false),
            &policy,
        )
        .unwrap();
        assert_eq!(direct.route, SettlementRoute::Direct);
        assert_eq!(direct.reason, "evidence_ok");
    }

    #[test]
    fn the_largest_remainder_split_is_exact() {
        // 10 分给 3333/3333/3334：先各取 3，剩 1 给余数最大的第三份。
        let parts = split_weights(Credits(10), &[3_333, 3_333, 3_334]).unwrap();
        assert_eq!(parts, vec![Credits(3), Credits(3), Credits(4)]);
        let sum: i64 = parts.iter().map(|c| c.get()).sum();
        assert_eq!(sum, 10);
        // 100 → 33/33/34；1000 → 333/333/334。
        assert_eq!(
            split_weights(Credits(100), &[3_333, 3_333, 3_334]).unwrap(),
            vec![Credits(33), Credits(33), Credits(34)]
        );
        assert_eq!(
            split_weights(Credits(1_000), &[3_333, 3_333, 3_334]).unwrap(),
            vec![Credits(333), Credits(333), Credits(334)]
        );
        // 三分之一分法在任意总额下都不丢钱。
        for total in 1..500i64 {
            let parts = split_weights(Credits(total), &[3_333, 3_333, 3_334]).unwrap();
            let sum: i64 = parts.iter().map(|c| c.get()).sum();
            assert_eq!(sum, total, "total={total}");
        }
    }

    #[test]
    fn a_split_must_add_up_to_the_whole() {
        assert_eq!(
            split_weights(Credits(100), &[5_000, 4_999]),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(
            split_weights(Credits(100), &[]),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(
            split_weights(Credits(100), &[10_001, -1]),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(
            split_weights(Credits(100), &[-1, 10_000]),
            Err(CoreError::NegativeAmount)
        );
        assert_eq!(
            split_weights(Credits(0), &[10_000]),
            Err(CoreError::ZeroAmount)
        );
    }

    #[test]
    fn direct_settlement_moves_funds_through_the_evidence_gate() {
        let mut kernel = Kernel::new(KernelConfig::default());
        let (payer_keys, p1_keys, p2_keys) = (AgentKeys::from_seed(&[1; 32]), AgentKeys::from_seed(&[2; 32]), AgentKeys::from_seed(&[3; 32]));
        kernel.register(&payer_keys, "payer", &["buy"], Credits(10)).unwrap();
        kernel.register(&p1_keys, "p1", &["compute"], Credits(10)).unwrap();
        kernel.register(&p2_keys, "p2", &["data"], Credits(10)).unwrap();
        let shares = vec![
            share(&p1_keys.did(), 7_000, ProviderRole::Compute),
            share(&p2_keys.did(), 3_000, ProviderRole::Data),
        ];
        let outcome = pay_split(
            &mut kernel,
            &request(&payer_keys.did(), &p1_keys.did(), 200, EvidenceGrade::Verified, false),
            &shares,
            &SettlementPolicy::DEFAULT,
        )
        .unwrap();
        match outcome {
            SettlementOutcome::Paid(receipts) => {
                assert_eq!(receipts.len(), 2);
                assert_eq!(receipts[0].amount, Credits(140)); // 200 × 70%
                assert_eq!(receipts[1].amount, Credits(60)); // 200 × 30%
                assert_eq!(receipts[0].role, ProviderRole::Compute);
            }
            other => panic!("期望直接结算，实际 {other:?}"),
        }
        assert_eq!(kernel.ledger().balance(&payer_keys.did()).available, Credits(790));
        assert_eq!(kernel.ledger().balance(&p1_keys.did()).available, Credits(1_130));
        assert_eq!(kernel.ledger().balance(&p2_keys.did()).available, Credits(1_050));
        kernel.ledger().check_conservation().unwrap();
    }

    #[test]
    fn withheld_and_escrowed_do_not_move_money() {
        let mut kernel = Kernel::new(KernelConfig::default());
        let payer_keys = AgentKeys::from_seed(&[4; 32]);
        let payee_keys = AgentKeys::from_seed(&[5; 32]);
        kernel.register(&payer_keys, "payer", &["buy"], Credits(10)).unwrap();
        kernel.register(&payee_keys, "payee", &["skill"], Credits(10)).unwrap();
        let shares = vec![share(&payee_keys.did(), 10_000, ProviderRole::Skill)];
        let before = kernel.ledger().balance(&payer_keys.did());

        // Unverified → 拒付，账本不动，并留下类型化拒绝记录。
        let outcome = pay_split(
            &mut kernel,
            &request(&payer_keys.did(), &payee_keys.did(), 50, EvidenceGrade::Unverified, false),
            &shares,
            &SettlementPolicy::DEFAULT,
        )
        .unwrap();
        assert!(matches!(outcome, SettlementOutcome::Withheld(_)));
        assert_eq!(kernel.ledger().balance(&payer_keys.did()), before);
        assert_eq!(kernel.refusals().len(), 1);
        assert_eq!(kernel.refusals()[0].1.code, au4a_core::RefusalCode::PolicyDenied);

        // 收款方有未结争议 → 托管，同样不动账本。
        let outcome = pay_split(
            &mut kernel,
            &request(&payer_keys.did(), &payee_keys.did(), 50, EvidenceGrade::Verified, true),
            &shares,
            &SettlementPolicy::DEFAULT,
        )
        .unwrap();
        assert!(matches!(outcome, SettlementOutcome::Escrowed(_)));
        assert_eq!(kernel.ledger().balance(&payer_keys.did()), before);
        kernel.ledger().check_conservation().unwrap();
    }

    #[test]
    fn escrow_can_be_released_or_refunded() {
        let (payer, payee) = (did(6), did(7));
        let mut ledger = Ledger::new();
        ledger.mint(&payer, Credits(1_000)).unwrap();

        execute_escrow(&mut ledger, &payer, Credits(300)).unwrap();
        assert_eq!(ledger.balance(&payer).locked, Credits(300));
        refund_escrow(&mut ledger, &payer, Credits(300)).unwrap();
        assert_eq!(ledger.balance(&payer).available, Credits(1_000));
        assert_eq!(ledger.balance(&payer).locked, Credits::ZERO);

        execute_escrow(&mut ledger, &payer, Credits(300)).unwrap();
        release_escrow(&mut ledger, &payer, &payee, Credits(300)).unwrap();
        assert_eq!(ledger.balance(&payee).available, Credits(300));
        assert_eq!(ledger.balance(&payer).available, Credits(700));
        assert_eq!(ledger.balance(&payer).locked, Credits::ZERO);
        ledger.check_conservation().unwrap();
    }

    #[test]
    fn a_human_operator_only_receives_income() {
        let human = Beneficiary::human_operator(did(8));
        let agent = Beneficiary::agent(did(9));
        assert_eq!(human.decision_rights(), DecisionRights::IncomeOnly);
        assert!(!human.may_decide());
        assert_eq!(agent.decision_rights(), DecisionRights::Agent);
        assert!(agent.may_decide());

        let mut book = RevenueBook::new();
        book.record_receipt(Receipt {
            beneficiary: human.did.as_str().to_string(),
            amount: Credits(42),
            share_bp: 10_000,
            role: ProviderRole::Compute,
            kind: BeneficiaryKind::HumanOperator,
            at: 7,
        });
        assert_eq!(book.earned(&human.did).unwrap(), Credits(42));
        assert_eq!(book.total_earned().unwrap(), Credits(42));
        let value = book.to_json();
        let canonical = au4a_core::canonicalize(&value).unwrap();
        assert!(!canonical.contains('.'));
        assert_eq!(value["receipts"][0]["kind"], json!("human_operator"));
    }

    #[test]
    fn zero_weight_providers_get_no_receipt() {
        let mut kernel = Kernel::new(KernelConfig::default());
        let payer_keys = AgentKeys::from_seed(&[10; 32]);
        let p1_keys = AgentKeys::from_seed(&[11; 32]);
        let p2_keys = AgentKeys::from_seed(&[12; 32]);
        kernel.register(&payer_keys, "payer", &["buy"], Credits(10)).unwrap();
        kernel.register(&p1_keys, "p1", &["compute"], Credits(10)).unwrap();
        kernel.register(&p2_keys, "p2", &["data"], Credits(10)).unwrap();
        let shares = vec![
            share(&p1_keys.did(), 10_000, ProviderRole::Compute),
            share(&p2_keys.did(), 0, ProviderRole::Data),
        ];
        let outcome = pay_split(
            &mut kernel,
            &request(&payer_keys.did(), &p1_keys.did(), 60, EvidenceGrade::Verified, false),
            &shares,
            &SettlementPolicy::DEFAULT,
        )
        .unwrap();
        match outcome {
            SettlementOutcome::Paid(receipts) => {
                assert_eq!(receipts.len(), 1);
                assert_eq!(receipts[0].amount, Credits(60));
            }
            other => panic!("期望直接结算，实际 {other:?}"),
        }
        assert_eq!(kernel.ledger().balance(&p2_keys.did()).available, Credits(990));
        // 空分成表是调用错误。
        assert_eq!(
            pay_split(
                &mut kernel,
                &request(&payer_keys.did(), &p1_keys.did(), 60, EvidenceGrade::Verified, false),
                &[],
                &SettlementPolicy::DEFAULT,
            ),
            Err(CoreError::InvalidKind)
        );
    }

    #[test]
    fn the_revenue_book_tracks_withheld_and_escrow_lifecycle() {
        let mut book = RevenueBook::new();
        book.record_withheld(WithheldRecord {
            payer: did(13).as_str().to_string(),
            payee: did(14).as_str().to_string(),
            amount: Credits(10),
            reason: "evidence_unverified".to_string(),
            at: 1,
        });
        book.record_escrow(EscrowRecord {
            payer: did(13).as_str().to_string(),
            amount: Credits(20),
            at: 2,
            closed: false,
        });
        book.close_last_escrow();
        assert_eq!(book.escrows()[0].closed, true);
        assert_eq!(book.withheld().len(), 1);
        assert_eq!(book.total_earned().unwrap(), Credits::ZERO);
        assert_eq!(book.to_json()["receipt_count"], json!(0));
    }
}
