//! v1.5.6 PMB 扩展：安全 API 的三种消息类型 + 服务回执。
//!
//! 安全接口必须能走 Agent-to-Agent 的**同一条消息通道**，否则它就退化成一个进程内函数调用。
//! 这里引入三种消息类型：
//!
//! | 类型 | 方向 | 载荷 |
//! |---|---|---|
//! | `safety.query` | Agent → 服务 | `PermissionQuery`（只能查自己） |
//! | `safety.report` | Agent → 服务 | `{ report, evidence }` |
//! | `safety.appeal` | Agent → 服务 | `{ appeal, evidence[] }` |
//! | `safety.receipt` | 服务 → Agent | 回执（含边界 / 案件状态 / 链头） |
//!
//! 关键点：**信封签名与记录签名是两件事**。信封证明「谁把这条消息发出来的」，
//! 记录签名证明「谁声明了这件事」。两者都必须成立，且信封发送者必须等于记录作者
//! （`Envelope.from == report.reporter == appeal.appellant`），因此无法「替别人举报/申诉」。
//!
//! `MsgKind` 是受校验的字符串（基元层刻意不做封闭枚举），所以新增消息类型不需要
//! 修改冻结基元；`safety.report` 与 `au4a_core::kinds::SAFETY_REPORT` 是同一个线格式名。

use au4a_core::{AgentKeys, CoreError, CoreResult, Did, Envelope};
use serde_json::{json, Value};

use crate::appeal::Appeal;
use crate::case::ViolationReport;
use crate::permission::PermissionQuery;

/// 安全 API 的 PMB 消息类型（线格式名）。
pub mod kinds {
    /// 权限边界查询。
    pub const SAFETY_QUERY: &str = "safety.query";
    /// 违规举报（含证据本体，供服务复算摘要）。
    pub const SAFETY_REPORT: &str = "safety.report";
    /// 申诉（含证据本体）。
    pub const SAFETY_APPEAL: &str = "safety.appeal";
    /// 服务回执。
    pub const SAFETY_RECEIPT: &str = "safety.receipt";
}

/// 解析后的安全消息。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SafetyMessage {
    /// 权限查询（`about` 必须等于发送者）。
    Query(PermissionQuery),
    /// 举报 + 其证据本体。
    Report {
        report: ViolationReport,
        evidence: Value,
    },
    /// 申诉 + 其证据本体（与 `appeal.evidence` 一一对应）。
    Appeal {
        appeal: Appeal,
        evidence: Vec<Value>,
    },
}

impl SafetyMessage {
    pub fn kind(&self) -> &'static str {
        match self {
            SafetyMessage::Query(_) => kinds::SAFETY_QUERY,
            SafetyMessage::Report { .. } => kinds::SAFETY_REPORT,
            SafetyMessage::Appeal { .. } => kinds::SAFETY_APPEAL,
        }
    }
}

/// 校验并解析一个安全消息：**先验信封，再验记录，再验作者一致**。
pub fn classify(env: &Envelope) -> CoreResult<SafetyMessage> {
    env.verify()?;
    match env.kind.as_str() {
        kinds::SAFETY_QUERY => {
            let query = PermissionQuery::from_json(&env.body)?;
            if !query.is_self_query(&env.from) {
                // 查询别人的权限边界不是本 API 的语义（边界可由公开状态自行复算）。
                return Err(CoreError::InvalidKind);
            }
            Ok(SafetyMessage::Query(query))
        }
        kinds::SAFETY_REPORT => {
            let report = ViolationReport::from_json(
                env.body.get("report").ok_or(CoreError::Encoding)?,
            )?;
            if report.reporter != env.from {
                return Err(CoreError::InvalidSignature);
            }
            report.verify()?;
            let evidence = env
                .body
                .get("evidence")
                .cloned()
                .ok_or(CoreError::Encoding)?;
            Ok(SafetyMessage::Report { report, evidence })
        }
        kinds::SAFETY_APPEAL => {
            let appeal = Appeal::from_json(env.body.get("appeal").ok_or(CoreError::Encoding)?)?;
            if appeal.appellant != env.from {
                return Err(CoreError::InvalidSignature);
            }
            appeal.verify()?;
            let evidence = env
                .body
                .get("evidence")
                .and_then(|value| value.as_array())
                .cloned()
                .ok_or(CoreError::Encoding)?;
            Ok(SafetyMessage::Appeal { appeal, evidence })
        }
        _ => Err(CoreError::InvalidKind),
    }
}

/// 构造 `safety.query` 信封（已签名）。
pub fn query_envelope(
    keys: &AgentKeys,
    to: &Did,
    ts: u64,
    about: &Did,
) -> CoreResult<Envelope> {
    let query = PermissionQuery::new(about.clone());
    Envelope::new(
        keys.did(),
        Some(to.clone()),
        kinds::SAFETY_QUERY,
        ts,
        None,
        query.to_json()?,
    )?
    .seal(keys)
}

/// 构造 `safety.report` 信封（已签名）。证据本体随消息提交，服务才能复算摘要。
pub fn report_envelope(
    keys: &AgentKeys,
    to: &Did,
    ts: u64,
    report: &ViolationReport,
    evidence: &Value,
) -> CoreResult<Envelope> {
    if report.reporter != keys.did() {
        return Err(CoreError::InvalidSignature);
    }
    report.verify()?;
    Envelope::new(
        keys.did(),
        Some(to.clone()),
        kinds::SAFETY_REPORT,
        ts,
        None,
        json!({"report": report.to_json()?, "evidence": evidence}),
    )?
    .seal(keys)
}

/// 构造 `safety.appeal` 信封（已签名）。
pub fn appeal_envelope(
    keys: &AgentKeys,
    to: &Did,
    ts: u64,
    appeal: &Appeal,
    evidence: &[Value],
) -> CoreResult<Envelope> {
    if appeal.appellant != keys.did() {
        return Err(CoreError::InvalidSignature);
    }
    appeal.verify()?;
    if evidence.len() != appeal.evidence.len() {
        return Err(CoreError::Encoding);
    }
    Envelope::new(
        keys.did(),
        Some(to.clone()),
        kinds::SAFETY_APPEAL,
        ts,
        None,
        json!({"appeal": appeal.to_json()?, "evidence": evidence}),
    )?
    .seal(keys)
}

/// 构造服务回执（已签名）。
pub fn receipt_envelope(
    keys: &AgentKeys,
    to: &Did,
    ts: u64,
    in_reply_to: &str,
    in_reply_kind: &str,
    body: Value,
) -> CoreResult<Envelope> {
    Envelope::new(
        keys.did(),
        Some(to.clone()),
        kinds::SAFETY_RECEIPT,
        ts,
        Some(in_reply_to.to_string()),
        json!({"in_reply_kind": in_reply_kind, "result": body}),
    )?
    .seal(keys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{EvidenceKind, EvidenceRef};
    use crate::setup;
    use au4a_core::{decode_frame, encode_frame, kinds as core_kinds};

    fn report(keys: &AgentKeys, subject: &Did) -> (ViolationReport, Value) {
        let payload = json!({"task": "t-1", "delivered": false});
        let evidence = EvidenceRef::commit(EvidenceKind::Transcript, &payload).unwrap();
        let report = ViolationReport::new(
            keys.did(),
            subject.clone(),
            crate::case::ViolationKind::NonDelivery,
            evidence,
            3,
        )
        .sign(keys)
        .unwrap();
        (report, payload)
    }

    #[test]
    fn wire_names_are_valid_kinds_and_match_the_frozen_primitive() {
        for name in [
            kinds::SAFETY_QUERY,
            kinds::SAFETY_REPORT,
            kinds::SAFETY_APPEAL,
            kinds::SAFETY_RECEIPT,
        ] {
            assert!(au4a_core::MsgKind::new(name).is_ok(), "{name}");
        }
        // 基元层已经预留了 safety.report：两条路径必须是同一个线格式名。
        assert_eq!(kinds::SAFETY_REPORT, core_kinds::SAFETY_REPORT);
    }

    #[test]
    fn a_query_envelope_roundtrips_through_the_wire_format() {
        let agent = setup::keys(setup::ROLE_REPORTER);
        let service = setup::keys(setup::ROLE_SERVICE);
        let env = query_envelope(&agent, &service.did(), 5, &agent.did()).unwrap();
        env.verify().unwrap();
        assert_eq!(env.kind.as_str(), kinds::SAFETY_QUERY);

        let frame = encode_frame(&env).unwrap();
        let decoded = decode_frame(&frame).unwrap();
        assert_eq!(decoded, env);
        match classify(&decoded).unwrap() {
            SafetyMessage::Query(query) => assert_eq!(query.about, agent.did()),
            other => panic!("期望 query，得到 {other:?}"),
        }
    }

    #[test]
    fn a_query_about_someone_else_is_refused() {
        let agent = setup::keys(setup::ROLE_REPORTER);
        let service = setup::keys(setup::ROLE_SERVICE);
        let other = setup::keys(setup::ROLE_SUBJECT);
        let env = query_envelope(&agent, &service.did(), 5, &other.did()).unwrap();
        assert_eq!(classify(&env), Err(CoreError::InvalidKind));
    }

    #[test]
    fn a_report_envelope_carries_both_signatures() {
        let agent = setup::keys(setup::ROLE_REPORTER);
        let service = setup::keys(setup::ROLE_SERVICE);
        let subject = setup::keys(setup::ROLE_SUBJECT).did();
        let (report, payload) = report(&agent, &subject);
        let env = report_envelope(&agent, &service.did(), 4, &report, &payload).unwrap();
        match classify(&env).unwrap() {
            SafetyMessage::Report { report: got, evidence } => {
                assert_eq!(got.id, report.id);
                assert_eq!(evidence, payload);
            }
            other => panic!("期望 report，得到 {other:?}"),
        }
    }

    #[test]
    fn tampering_with_a_report_body_is_detected() {
        let agent = setup::keys(setup::ROLE_REPORTER);
        let service = setup::keys(setup::ROLE_SERVICE);
        let subject = setup::keys(setup::ROLE_SUBJECT).did();
        let (report, payload) = report(&agent, &subject);
        let mut env = report_envelope(&agent, &service.did(), 4, &report, &payload).unwrap();
        env.body["evidence"] = json!({"task": "t-1", "delivered": true});
        assert_eq!(env.verify(), Err(CoreError::InvalidSignature));
        assert_eq!(classify(&env), Err(CoreError::InvalidSignature));
    }

    #[test]
    fn an_envelope_from_someone_other_than_the_record_author_is_refused() {
        let agent = setup::keys(setup::ROLE_REPORTER);
        let service = setup::keys(setup::ROLE_SERVICE);
        let stranger = setup::keys(0x91);
        let subject = setup::keys(setup::ROLE_SUBJECT).did();
        let (report, payload) = report(&agent, &subject);
        // 信封由陌生人签名，但内层记录的作者是 reporter：身份不一致。
        let env = Envelope::new(
            stranger.did(),
            Some(service.did()),
            kinds::SAFETY_REPORT,
            4,
            None,
            json!({"report": report.to_json().unwrap(), "evidence": payload}),
        )
        .unwrap()
        .seal(&stranger)
        .unwrap();
        assert_eq!(classify(&env), Err(CoreError::InvalidSignature));
    }

    #[test]
    fn an_appeal_envelope_requires_matching_evidence_count() {
        let subject = setup::keys(setup::ROLE_SUBJECT);
        let service = setup::keys(setup::ROLE_SERVICE);
        let payload = json!({"receipt": true});
        let reference = EvidenceRef::commit(EvidenceKind::Witness, &payload).unwrap();
        let appeal = Appeal::new("case-9", subject.did(), vec![reference], 6)
            .sign(&subject)
            .unwrap();

        assert_eq!(
            appeal_envelope(&subject, &service.did(), 7, &appeal, &[]),
            Err(CoreError::Encoding)
        );
        let env = appeal_envelope(&subject, &service.did(), 7, &appeal, &[payload.clone()]).unwrap();
        match classify(&env).unwrap() {
            SafetyMessage::Appeal { appeal: got, evidence } => {
                assert_eq!(got.id, appeal.id);
                assert_eq!(evidence, vec![payload]);
            }
            other => panic!("期望 appeal，得到 {other:?}"),
        }
    }

    #[test]
    fn unrelated_kinds_are_not_safety_messages() {
        let agent = setup::keys(setup::ROLE_REPORTER);
        let env = Envelope::new(
            agent.did(),
            None,
            core_kinds::AGENT_CARD,
            1,
            None,
            json!({}),
        )
        .unwrap()
        .seal(&agent)
        .unwrap();
        assert_eq!(classify(&env), Err(CoreError::InvalidKind));
    }

    #[test]
    fn a_receipt_points_back_at_its_request() {
        let service = setup::keys(setup::ROLE_SERVICE);
        let agent = setup::keys(setup::ROLE_REPORTER);
        let env = receipt_envelope(
            &service,
            &agent.did(),
            9,
            "request-id",
            kinds::SAFETY_QUERY,
            json!({"ok": true}),
        )
        .unwrap();
        env.verify().unwrap();
        assert_eq!(env.kind.as_str(), kinds::SAFETY_RECEIPT);
        assert_eq!(env.in_reply_to.as_deref(), Some("request-id"));
        assert_eq!(env.body["in_reply_kind"], json!(kinds::SAFETY_QUERY));
    }
}
