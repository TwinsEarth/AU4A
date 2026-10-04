//! v1.5.2 案件模型：违规种类、案件状态、举报记录。
//!
//! 举报是**签名声明**，不是判决：`ViolationReport` 由举报人签名并内容寻址，
//! 它进入案件后只把状态推到 `reported`。任何惩罚都必须来自后续的裁决
//! （v1.5.4 的处罚契约 / v1.5.7 的仲裁裁决），本模块不做任何账本操作。

use au4a_core::{canonical_hash, AgentKeys, CoreError, CoreResult, Did};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::evidence::EvidenceRef;

/// 违规种类。封闭集合：新增种类是接口扩展，不是自由文本。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViolationKind {
    /// 垃圾信息 / 公告轰炸。
    Spam,
    /// 欺诈性交付声明。
    Fraud,
    /// 承诺后不交付。
    NonDelivery,
    /// 提交伪造证据。
    FakeEvidence,
    /// 未授权访问。
    UnauthorizedAccess,
    /// 串通操纵。
    Collusion,
}

impl ViolationKind {
    pub const ALL: [ViolationKind; 6] = [
        ViolationKind::Spam,
        ViolationKind::Fraud,
        ViolationKind::NonDelivery,
        ViolationKind::FakeEvidence,
        ViolationKind::UnauthorizedAccess,
        ViolationKind::Collusion,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ViolationKind::Spam => "spam",
            ViolationKind::Fraud => "fraud",
            ViolationKind::NonDelivery => "non_delivery",
            ViolationKind::FakeEvidence => "fake_evidence",
            ViolationKind::UnauthorizedAccess => "unauthorized_access",
            ViolationKind::Collusion => "collusion",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        ViolationKind::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// 案件状态。`Reported` 是**未确认**状态：它不改动账本，也不改信誉。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseStatus {
    /// 已受理、未确认。
    Reported,
    /// 被处罚方（案件主体）已提交申诉。
    Appealed,
    /// 已由仲裁者签名的处罚契约执行处罚。
    Penalized,
    /// 已由仲裁裁决终局（v1.5.7）。
    Arbitrated,
}

impl CaseStatus {
    pub const ALL: [CaseStatus; 4] = [
        CaseStatus::Reported,
        CaseStatus::Appealed,
        CaseStatus::Penalized,
        CaseStatus::Arbitrated,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            CaseStatus::Reported => "reported",
            CaseStatus::Appealed => "appealed",
            CaseStatus::Penalized => "penalized",
            CaseStatus::Arbitrated => "arbitrated",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        CaseStatus::ALL.into_iter().find(|status| status.as_str() == s)
    }
}

/// 举报记录。签名覆盖 `id`，`id` 是对未签名字段的内容寻址——
/// 与 PMB `Envelope` 的封口约定完全一致（先定 id，再签 id）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViolationReport {
    pub id: String,
    pub reporter: Did,
    pub subject: Did,
    pub violation: ViolationKind,
    pub evidence: EvidenceRef,
    /// 逻辑时钟读数。
    pub at: u64,
    /// hex 编码的 Ed25519 签名；空串表示未签名。
    pub sig: String,
}

impl ViolationReport {
    /// 构造未签名举报（`id` 与 `sig` 由 [`ViolationReport::sign`] 填充）。
    pub fn new(
        reporter: Did,
        subject: Did,
        violation: ViolationKind,
        evidence: EvidenceRef,
        at: u64,
    ) -> Self {
        Self {
            id: String::new(),
            reporter,
            subject,
            violation,
            evidence,
            at,
            sig: String::new(),
        }
    }

    fn unsigned_payload(&self) -> Value {
        json!({
            "reporter": self.reporter,
            "subject": self.subject,
            "violation": self.violation,
            "evidence": self.evidence,
            "at": self.at,
        })
    }

    /// 被签名的载荷：含 `id`。
    pub fn signing_payload(&self) -> Value {
        json!({
            "id": self.id,
            "reporter": self.reporter,
            "subject": self.subject,
            "violation": self.violation,
            "evidence": self.evidence,
            "at": self.at,
        })
    }

    pub fn compute_id(&self) -> CoreResult<String> {
        canonical_hash(&self.unsigned_payload())
    }

    /// 由举报人签名并封口。签名者必须是 `reporter` 本人。
    pub fn sign(mut self, keys: &AgentKeys) -> CoreResult<Self> {
        if self.reporter != keys.did() {
            return Err(CoreError::InvalidSignature);
        }
        self.id = self.compute_id()?;
        self.sig = keys.sign_json(&self.signing_payload())?;
        Ok(self)
    }

    /// 校验：`id` 自洽 + 证据摘要成形 + 签名由举报人出具。
    pub fn verify(&self) -> CoreResult<()> {
        if self.sig.is_empty() {
            return Err(CoreError::NotSealed);
        }
        if self.compute_id()? != self.id {
            return Err(CoreError::InvalidSignature);
        }
        if !self.evidence.is_well_formed() {
            return Err(CoreError::InvalidSignature);
        }
        let bytes = au4a_core::canonicalize(&self.signing_payload())?;
        self.reporter.verify(bytes.as_bytes(), &self.sig)
    }

    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn from_json(value: &Value) -> CoreResult<Self> {
        serde_json::from_value(value.clone()).map_err(|_| CoreError::Encoding)
    }
}

/// 一个案件：一次举报 + 它的状态与历史。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Case {
    /// 案件 id == 举报 id（内容寻址）。
    pub id: String,
    pub reporter: Did,
    pub subject: Did,
    pub violation: ViolationKind,
    pub evidence: EvidenceRef,
    pub opened_at: u64,
    pub status: CaseStatus,
}

impl Case {
    pub fn from_report(report: &ViolationReport) -> Self {
        Self {
            id: report.id.clone(),
            reporter: report.reporter.clone(),
            subject: report.subject.clone(),
            violation: report.violation,
            evidence: report.evidence.clone(),
            opened_at: report.at,
            status: CaseStatus::Reported,
        }
    }

    pub fn involves(&self, did: &Did) -> bool {
        &self.reporter == did || &self.subject == did
    }

    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::EvidenceKind;
    use serde_json::json;

    fn signed_report(seed: u8) -> (AgentKeys, ViolationReport) {
        let reporter = AgentKeys::from_seed(&[seed; 32]);
        let subject = AgentKeys::from_seed(&[seed.wrapping_add(1); 32]);
        let evidence =
            EvidenceRef::commit(EvidenceKind::Transcript, &json!({"delivered": false})).unwrap();
        let report = ViolationReport::new(
            reporter.did(),
            subject.did(),
            ViolationKind::NonDelivery,
            evidence,
            7,
        )
        .sign(&reporter)
        .unwrap();
        (reporter, report)
    }

    #[test]
    fn a_signed_report_verifies_and_is_content_addressed() {
        let (reporter, report) = signed_report(11);
        report.verify().unwrap();
        assert_eq!(report.id, report.compute_id().unwrap());
        let again = ViolationReport::new(
            report.reporter.clone(),
            report.subject.clone(),
            report.violation,
            report.evidence.clone(),
            report.at,
        )
        .sign(&reporter)
        .unwrap();
        assert_eq!(again.id, report.id);
        assert_eq!(again.sig, report.sig, "同一内容同一密钥必须得到同一签名");
    }

    #[test]
    fn tampering_with_the_subject_or_evidence_is_detected() {
        let (_, mut report) = signed_report(12);
        report.subject = AgentKeys::from_seed(&[99u8; 32]).did();
        assert_eq!(report.verify(), Err(CoreError::InvalidSignature));

        let (_, mut report) = signed_report(13);
        report.evidence.digest = "0".repeat(64);
        assert_eq!(report.verify(), Err(CoreError::InvalidSignature));
    }

    #[test]
    fn someone_elses_report_cannot_be_sealed_and_an_unsigned_one_is_refused() {
        let (_, report) = signed_report(14);
        let stranger = AgentKeys::from_seed(&[200u8; 32]);
        let forged = ViolationReport::new(
            report.reporter.clone(),
            report.subject.clone(),
            report.violation,
            report.evidence.clone(),
            report.at,
        )
        .sign(&stranger);
        assert_eq!(forged, Err(CoreError::InvalidSignature));

        let unsigned = ViolationReport::new(
            report.reporter.clone(),
            report.subject.clone(),
            report.violation,
            report.evidence.clone(),
            report.at,
        );
        assert_eq!(unsigned.verify(), Err(CoreError::NotSealed));
    }

    #[test]
    fn report_roundtrips_through_json() {
        let (_, report) = signed_report(15);
        let json = report.to_json().unwrap();
        assert_eq!(ViolationReport::from_json(&json).unwrap(), report);
    }

    #[test]
    fn kinds_and_statuses_roundtrip_and_are_unique() {
        assert_eq!(ViolationKind::ALL.len(), 6);
        for kind in ViolationKind::ALL {
            assert_eq!(ViolationKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(ViolationKind::parse("treason"), None);
        assert_eq!(CaseStatus::ALL.len(), 4);
        for status in CaseStatus::ALL {
            assert_eq!(CaseStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(CaseStatus::parse("forgiven"), None);
    }

    #[test]
    fn a_case_starts_unconfirmed() {
        let (_, report) = signed_report(16);
        let case = Case::from_report(&report);
        assert_eq!(case.status, CaseStatus::Reported);
        assert_eq!(case.id, report.id);
        assert!(case.involves(&report.reporter));
        assert!(case.involves(&report.subject));
        assert!(!case.involves(&AgentKeys::from_seed(&[3u8; 32]).did()));
        assert_eq!(case.to_json().unwrap()["status"], json!("reported"));
    }
}
