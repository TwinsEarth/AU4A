//! v1.2.7 — 仲裁接入。
//!
//! 仲裁不是「请人类来评理」，而是一条**数据契约 + 两个 Agent 的签名**：
//!
//! ```text
//! BreachClaim + 双签合约  ──file──▶  ArbitrationCase（case_id 内容寻址，两名仲裁员）
//!                                          │
//!                         ArbitrationPolicy.assess（纯整数规则，无人类裁量）
//!                                          ▼
//!                                     Ruling（恰好两名仲裁员签名）
//!                                          │
//!                      ledger.slash（罚没销毁）+ kernel.settle（赔付，过证据闸门）
//!                                          ▼
//!                                  Enforcement + 守恒断言
//! ```
//!
//! 三条硬约束：
//!
//! 1. **仲裁员必须是第三方**：当事人不能审自己的案子，两名人选必须互不相同。
//! 2. **裁决必须双签**：`Ruling` 的签名集合必须恰好等于本案的两名仲裁员，缺一不生效。
//! 3. **钱必须走冻结路径**：罚没用 `Ledger::slash`（只动锁定质押、销毁不转移），
//!    赔付用 `Kernel::settle`（先过证据闸门：`unverified` 永远不可结算）。
//!    执行前后都断言 `check_conservation()`。

use au4a_core::{
    canonical_hash, canonicalize, short_id, AgentKeys, CoreError, CoreResult, Credits, Did,
    EvidenceGrade,
};
use au4a_kernel::Kernel;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::breach::BreachClaim;
use crate::contract::Contract;
use crate::msg::MAX_NOTE;
use crate::state::PartySignature;

/// 仲裁结论。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// 违约成立。
    Upheld,
    /// 证据不足或申诉不成立。
    Rejected,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Upheld => "upheld",
            Verdict::Rejected => "rejected",
        }
    }
}

/// 仲裁政策：把「证据等级」映射成罚没/赔付额度，全部整数基点运算，没有人类裁量。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArbitrationPolicy {
    /// 罚没基点（万分比），从被诉方的锁定质押中销毁。
    pub slash_bp: i64,
    /// 赔付基点（万分比），从被诉方可用余额转给申诉方。
    pub compensate_bp: i64,
}

impl Default for ArbitrationPolicy {
    fn default() -> Self {
        Self {
            slash_bp: 2_000,
            compensate_bp: 5_000,
        }
    }
}

impl ArbitrationPolicy {
    pub fn new(slash_bp: i64, compensate_bp: i64) -> CoreResult<Self> {
        if slash_bp < 0 || compensate_bp < 0 {
            return Err(CoreError::NegativeAmount);
        }
        Ok(Self {
            slash_bp,
            compensate_bp,
        })
    }

    /// 判定：证据等级决定结论，合约金额与基点决定金额。
    ///
    /// `Unverified` 的申诉一律 `Rejected`——不是因为「看起来假」，
    /// 而是因为基元层的规则是 `unverified` 永远不可结算（见 [`EvidenceGrade::settleable`]）。
    pub fn assess(
        &self,
        claim: &BreachClaim,
        price: Credits,
    ) -> CoreResult<(Verdict, Credits, Credits)> {
        claim.verify()?;
        if claim.evidence == EvidenceGrade::Unverified {
            return Ok((Verdict::Rejected, Credits::ZERO, Credits::ZERO));
        }
        Ok((
            Verdict::Upheld,
            price.scaled_bp(self.slash_bp)?,
            price.scaled_bp(self.compensate_bp)?,
        ))
    }
}

/// 裁决：两名仲裁员对结论、金额与理由的共同签名。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ruling {
    pub case_id: String,
    pub verdict: Verdict,
    /// 名义罚没额（实际销毁以锁定余额为上限）。
    pub slash: Credits,
    /// 名义赔付额（实际转账以可用余额与证据闸门上限为准）。
    pub compensate: Credits,
    pub rationale: String,
    pub at: u64,
    /// 恰好两名仲裁员的签名，按 DID 排序。
    pub signatures: Vec<PartySignature>,
}

impl Ruling {
    pub fn payload(&self) -> Value {
        json!({
            "case_id": self.case_id,
            "verdict": self.verdict,
            "slash": self.slash,
            "compensate": self.compensate,
            "rationale": self.rationale,
            "at": self.at,
        })
    }

    /// 签名必须恰好覆盖给定的两名仲裁员，且每个签名真的覆盖了载荷。
    pub fn verify_against(&self, arbiters: &[Did]) -> CoreResult<()> {
        if arbiters.len() != 2 || arbiters[0] == arbiters[1] {
            return Err(CoreError::InvalidKind);
        }
        if self.signatures.len() != 2 {
            return Err(CoreError::NotSealed);
        }
        let bytes = canonicalize(&self.payload())?;
        for arbiter in arbiters {
            let signature = self
                .signatures
                .iter()
                .find(|s| &s.did == arbiter)
                .ok_or(CoreError::NotSealed)?;
            arbiter.verify(bytes.as_bytes(), &signature.sig)?;
        }
        if self
            .signatures
            .iter()
            .any(|s| !arbiters.contains(&s.did))
        {
            return Err(CoreError::UnknownAgent);
        }
        Ok(())
    }

    pub fn summary(&self) -> Value {
        json!({
            "case_id": self.case_id,
            "verdict": self.verdict.as_str(),
            "slash": self.slash.0,
            "compensate": self.compensate.0,
            "rationale": self.rationale,
            "signatures": self.signatures.len(),
            "arbiters": self.signatures.iter().map(|s| s.did.as_str()).collect::<Vec<_>>(),
        })
    }
}

/// 执行结果（对账用）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Enforcement {
    pub case_id: String,
    pub verdict: Verdict,
    /// 实际销毁（受锁定余额上限约束）。
    pub slashed: Credits,
    /// 实际赔付（受可用余额与证据闸门约束）。
    pub compensated: Credits,
    /// 执行后守恒断言是否通过。
    pub conservation_ok: bool,
    pub at: u64,
}

impl Enforcement {
    pub fn summary(&self) -> Value {
        json!({
            "case_id": self.case_id,
            "verdict": self.verdict.as_str(),
            "slashed": self.slashed.0,
            "compensated": self.compensated.0,
            "conservation_ok": self.conservation_ok,
        })
    }
}

/// 一个仲裁案件。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArbitrationCase {
    /// 内容寻址：`hash(合约哈希 + 申诉载荷)`。
    pub case_id: String,
    pub contract_id: String,
    pub contract_hash: String,
    pub claimant: Did,
    pub accused: Did,
    pub claim: BreachClaim,
    pub filed_at: u64,
    /// 两名仲裁员（第三方）。
    pub arbiters: Vec<Did>,
    pub ruling: Option<Ruling>,
}

impl ArbitrationCase {
    /// 立案：申诉必须挂在一条双签合约上，仲裁员必须是两个互不相同的第三方。
    pub fn file(
        claim: &BreachClaim,
        contract: &Contract,
        arbiters: &[Did],
        at: u64,
    ) -> CoreResult<Self> {
        contract.verify()?;
        claim.verify()?;
        if !claim.is_against(contract) || !claim.accused_is_counterparty(contract) {
            // 申诉与合约对不上，或者被诉方不是合约另一方。
            return Err(CoreError::InvalidKind);
        }
        if arbiters.len() != 2 || arbiters[0] == arbiters[1] {
            return Err(CoreError::InvalidKind);
        }
        for arbiter in arbiters {
            if *arbiter == claim.claimant || *arbiter == claim.accused {
                // 当事人不能审自己的案子。
                return Err(CoreError::UnknownAgent);
            }
        }
        let case_id = canonical_hash(&json!({
            "contract_hash": claim.contract_hash,
            "claim": claim.payload(),
        }))?;
        Ok(Self {
            case_id,
            contract_id: claim.contract_id.clone(),
            contract_hash: claim.contract_hash.clone(),
            claimant: claim.claimant.clone(),
            accused: claim.accused.clone(),
            claim: claim.clone(),
            filed_at: at,
            arbiters: arbiters.to_vec(),
            ruling: None,
        })
    }

    pub fn case_id(&self) -> &str {
        self.case_id.as_str()
    }

    pub fn short_id(&self) -> String {
        short_id(&self.case_id)
    }

    pub fn arbiters(&self) -> &[Did] {
        &self.arbiters
    }

    pub fn ruling(&self) -> Option<&Ruling> {
        self.ruling.as_ref()
    }

    /// 裁决：必须由**本案的两名仲裁员**共同签署，缺一不生效。
    pub fn rule(
        &mut self,
        policy: &ArbitrationPolicy,
        arbiters: &[&AgentKeys],
        price: Credits,
        rationale: &str,
        at: u64,
    ) -> CoreResult<Ruling> {
        if arbiters.len() != 2 {
            return Err(CoreError::InvalidKind);
        }
        if rationale.len() > MAX_NOTE {
            return Err(CoreError::InvalidKind);
        }
        let mut seen: Vec<Did> = Vec::new();
        for keys in arbiters {
            let did = keys.did();
            if !self.arbiters.contains(&did) {
                // 不在本案仲裁员名单里的 Agent 无权裁决。
                return Err(CoreError::UnknownAgent);
            }
            if seen.contains(&did) {
                return Err(CoreError::InvalidSignature);
            }
            seen.push(did);
        }
        if seen.len() != self.arbiters.len() {
            return Err(CoreError::NotSealed);
        }

        let (verdict, slash, compensate) = policy.assess(&self.claim, price)?;
        let mut ruling = Ruling {
            case_id: self.case_id.clone(),
            verdict,
            slash,
            compensate,
            rationale: rationale.to_string(),
            at,
            signatures: Vec::new(),
        };
        for keys in arbiters {
            let sig = keys.sign_json(&ruling.payload())?;
            ruling.signatures.push(PartySignature {
                did: keys.did(),
                sig,
            });
        }
        ruling.signatures.sort_by(|a, b| a.did.cmp(&b.did));
        ruling.verify_against(&self.arbiters)?;
        self.ruling = Some(ruling.clone());
        Ok(ruling)
    }

    /// 复核已有裁决。
    pub fn verify_ruling(&self) -> CoreResult<()> {
        let ruling = self.ruling.as_ref().ok_or(CoreError::NotSealed)?;
        if ruling.case_id != self.case_id {
            return Err(CoreError::InvalidSignature);
        }
        ruling.verify_against(&self.arbiters)
    }

    /// 执行裁决：罚没（`Ledger::slash`）+ 赔付（`Kernel::settle`，过证据闸门）。
    ///
    /// 所有检查都在动账之前完成，因此要么整案执行，要么账本完全不动。
    pub fn enforce(&mut self, kernel: &mut Kernel, at: u64) -> CoreResult<Enforcement> {
        self.verify_ruling()?;
        let ruling = self.ruling.clone().ok_or(CoreError::NotSealed)?;

        // 先查账本守恒，再查证据闸门——检查都在写之前。
        kernel.ledger().check_conservation()?;
        let available = kernel.ledger().balance(&self.accused).available;
        let nominal = ruling.compensate;
        let effective = if nominal > available { available } else { nominal };
        if ruling.verdict == Verdict::Upheld
            && !self.claim.evidence.settleable(effective, kernel.config().cpu_proto_settle_cap)
        {
            // 证据等级不足或超出原型结算上限：整案拒绝执行，不动账本。
            kernel.refuse(
                &self.claimant,
                au4a_core::RefusalCode::PolicyDenied,
                "evidence gate refused arbitration payout",
            );
            return Err(CoreError::InsufficientFunds);
        }

        let slashed_before = kernel.ledger().slashed();
        let mut compensated = Credits::ZERO;
        if ruling.verdict == Verdict::Upheld {
            if ruling.slash > Credits::ZERO {
                // 罚没从锁定质押里销毁；`slash` 自身以锁定余额为上限。
                kernel.ledger_mut().slash(&self.accused, ruling.slash)?;
            }
            if effective > Credits::ZERO {
                // 赔付走内核结算路径：证据闸门与余额检查都在里面。
                kernel.settle(&self.accused, &self.claimant, effective, self.claim.evidence)?;
                compensated = effective;
            }
        }
        let slashed = kernel.ledger().slashed().checked_sub(slashed_before)?;
        kernel.ledger().check_conservation()?;
        kernel.emit(
            "arbitration.enforced",
            format!(
                "case={} verdict={} slashed={} compensated={}",
                self.short_id(),
                ruling.verdict.as_str(),
                slashed,
                compensated
            ),
        );
        Ok(Enforcement {
            case_id: self.case_id.clone(),
            verdict: ruling.verdict,
            slashed,
            compensated,
            conservation_ok: true,
            at,
        })
    }

    pub fn summary(&self) -> Value {
        json!({
            "case_id": self.case_id,
            "short_id": self.short_id(),
            "contract_id": self.contract_id,
            "claimant": self.claimant.as_str(),
            "accused": self.accused.as_str(),
            "kind": self.claim.kind.as_str(),
            "evidence": self.claim.evidence.as_str(),
            "arbiters": self.arbiters.iter().map(|d| d.as_str()).collect::<Vec<_>>(),
            "ruling": self.ruling.as_ref().map(|r| r.summary()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use au4a_core::EvidenceGrade;
    use au4a_kernel::KernelConfig;

    fn agent(seed: u8) -> AgentKeys {
        AgentKeys::from_seed(&[seed; 32])
    }

    fn terms(price: i64) -> crate::msg::Terms {
        crate::msg::Terms::new("summarize.zh", Credits(price), 40, EvidenceGrade::Verified).unwrap()
    }

    /// 双签合约 + 一条有效申诉 + 两名仲裁员，全部离线构造。
    fn case_with(price: i64, evidence: EvidenceGrade) -> (ArbitrationCase, AgentKeys, AgentKeys) {
        let client = agent(1);
        let provider = agent(2);
        let arb1 = agent(3);
        let arb2 = agent(4);
        let mut contract =
            Contract::draft(&client, &provider.did(), &terms(price), "session-arb", 1).unwrap();
        contract.sign(&client).unwrap();
        contract.sign(&provider).unwrap();
        let claim = BreachClaim::file(
            &client,
            &contract,
            crate::msg::BreachKind::NonDelivery,
            evidence,
            "nothing arrived",
            2,
        )
        .unwrap();
        let case =
            ArbitrationCase::file(&claim, &contract, &[arb1.did(), arb2.did()], 3).unwrap();
        (case, arb1, arb2)
    }

    fn registered_kernel(client: &AgentKeys, provider: &AgentKeys) -> Kernel {
        let mut k = Kernel::new(KernelConfig::default());
        k.register(client, "client", &["summarize.zh"], Credits(50)).unwrap();
        k.register(provider, "provider", &["summarize.zh"], Credits(50)).unwrap();
        k
    }

    #[test]
    fn a_case_is_content_addressed_and_bound_to_the_contract() {
        let (case, _, _) = case_with(100, EvidenceGrade::Verified);
        assert_eq!(case.case_id.len(), 64);
        assert_eq!(case.short_id().len(), 16);
        assert_eq!(case.contract_hash, case.claim.contract_hash);
        assert_eq!(case.arbiters().len(), 2);
        assert!(case.ruling().is_none());
        assert_eq!(case.summary()["arbiters"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn parties_and_duplicate_arbiters_are_refused() {
        let client = agent(1);
        let provider = agent(2);
        let mut contract =
            Contract::draft(&client, &provider.did(), &terms(100), "s", 1).unwrap();
        contract.sign(&client).unwrap();
        contract.sign(&provider).unwrap();
        let claim = BreachClaim::file(
            &client,
            &contract,
            crate::msg::BreachKind::LateDelivery,
            EvidenceGrade::Verified,
            "late",
            2,
        )
        .unwrap();

        // 当事人当仲裁员：拒绝。
        assert_eq!(
            ArbitrationCase::file(&claim, &contract, &[client.did(), agent(3).did()], 3),
            Err(CoreError::UnknownAgent)
        );
        // 同一人两名：拒绝。
        assert_eq!(
            ArbitrationCase::file(&claim, &contract, &[agent(3).did(), agent(3).did()], 3),
            Err(CoreError::InvalidKind)
        );
        // 只有一名仲裁员：拒绝。
        assert_eq!(
            ArbitrationCase::file(&claim, &contract, &[agent(3).did()], 3),
            Err(CoreError::InvalidKind)
        );
        // 第三方两名：接受。
        assert!(ArbitrationCase::file(&claim, &contract, &[agent(3).did(), agent(4).did()], 3).is_ok());
    }

    #[test]
    fn a_ruling_needs_both_arbiters_and_nobody_else() {
        let (mut case, arb1, arb2) = case_with(100, EvidenceGrade::Verified);
        let outsider = agent(9);
        let policy = ArbitrationPolicy::default();

        // 只有一名仲裁员签名：不生效。
        assert_eq!(
            case.rule(&policy, &[&arb1], Credits(100), "one signature", 4),
            Err(CoreError::InvalidKind)
        );
        // 外部 Agent 混进来：无权限。
        assert_eq!(
            case.rule(&policy, &[&arb1, &outsider], Credits(100), "outsider", 4),
            Err(CoreError::UnknownAgent)
        );
        // 两名仲裁员都签：成立。
        let ruling = case
            .rule(&policy, &[&arb1, &arb2], Credits(100), "provider never delivered", 4)
            .unwrap();
        assert_eq!(ruling.verdict, Verdict::Upheld);
        assert_eq!(ruling.signatures.len(), 2);
        assert_eq!(ruling.slash, Credits(20), "100 的 2000bp = 20");
        assert_eq!(ruling.compensate, Credits(50), "100 的 5000bp = 50");
        case.verify_ruling().unwrap();

        // 篡改裁决金额 → 复核失败。
        let mut tampered = case.clone();
        if let Some(target) = tampered.ruling.as_mut() {
            target.compensate = Credits(999);
        }
        assert_eq!(tampered.verify_ruling(), Err(CoreError::InvalidSignature));

        // 一人代两人签名 → 复核失败。
        let mut single = case.clone();
        if let Some(target) = single.ruling.as_mut() {
            target.signatures.pop();
        }
        assert_eq!(single.verify_ruling(), Err(CoreError::NotSealed));
    }

    #[test]
    fn unverified_claims_are_rejected_by_policy_with_zero_amounts() {
        let (mut case, arb1, arb2) = case_with(100, EvidenceGrade::Unverified);
        let ruling = case
            .rule(
                &ArbitrationPolicy::default(),
                &[&arb1, &arb2],
                Credits(100),
                "no evidence",
                4,
            )
            .unwrap();
        assert_eq!(ruling.verdict, Verdict::Rejected);
        assert_eq!(ruling.slash, Credits::ZERO);
        assert_eq!(ruling.compensate, Credits::ZERO);
    }

    #[test]
    fn enforcement_slashes_from_stake_and_pays_through_the_kernel() {
        let (mut case, arb1, arb2) = case_with(100, EvidenceGrade::Verified);
        let client = AgentKeys::from_seed(&[1u8; 32]);
        let provider = AgentKeys::from_seed(&[2u8; 32]);
        let mut k = registered_kernel(&client, &provider);
        let before = k.ledger().view();

        case.rule(
            &ArbitrationPolicy::default(),
            &[&arb1, &arb2],
            Credits(100),
            "non-delivery proven",
            5,
        )
        .unwrap();
        let report = case.enforce(&mut k, 6).unwrap();

        assert_eq!(report.verdict, Verdict::Upheld);
        assert_eq!(report.slashed, Credits(20), "罚没 20 微积分被销毁");
        assert_eq!(report.compensated, Credits(50), "赔付 50 微积分给申诉方");
        assert!(report.conservation_ok);
        k.ledger().check_conservation().unwrap();

        // 罚没销毁：被诉方锁定 50 → 30；发行量不变，销毁量 +20。
        let after = k.ledger().view();
        assert_eq!(after.slashed, before.slashed.checked_add(Credits(20)).unwrap());
        assert_eq!(after.minted, before.minted);
        let accused_before = before.accounts.get(provider.did().as_str()).cloned().unwrap_or_default();
        let accused_after = after.accounts.get(provider.did().as_str()).cloned().unwrap_or_default();
        assert_eq!(accused_after.locked.get(), accused_before.locked.get() - 20);
        assert_eq!(accused_after.available.get(), accused_before.available.get() - 50);
        // 赔付到账：申诉方可用 +50。
        let claimant_before = before.accounts.get(client.did().as_str()).cloned().unwrap_or_default();
        let claimant_after = after.accounts.get(client.did().as_str()).cloned().unwrap_or_default();
        assert_eq!(claimant_after.available.get(), claimant_before.available.get() + 50);

        // 执行过的事件进进度流。
        assert!(k
            .observe()
            .progress
            .iter()
            .any(|p| p.kind == "arbitration.enforced" && p.detail.contains(&case.short_id())));
    }

    #[test]
    fn enforcement_is_all_or_nothing_when_the_evidence_gate_refuses() {
        // cpu-proto 证据的赔付 500 超过默认原型结算上限 100：整案不执行。
        let (mut case, arb1, arb2) = case_with(1_000, EvidenceGrade::CpuProto);
        let client = AgentKeys::from_seed(&[1u8; 32]);
        let provider = AgentKeys::from_seed(&[2u8; 32]);
        let mut k = registered_kernel(&client, &provider);
        let before = k.ledger().view();

        let ruling = case
            .rule(
                &ArbitrationPolicy::default(),
                &[&arb1, &arb2],
                Credits(1_000),
                "cpu-proto evidence, large payout",
                5,
            )
            .unwrap();
        assert_eq!(ruling.verdict, Verdict::Upheld);
        assert_eq!(ruling.compensate, Credits(500));
        assert_eq!(case.enforce(&mut k, 6), Err(CoreError::InsufficientFunds));

        // 账本一动没动：罚没也没有执行。
        let after = k.ledger().view();
        assert_eq!(after.slashed, before.slashed);
        assert_eq!(after.minted, before.minted);
        assert_eq!(after.accounts, before.accounts);
        k.ledger().check_conservation().unwrap();
        assert_eq!(
            k.refusals().last().unwrap().1.code,
            au4a_core::RefusalCode::PolicyDenied
        );
    }

    #[test]
    fn a_rejected_verdict_moves_no_money() {
        let (mut case, arb1, arb2) = case_with(100, EvidenceGrade::Unverified);
        let client = AgentKeys::from_seed(&[1u8; 32]);
        let provider = AgentKeys::from_seed(&[2u8; 32]);
        let mut k = registered_kernel(&client, &provider);
        let before = k.ledger().view();
        case.rule(&ArbitrationPolicy::default(), &[&arb1, &arb2], Credits(100), "no proof", 5)
            .unwrap();
        let report = case.enforce(&mut k, 6).unwrap();
        assert_eq!(report.verdict, Verdict::Rejected);
        assert_eq!(report.slashed, Credits::ZERO);
        assert_eq!(report.compensated, Credits::ZERO);
        assert_eq!(report.conservation_ok, true);
        let after = k.ledger().view();
        assert_eq!(after.accounts, before.accounts);
        assert_eq!(after.slashed, before.slashed);
    }

    #[test]
    fn enforcement_needs_a_ruling_first() {
        let (mut case, _, _) = case_with(100, EvidenceGrade::Verified);
        let client = AgentKeys::from_seed(&[1u8; 32]);
        let provider = AgentKeys::from_seed(&[2u8; 32]);
        let mut k = registered_kernel(&client, &provider);
        assert_eq!(case.verify_ruling(), Err(CoreError::NotSealed));
        assert_eq!(case.enforce(&mut k, 1), Err(CoreError::NotSealed));
    }
}
