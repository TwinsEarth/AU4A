//! v1.5.3 申诉：被处罚方（案件主体）提交申诉证据。
//!
//! 两条设计判断：
//!
//! 1. **申诉必须由案件主体本人签名**。第三方「代为申诉」在密码学上就不成立：
//!    校验用的是主体的 DID，别人的签名过不了。这样申诉权无法被代理，也无法被冒充。
//! 2. **申诉权不被终局性剥夺**。案件即使已 `arbitrated`，只要出现新证据仍可再申诉
//!    （状态回到 `appealed`，由后续裁决重新处理）。终局性属于裁决，不属于申诉人。
//!
//! 申诉本身**不动账本**：它只提交证据、推状态、写链。

use au4a_core::{canonical_hash, AgentKeys, CoreError, CoreResult, Did};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::evidence::{require_well_formed, EvidenceRef};

/// 一条申诉记录。封口约定与 `Envelope` / `ViolationReport` 一致：先定 id，再签 id。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Appeal {
    pub id: String,
    /// 案件 id。
    pub case: String,
    /// 申诉人 == 案件主体（由服务侧强制）。
    pub appellant: Did,
    /// 申诉证据（至少一条，且摘要必须可复算）。
    pub evidence: Vec<EvidenceRef>,
    pub at: u64,
    pub sig: String,
}

impl Appeal {
    pub fn new(
        case: impl Into<String>,
        appellant: Did,
        evidence: Vec<EvidenceRef>,
        at: u64,
    ) -> Self {
        Self {
            id: String::new(),
            case: case.into(),
            appellant,
            evidence,
            at,
            sig: String::new(),
        }
    }

    fn unsigned_payload(&self) -> Value {
        json!({
            "case": self.case,
            "appellant": self.appellant,
            "evidence": self.evidence,
            "at": self.at,
        })
    }

    /// 被签名的载荷：含 `id`。
    pub fn signing_payload(&self) -> Value {
        json!({
            "id": self.id,
            "case": self.case,
            "appellant": self.appellant,
            "evidence": self.evidence,
            "at": self.at,
        })
    }

    pub fn compute_id(&self) -> CoreResult<String> {
        canonical_hash(&self.unsigned_payload())
    }

    pub fn sign(mut self, keys: &AgentKeys) -> CoreResult<Self> {
        if self.appellant != keys.did() {
            return Err(CoreError::InvalidSignature);
        }
        require_well_formed(&self.evidence)?;
        self.id = self.compute_id()?;
        self.sig = keys.sign_json(&self.signing_payload())?;
        Ok(self)
    }

    pub fn verify(&self) -> CoreResult<()> {
        if self.sig.is_empty() {
            return Err(CoreError::NotSealed);
        }
        if self.compute_id()? != self.id {
            return Err(CoreError::InvalidSignature);
        }
        require_well_formed(&self.evidence)?;
        let bytes = au4a_core::canonicalize(&self.signing_payload())?;
        self.appellant.verify(bytes.as_bytes(), &self.sig)
    }

    pub fn evidence_count(&self) -> usize {
        self.evidence.len()
    }

    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn from_json(value: &Value) -> CoreResult<Self> {
        serde_json::from_value(value.clone()).map_err(|_| CoreError::Encoding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::EvidenceKind;
    use serde_json::json;

    fn refs(tag: &str) -> (Vec<EvidenceRef>, Vec<Value>) {
        let payloads = vec![json!({"tag": tag, "reason": "misread the transcript"})];
        let references = payloads
            .iter()
            .map(|p| EvidenceRef::commit(EvidenceKind::Transcript, p).unwrap())
            .collect();
        (references, payloads)
    }

    fn signed_appeal(seed: u8) -> (AgentKeys, Appeal) {
        let appellant = AgentKeys::from_seed(&[seed; 32]);
        let (references, _) = refs("appeal");
        let appeal = Appeal::new("case-1", appellant.did(), references, 9)
            .sign(&appellant)
            .unwrap();
        (appellant, appeal)
    }

    #[test]
    fn an_appeal_is_signed_by_its_appellant_and_content_addressed() {
        let (_, appeal) = signed_appeal(21);
        appeal.verify().unwrap();
        assert_eq!(appeal.id, appeal.compute_id().unwrap());
        assert_eq!(appeal.evidence_count(), 1);
    }

    #[test]
    fn a_third_party_cannot_seal_an_appeal_for_someone_else() {
        let (appellant, appeal) = signed_appeal(22);
        let stranger = AgentKeys::from_seed(&[0x33; 32]);
        let forged = Appeal::new(
            appeal.case.clone(),
            appellant.did(),
            appeal.evidence.clone(),
            9,
        )
        .sign(&stranger);
        assert_eq!(forged, Err(CoreError::InvalidSignature));
    }

    #[test]
    fn tampering_with_evidence_or_case_is_detected() {
        let (_, mut appeal) = signed_appeal(23);
        appeal.case = "another-case".to_string();
        assert_eq!(appeal.verify(), Err(CoreError::InvalidSignature));

        let (_, mut appeal) = signed_appeal(24);
        appeal.evidence[0].digest = "f".repeat(64);
        assert_eq!(appeal.verify(), Err(CoreError::InvalidSignature));
    }

    #[test]
    fn an_appeal_without_evidence_cannot_be_sealed_or_verified() {
        let appellant = AgentKeys::from_seed(&[25u8; 32]);
        let empty = Appeal::new("case-2", appellant.did(), Vec::new(), 3);
        assert_eq!(
            empty.clone().sign(&appellant),
            Err(CoreError::InvalidSignature)
        );
        assert_eq!(empty.verify(), Err(CoreError::NotSealed));
        let malformed = Appeal::new(
            "case-2",
            appellant.did(),
            vec![EvidenceRef {
                kind: EvidenceKind::Witness,
                digest: "short".to_string(),
            }],
            3,
        );
        assert_eq!(malformed.sign(&appellant), Err(CoreError::InvalidSignature));
    }

    #[test]
    fn appeals_roundtrip_through_json() {
        let (_, appeal) = signed_appeal(26);
        let json = appeal.to_json().unwrap();
        assert_eq!(Appeal::from_json(&json).unwrap(), appeal);
    }
}
