//! v1.2.6 — 违约处理。
//!
//! 违约不是「有人说了算」，而是一条**可验证的申诉**：
//!
//! 1. 申诉必须挂在一份**双方已签署**的合约上（[`Contract::verify`] 说了算）；
//! 2. 申诉方与被诉方必须是那条合约的两个当事人，**不能自己告自己**；
//! 3. 申诉由申诉方签名，载荷覆盖合约哈希、违约类型、证据等级与说明——改一个字节就废；
//! 4. 状态机从 `CONTRACT_SIGNED`/`EXECUTING` 进入 `ARBITRATION`，这条转换由
//!    **双方已签署的合约条款**预授权（[`crate::state::Basis::ContractClause`]）——
//!    被诉方不需要「当场同意被诉」，但他早已在合约里同意过「违约进仲裁」。
//!
//! 结论：`ARBITRATION` 的入口不依赖任何人类裁决者，也不依赖被诉方的临时配合。

use au4a_core::{canonicalize, AgentKeys, CoreError, CoreResult, Did, EvidenceGrade};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::contract::Contract;
use crate::msg::{is_label, BreachKind, MAX_NOTE};

/// 一条违约申诉。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreachClaim {
    pub contract_id: String,
    pub contract_hash: String,
    pub claimant: Did,
    pub accused: Did,
    pub kind: BreachKind,
    /// 申诉方自报的证据等级——仲裁与结算闸门都消费它（`unverified` 永远不结算）。
    pub evidence: EvidenceGrade,
    pub note: String,
    /// 逻辑时钟读数。
    pub at: u64,
    /// 申诉方对载荷的签名。
    pub sig: String,
}

impl BreachClaim {
    /// 立案：由合约的一方对另一方提出申诉。
    pub fn file(
        claimant: &AgentKeys,
        contract: &Contract,
        kind: BreachKind,
        evidence: EvidenceGrade,
        note: &str,
        at: u64,
    ) -> CoreResult<Self> {
        // 合约必须真的成立：单方签署的合约不能用来告人。
        contract.verify()?;
        if contract.hash != contract.compute_hash()? {
            return Err(CoreError::InvalidSignature);
        }
        let claimant_did = claimant.did();
        if claimant_did != contract.proposer && claimant_did != contract.responder {
            // 无关第三方没有资格就别人的合约提出申诉。
            return Err(CoreError::UnknownAgent);
        }
        if note.len() > MAX_NOTE {
            return Err(CoreError::InvalidKind);
        }
        let accused = if claimant_did == contract.proposer {
            contract.responder.clone()
        } else {
            contract.proposer.clone()
        };
        if accused == claimant_did {
            // 自己告自己不是申诉。
            return Err(CoreError::InvalidKind);
        }
        let mut claim = Self {
            contract_id: contract.id.clone(),
            contract_hash: contract.hash.clone(),
            claimant: claimant_did,
            accused,
            kind,
            evidence,
            note: note.to_string(),
            at,
            sig: String::new(),
        };
        claim.sig = claimant.sign_json(&claim.payload())?;
        claim.verify()?;
        Ok(claim)
    }

    /// 被签名的载荷。
    pub fn payload(&self) -> Value {
        json!({
            "contract_id": self.contract_id,
            "contract_hash": self.contract_hash,
            "claimant": self.claimant,
            "accused": self.accused,
            "kind": self.kind,
            "evidence": self.evidence,
            "note": self.note,
            "at": self.at,
        })
    }

    /// 校验：结构合法 + 申诉方签名成立 + 双方不同人。
    pub fn verify(&self) -> CoreResult<()> {
        if !is_label(&self.contract_id) || !crate::msg::is_hash(&self.contract_hash) {
            return Err(CoreError::InvalidKind);
        }
        if self.claimant == self.accused {
            return Err(CoreError::InvalidKind);
        }
        if self.note.len() > MAX_NOTE {
            return Err(CoreError::InvalidKind);
        }
        let bytes = canonicalize(&self.payload())?;
        self.claimant.verify(bytes.as_bytes(), &self.sig)
    }

    /// 申诉是否指向这条合约（哈希是内容寻址的，所以这就是「同一份合约」的判定）。
    pub fn is_against(&self, contract: &Contract) -> bool {
        self.contract_id == contract.id && self.contract_hash == contract.hash
    }

    /// 被诉方是不是合约的另一方（用于仲裁时对齐当事人）。
    pub fn accused_is_counterparty(&self, contract: &Contract) -> bool {
        let expected = if self.claimant == contract.proposer {
            &contract.responder
        } else if self.claimant == contract.responder {
            &contract.proposer
        } else {
            return false;
        };
        &self.accused == expected
    }

    pub fn summary(&self) -> Value {
        json!({
            "contract_id": self.contract_id,
            "contract_hash": self.contract_hash,
            "claimant": self.claimant.as_str(),
            "accused": self.accused.as_str(),
            "kind": self.kind.as_str(),
            "evidence": self.evidence.as_str(),
            "note": self.note,
            "at": self.at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg::Terms;
    use au4a_core::Credits;

    fn agent(seed: u8) -> AgentKeys {
        AgentKeys::from_seed(&[seed; 32])
    }

    fn terms(price: i64) -> Terms {
        Terms::new("summarize.zh", Credits(price), 40, EvidenceGrade::Verified).unwrap()
    }

    /// 一份双签合约 + 双方密钥。
    fn contract() -> (Contract, AgentKeys, AgentKeys) {
        let a = agent(1);
        let b = agent(2);
        let mut c = Contract::draft(&a, &b.did(), &terms(100), "session-1", 7).unwrap();
        c.sign(&a).unwrap();
        c.sign(&b).unwrap();
        (c, a, b)
    }

    #[test]
    fn a_claim_binds_the_contract_the_kind_and_the_evidence_grade() {
        let (c, a, _) = contract();
        let claim = BreachClaim::file(
            &a,
            &c,
            BreachKind::NonDelivery,
            EvidenceGrade::Verified,
            "nothing arrived",
            9,
        )
        .unwrap();
        claim.verify().unwrap();
        assert_eq!(claim.contract_id, c.id);
        assert_eq!(claim.contract_hash, c.hash);
        assert_eq!(claim.claimant, a.did());
        assert_eq!(claim.accused, c.responder);
        assert!(claim.is_against(&c));
        assert!(claim.accused_is_counterparty(&c));
        assert_eq!(claim.payload()["kind"], json!("non_delivery"));
        assert_eq!(claim.payload()["evidence"], json!("verified"));
        assert_eq!(claim.summary()["accused"], c.responder.as_str());
    }

    #[test]
    fn all_four_breach_kinds_are_accepted_and_roundtrip() {
        let (c, a, _) = contract();
        for kind in BreachKind::ALL {
            let claim = BreachClaim::file(&a, &c, kind, EvidenceGrade::CpuProto, "x", 1).unwrap();
            claim.verify().unwrap();
            let encoded = serde_json::to_string(&claim).unwrap();
            let decoded: BreachClaim = serde_json::from_str(&encoded).unwrap();
            assert_eq!(decoded, claim);
            assert_eq!(decoded.kind, kind);
        }
        assert_eq!(BreachKind::ALL.len(), 4);
    }

    #[test]
    fn tampering_or_forging_a_claim_is_refused() {
        let (c, a, b) = contract();
        let claim = BreachClaim::file(&a, &c, BreachKind::LateDelivery, EvidenceGrade::Verified, "late", 3)
            .unwrap();

        let mut kind_swapped = claim.clone();
        kind_swapped.kind = BreachKind::WrongEvidence;
        assert_eq!(kind_swapped.verify(), Err(CoreError::InvalidSignature));

        let mut hash_swapped = claim.clone();
        hash_swapped.contract_hash = "0".repeat(64);
        assert_eq!(hash_swapped.verify(), Err(CoreError::InvalidSignature));

        let mut note_changed = claim.clone();
        note_changed.note = "different story".to_string();
        assert_eq!(note_changed.verify(), Err(CoreError::InvalidSignature));

        // 被诉方替申诉方签名：不成立。
        let mut resigned = claim.clone();
        resigned.sig = b.sign_json(&resigned.payload()).unwrap();
        assert_eq!(resigned.verify(), Err(CoreError::InvalidSignature));

        // 申诉方=被诉方：结构上就拒绝。
        let mut self_claim = claim.clone();
        self_claim.accused = self_claim.claimant.clone();
        assert_eq!(self_claim.verify(), Err(CoreError::InvalidKind));
    }

    #[test]
    fn filing_requires_a_dual_signed_contract_and_a_party() {
        let (mut c, a, _) = contract();
        let outsider = agent(5);
        assert_eq!(
            BreachClaim::file(&outsider, &c, BreachKind::NonDelivery, EvidenceGrade::Verified, "x", 1),
            Err(CoreError::UnknownAgent)
        );

        // 单方签署的合约不能用来告人。
        c.signatures.pop();
        assert_eq!(
            BreachClaim::file(&a, &c, BreachKind::NonDelivery, EvidenceGrade::Verified, "x", 1),
            Err(CoreError::NotSealed)
        );
    }

    #[test]
    fn filing_refuses_bad_notes_and_broken_terms() {
        let (c, a, _) = contract();
        assert_eq!(
            BreachClaim::file(
                &a,
                &c,
                BreachKind::NonDelivery,
                EvidenceGrade::Verified,
                &"x".repeat(MAX_NOTE + 1),
                1
            ),
            Err(CoreError::InvalidKind)
        );

        // 条款被改动过的合约：哈希与签名不一致，不能作为申诉依据。
        let mut broken = c.clone();
        broken.terms.price = Credits(1);
        assert_eq!(
            BreachClaim::file(&a, &broken, BreachKind::NonDelivery, EvidenceGrade::Verified, "x", 1),
            Err(CoreError::InvalidSignature)
        );
    }
}
