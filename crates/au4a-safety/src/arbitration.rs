//! v1.5.7 仲裁接入：用**数据契约**与 `au4a-council` 解耦。
//!
//! 本 crate **不依赖** `au4a-council`，也不知道委员会是怎么投票、怎么加权、怎么防串通的。
//! 它只定义两份可序列化 JSON：
//!
//! * [`ArbitrationRequest`]：安全服务把案件事实（举报、证据、申诉、既有处罚）交出去；
//! * [`ArbitrationVerdict`]：仲裁方把结论交回来，由**配置里的仲裁者 DID** 签名。
//!
//! 这样解耦的好处是真实的：安全 API 的测试不需要委员会，委员会的实现也不需要安全 crate；
//! 两者之间只有一份可以逐字节复现的 JSON。任何一方换实现，另一方的验证逻辑不变。
//!
//! 两条语义：
//!
//! * `upheld`：执行处罚（`warning` 只记录、`stake_slash` 罚没、`quarantine` 只记录）。
//! * `rejected`：**回滚既有罚没**——误判必须能还钱。账本没有「反罚没」操作，
//!   因此归还以等额重新发行实现，守恒式 `Σ可用+Σ锁定+罚没 == 已发行` 仍然成立。
//!   账本的 `slashed` 是**历史罚没总额**（含已归还的部分），净额由记录上的 `reversed` 推导。

use au4a_core::{canonical_hash, AgentKeys, CoreError, CoreResult, Credits, Did};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::case::{CaseStatus, ViolationKind};
use crate::config::SafetyConfig;
use crate::evidence::{is_lower_hex64, EvidenceRef};
use crate::penalty::SanctionKind;

/// 裁决结论。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictOutcome {
    /// 违规成立：执行处罚。
    Upheld,
    /// 违规不成立：撤销处罚并归还罚没。
    Rejected,
}

impl VerdictOutcome {
    pub const ALL: [VerdictOutcome; 2] = [VerdictOutcome::Upheld, VerdictOutcome::Rejected];

    pub fn as_str(self) -> &'static str {
        match self {
            VerdictOutcome::Upheld => "upheld",
            VerdictOutcome::Rejected => "rejected",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        VerdictOutcome::ALL.into_iter().find(|o| o.as_str() == s)
    }
}

/// 交给仲裁方的案件事实（数据契约）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArbitrationRequest {
    pub case: String,
    pub reporter: Did,
    pub subject: Did,
    pub violation: ViolationKind,
    pub evidence: EvidenceRef,
    /// 已受理的申诉 id。
    pub appeals: Vec<String>,
    /// 已执行的处罚记录 id。
    pub penalties: Vec<String>,
    /// 生成请求时的逻辑时钟读数。
    pub requested_at: u64,
}

impl ArbitrationRequest {
    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn from_json(value: &Value) -> CoreResult<Self> {
        serde_json::from_value(value.clone()).map_err(|_| CoreError::Encoding)
    }
}

/// 仲裁裁决（数据契约）。`rationale_hash` 是理由文档的内容哈希：
/// 传承诺而不是自由文本，理由本体留在仲裁方，验证方只需要能复算。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArbitrationVerdict {
    pub id: String,
    pub case: String,
    pub outcome: VerdictOutcome,
    /// 仅 `upheld` 有意义。
    pub sanction: SanctionKind,
    /// 仅 `upheld` + `stake_slash` 有意义；其余情况必须为 0。
    pub amount: Credits,
    pub rationale_hash: String,
    pub arbiter: Did,
    pub issued_at: u64,
    pub sig: String,
}

impl ArbitrationVerdict {
    pub fn new(
        case: impl Into<String>,
        outcome: VerdictOutcome,
        sanction: SanctionKind,
        amount: Credits,
        rationale_hash: impl Into<String>,
        arbiter: Did,
        issued_at: u64,
    ) -> Self {
        Self {
            id: String::new(),
            case: case.into(),
            outcome,
            sanction,
            amount,
            rationale_hash: rationale_hash.into(),
            arbiter,
            issued_at,
            sig: String::new(),
        }
    }

    fn unsigned_payload(&self) -> Value {
        json!({
            "case": self.case,
            "outcome": self.outcome,
            "sanction": self.sanction,
            "amount": self.amount,
            "rationale_hash": self.rationale_hash,
            "arbiter": self.arbiter,
            "issued_at": self.issued_at,
        })
    }

    pub fn signing_payload(&self) -> Value {
        json!({
            "id": self.id,
            "case": self.case,
            "outcome": self.outcome,
            "sanction": self.sanction,
            "amount": self.amount,
            "rationale_hash": self.rationale_hash,
            "arbiter": self.arbiter,
            "issued_at": self.issued_at,
        })
    }

    pub fn compute_id(&self) -> CoreResult<String> {
        canonical_hash(&self.unsigned_payload())
    }

    /// 语义检查：理由承诺必须成形；处罚字段必须与结论一致。
    pub fn validate(&self) -> CoreResult<()> {
        if !is_lower_hex64(&self.rationale_hash) {
            return Err(CoreError::InvalidSignature);
        }
        match self.outcome {
            VerdictOutcome::Upheld => {
                if self.sanction.moves_ledger() {
                    if self.amount == Credits::ZERO {
                        return Err(CoreError::ZeroAmount);
                    }
                } else if self.amount != Credits::ZERO {
                    return Err(CoreError::InvalidKind);
                }
            }
            VerdictOutcome::Rejected => {
                if self.amount != Credits::ZERO {
                    // 撤销结论不带金额：归还额由记录本身决定，不由裁决书写。
                    return Err(CoreError::InvalidKind);
                }
            }
        }
        Ok(())
    }

    pub fn sign(mut self, keys: &AgentKeys) -> CoreResult<Self> {
        if self.arbiter != keys.did() {
            return Err(CoreError::InvalidSignature);
        }
        self.validate()?;
        self.id = self.compute_id()?;
        self.sig = keys.sign_json(&self.signing_payload())?;
        Ok(self)
    }

    /// 信任闸门：仲裁者名单 + `id` 自洽 + 签名 + 语义。
    pub fn verify_against(&self, config: &SafetyConfig) -> CoreResult<()> {
        config.require_arbiter(&self.arbiter)?;
        if self.sig.is_empty() {
            return Err(CoreError::NotSealed);
        }
        if self.compute_id()? != self.id {
            return Err(CoreError::InvalidSignature);
        }
        self.validate()?;
        let bytes = au4a_core::canonicalize(&self.signing_payload())?;
        self.arbiter.verify(bytes.as_bytes(), &self.sig)
    }

    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn from_json(value: &Value) -> CoreResult<Self> {
        serde_json::from_value(value.clone()).map_err(|_| CoreError::Encoding)
    }
}

/// 一次裁决执行后的结果（安全服务输出的摘要值）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseOutcome {
    pub case: String,
    pub outcome: VerdictOutcome,
    /// 裁决 id。
    pub verdict: String,
    /// 本次新罚没额（`upheld` + `stake_slash` 时可能 > 0）。
    pub applied: Credits,
    /// 本次归还额（`rejected` 时等于该案未回滚罚没的合计）。
    pub refunded: Credits,
    pub status: CaseStatus,
}

impl CaseOutcome {
    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::EvidenceKind;
    use crate::setup;
    use serde_json::json;

    fn arbiter() -> AgentKeys {
        setup::keys(setup::ROLE_ARBITER)
    }

    fn config() -> SafetyConfig {
        SafetyConfig::single_arbiter(setup::keys(setup::ROLE_SERVICE).did(), arbiter().did())
    }

    fn rationale() -> String {
        canonical_hash(&json!({"reason": "delivery receipt verified"})).unwrap()
    }

    fn upheld(amount: i64) -> ArbitrationVerdict {
        ArbitrationVerdict::new(
            "case-1",
            VerdictOutcome::Upheld,
            SanctionKind::StakeSlash,
            Credits(amount),
            rationale(),
            arbiter().did(),
            12,
        )
    }

    #[test]
    fn a_signed_verdict_passes_the_trust_gate() {
        let verdict = upheld(5).sign(&arbiter()).unwrap();
        verdict.verify_against(&config()).unwrap();
        assert_eq!(verdict.id, verdict.compute_id().unwrap());
        assert_eq!(verdict.outcome, VerdictOutcome::Upheld);
    }

    #[test]
    fn only_trusted_arbiters_can_seal_a_verdict() {
        let outsider = AgentKeys::from_seed(&[0x71; 32]);
        assert_eq!(upheld(5).sign(&outsider), Err(CoreError::InvalidSignature));

        // 陌生人自签自证：签名自洽但不在信任名单内。
        let stranger = AgentKeys::from_seed(&[0x72; 32]);
        let mut foreign = ArbitrationVerdict::new(
            "case-1",
            VerdictOutcome::Upheld,
            SanctionKind::StakeSlash,
            Credits(5),
            rationale(),
            stranger.did(),
            12,
        );
        foreign.id = foreign.compute_id().unwrap();
        foreign.sig = stranger.sign_json(&foreign.signing_payload()).unwrap();
        assert_eq!(
            foreign.verify_against(&config()),
            Err(CoreError::InvalidSignature)
        );
    }

    #[test]
    fn tampering_with_the_outcome_or_amount_is_detected() {
        let mut verdict = upheld(5).sign(&arbiter()).unwrap();
        verdict.amount = Credits(500);
        assert_eq!(
            verdict.verify_against(&config()),
            Err(CoreError::InvalidSignature)
        );

        let mut verdict = upheld(5).sign(&arbiter()).unwrap();
        verdict.outcome = VerdictOutcome::Rejected;
        assert_eq!(
            verdict.verify_against(&config()),
            Err(CoreError::InvalidSignature)
        );
    }

    #[test]
    fn the_rationale_must_be_a_real_commitment() {
        let bad = ArbitrationVerdict::new(
            "case-1",
            VerdictOutcome::Upheld,
            SanctionKind::StakeSlash,
            Credits(5),
            "because I said so",
            arbiter().did(),
            12,
        );
        assert_eq!(
            bad.clone().sign(&arbiter()),
            Err(CoreError::InvalidSignature)
        );
        assert_eq!(bad.validate(), Err(CoreError::InvalidSignature));
    }

    #[test]
    fn rejected_verdicts_carry_no_amount() {
        let rejected = ArbitrationVerdict::new(
            "case-1",
            VerdictOutcome::Rejected,
            SanctionKind::Warning,
            Credits(0),
            rationale(),
            arbiter().did(),
            12,
        )
        .sign(&arbiter())
        .unwrap();
        rejected.verify_against(&config()).unwrap();

        let greedy = ArbitrationVerdict::new(
            "case-1",
            VerdictOutcome::Rejected,
            SanctionKind::StakeSlash,
            Credits(5),
            rationale(),
            arbiter().did(),
            12,
        );
        assert_eq!(greedy.sign(&arbiter()), Err(CoreError::InvalidKind));
    }

    #[test]
    fn upheld_warnings_must_not_carry_an_amount() {
        let warning = ArbitrationVerdict::new(
            "case-1",
            VerdictOutcome::Upheld,
            SanctionKind::Warning,
            Credits(3),
            rationale(),
            arbiter().did(),
            12,
        );
        assert_eq!(warning.sign(&arbiter()), Err(CoreError::InvalidKind));
    }

    #[test]
    fn the_contract_roundtrips_through_plain_json_without_any_external_crate() {
        // 「外部仲裁方」在这个测试里就是一个手写的 JSON 字面量 + 一把密钥：
        // 这正是解耦的含义——契约只要求字节一致，不要求同一个 crate。
        let rationale = rationale();
        let arbiter = arbiter();
        let draft = json!({
            "id": "",
            "sig": "",
            "case": "case-42",
            "outcome": "upheld",
            "sanction": "stake_slash",
            "amount": 7,
            "rationale_hash": rationale,
            "arbiter": arbiter.did(),
            "issued_at": 99,
        });
        let mut verdict = ArbitrationVerdict::from_json(&draft).unwrap();
        verdict.id = verdict.compute_id().unwrap();
        verdict.sig = arbiter.sign_json(&verdict.signing_payload()).unwrap();
        verdict.verify_against(&config()).unwrap();

        // 往返一次仍然是同一串规范字节。
        let text = serde_json::to_string(&verdict.to_json().unwrap()).unwrap();
        let reparsed =
            ArbitrationVerdict::from_json(&serde_json::from_str(&text).unwrap()).unwrap();
        assert_eq!(reparsed, verdict);
        assert_eq!(reparsed.id, verdict.id);
    }

    #[test]
    fn requests_carry_the_case_facts_and_roundtrip() {
        let request = ArbitrationRequest {
            case: "case-7".to_string(),
            reporter: setup::keys(setup::ROLE_REPORTER).did(),
            subject: setup::keys(setup::ROLE_SUBJECT).did(),
            violation: ViolationKind::NonDelivery,
            evidence: EvidenceRef::commit(EvidenceKind::Transcript, &json!({"x": 1})).unwrap(),
            appeals: vec!["appeal-1".to_string()],
            penalties: vec!["penalty-1".to_string()],
            requested_at: 30,
        };
        let json = request.to_json().unwrap();
        assert_eq!(ArbitrationRequest::from_json(&json).unwrap(), request);
    }

    #[test]
    fn outcomes_roundtrip_and_are_unique() {
        assert_eq!(VerdictOutcome::ALL.len(), 2);
        for outcome in VerdictOutcome::ALL {
            assert_eq!(VerdictOutcome::parse(outcome.as_str()), Some(outcome));
        }
        assert_eq!(VerdictOutcome::parse("maybe"), None);
    }
}
