//! v1.5.2 安全服务（`SafetyOffice`）：受理举报，并把每次状态变更写进哈希链。
//!
//! **无罪不罚在这里是结构性保证，不是纪律要求**：`report()` 路径上不存在任何
//! 账本写入调用。信誉同理——`AgentCard` 只读不写。要动账本，必须有一条带仲裁者
//! 签名的裁决（v1.5.4 / v1.5.7），这条路径在 `report()` 里根本不可达。
//!
//! 服务自身也是一个 Agent（`service` 密钥）：它与其它 Agent 用同一套签名原语，
//! 没有「运营方」这种超越身份的角色。

use std::collections::BTreeMap;

use au4a_core::{canonical_hash, AgentKeys, CoreError, CoreResult, Did, RefusalCode};
use au4a_kernel::Kernel;
use serde_json::{json, Value};

use crate::appeal::Appeal;
use crate::case::{Case, CaseStatus, ViolationKind, ViolationReport};
use crate::chain::{chain_head, verify_chain, ChainVerdict, SafetyEvent, SafetyEventKind};
use crate::config::SafetyConfig;
use crate::evidence::EvidenceRef;

/// 安全服务：案件登记处 + 变更日志。
pub struct SafetyOffice {
    config: SafetyConfig,
    service: AgentKeys,
    events: Vec<SafetyEvent>,
    cases: BTreeMap<String, Case>,
    appeals: BTreeMap<String, Appeal>,
}

impl SafetyOffice {
    /// 构造服务。服务密钥必须与配置中的 `service` DID 一致——
    /// 否则任何人都能拿别的密钥冒充安全服务签发回执。
    pub fn new(config: SafetyConfig, service: AgentKeys) -> CoreResult<Self> {
        if service.did() != config.service {
            return Err(CoreError::InvalidSignature);
        }
        Ok(Self {
            config,
            service,
            events: Vec::new(),
            cases: BTreeMap::new(),
            appeals: BTreeMap::new(),
        })
    }

    pub fn config(&self) -> &SafetyConfig {
        &self.config
    }

    /// 服务身份（它也是一个 Agent）。DID 直接由持有的密钥派生，
    /// 因此「谁是安全服务」永远等于「谁持有能签发回执的那把密钥」。
    pub fn service_did(&self) -> Did {
        self.service.did()
    }

    pub fn events(&self) -> &[SafetyEvent] {
        &self.events
    }

    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// 当前链头（空链为创世前驱）。
    pub fn chain_head(&self) -> String {
        chain_head(&self.events)
    }

    pub fn verify_chain(&self) -> ChainVerdict {
        verify_chain(&self.events)
    }

    pub fn case(&self, id: &str) -> Option<&Case> {
        self.cases.get(id)
    }

    pub fn case_count(&self) -> usize {
        self.cases.len()
    }

    pub fn cases(&self) -> impl Iterator<Item = &Case> {
        self.cases.values()
    }

    /// 追加一条链上事件。序号、时间、前驱全部由链自身决定，调用方无法伪造。
    pub(crate) fn append(
        &mut self,
        kernel: &mut Kernel,
        kind: SafetyEventKind,
        payload: Value,
    ) -> CoreResult<()> {
        let at = kernel.tick();
        let seq = self.events.len() as u64;
        let prev = chain_head(&self.events);
        let event = SafetyEvent::seal(seq, at, kind, payload, &prev)?;
        self.events.push(event);
        Ok(())
    }

    /// 受理一次举报。**不触碰账本，不触碰信誉。**
    ///
    /// 证据本体随举报提交：安全服务必须能复算摘要，否则「带哈希」只是装饰。
    pub fn report(
        &mut self,
        kernel: &mut Kernel,
        keys: &AgentKeys,
        subject: &Did,
        violation: ViolationKind,
        evidence: EvidenceRef,
        payload: &Value,
    ) -> CoreResult<ViolationReport> {
        let reporter = keys.did();

        if kernel.card(&reporter).is_none() {
            kernel.refuse(
                &reporter,
                RefusalCode::Unauthorized,
                "report from an unregistered did",
            );
            return Err(CoreError::UnknownAgent);
        }
        if reporter == *subject {
            kernel.refuse(
                &reporter,
                RefusalCode::PolicyDenied,
                "a self-report is not a violation report",
            );
            return Err(CoreError::InvalidKind);
        }
        if kernel.card(subject).is_none() {
            kernel.refuse(
                &reporter,
                RefusalCode::StaleEpoch,
                "report subject is not a registered agent",
            );
            return Err(CoreError::UnknownAgent);
        }
        if !evidence.is_well_formed() {
            kernel.refuse(
                &reporter,
                RefusalCode::PolicyDenied,
                "evidence digest is missing or malformed",
            );
            return Err(CoreError::InvalidSignature);
        }
        if payload.is_null() {
            kernel.refuse(
                &reporter,
                RefusalCode::PolicyDenied,
                "evidence payload is missing so the digest cannot be recomputed",
            );
            return Err(CoreError::Encoding);
        }
        if evidence.verify(payload).is_err() {
            kernel.refuse(
                &reporter,
                RefusalCode::PolicyDenied,
                "evidence digest does not match the submitted payload",
            );
            return Err(CoreError::InvalidSignature);
        }

        let at = kernel.tick();
        let report = ViolationReport::new(
            reporter.clone(),
            subject.clone(),
            violation,
            evidence,
            at,
        )
        .sign(keys)?;
        report.verify()?;

        let case = Case::from_report(&report);
        let case_id = case.id.clone();
        self.cases.insert(case_id.clone(), case);
        self.append(
            kernel,
            SafetyEventKind::Reported,
            json!({"case": case_id, "report": report.to_json()?}),
        )?;
        kernel.emit(
            &format!("{}.reported", crate::TRACK),
            format!(
                "{} 举报 {}（{}）",
                au4a_core::short_id(reporter.as_str()),
                au4a_core::short_id(subject.as_str()),
                violation.as_str()
            ),
        );
        Ok(report)
    }

    /// 案件的处理状态（未确认 == `reported`）。
    pub fn status_of(&self, case_id: &str) -> Option<CaseStatus> {
        self.cases.get(case_id).map(|c| c.status)
    }

    pub fn appeal_by_id(&self, id: &str) -> Option<&Appeal> {
        self.appeals.get(id)
    }

    pub fn appeal_count(&self) -> usize {
        self.appeals.len()
    }

    /// 某案件的申诉（按提交顺序）。
    pub fn appeals_of(&self, case_id: &str) -> Vec<&Appeal> {
        self.cases
            .get(case_id)
            .map(|case| {
                case.appeals
                    .iter()
                    .filter_map(|id| self.appeals.get(id))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 受理一次申诉。**只有案件主体本人**能申诉；申诉只提交证据、推状态、写链，
    /// 不触碰账本——罚没与归还永远只发生在裁决路径上。
    ///
    /// 申诉权不被终局性剥夺：已 `arbitrated` 的案件收到新证据后回到 `appealed`，
    /// 由下一次裁决重新处理。
    pub fn appeal(
        &mut self,
        kernel: &mut Kernel,
        keys: &AgentKeys,
        case_id: &str,
        evidence: Vec<EvidenceRef>,
        payloads: &[Value],
    ) -> CoreResult<Appeal> {
        let appellant = keys.did();

        let subject = match self.cases.get(case_id) {
            Some(case) => case.subject.clone(),
            None => {
                kernel.refuse(
                    &appellant,
                    RefusalCode::StaleEpoch,
                    "appeal references an unknown case",
                );
                return Err(CoreError::UnknownAgent);
            }
        };
        if appellant != subject {
            kernel.refuse(
                &appellant,
                RefusalCode::Unauthorized,
                "only the case subject may appeal; a third party cannot sign for them",
            );
            return Err(CoreError::InvalidSignature);
        }
        if evidence.is_empty() || evidence.len() != payloads.len() {
            kernel.refuse(
                &appellant,
                RefusalCode::PolicyDenied,
                "appeal evidence is missing or its payloads do not line up",
            );
            return Err(CoreError::InvalidSignature);
        }
        for (reference, payload) in evidence.iter().zip(payloads.iter()) {
            if payload.is_null() || reference.verify(payload).is_err() {
                kernel.refuse(
                    &appellant,
                    RefusalCode::PolicyDenied,
                    "appeal evidence digest does not match its payload",
                );
                return Err(CoreError::InvalidSignature);
            }
        }

        let at = kernel.tick();
        let appeal =
            Appeal::new(case_id.to_string(), appellant.clone(), evidence, at).sign(keys)?;
        appeal.verify()?;

        if let Some(case) = self.cases.get_mut(case_id) {
            case.status = CaseStatus::Appealed;
            case.appeals.push(appeal.id.clone());
        }
        self.appeals.insert(appeal.id.clone(), appeal.clone());
        self.append(
            kernel,
            SafetyEventKind::Appealed,
            json!({"case": case_id, "appeal": appeal.to_json()?}),
        )?;
        kernel.emit(
            &format!("{}.appealed", crate::TRACK),
            format!(
                "案件 {} 收到申诉（证据 {} 条）",
                au4a_core::short_id(case_id),
                appeal.evidence_count()
            ),
        );
        Ok(appeal)
    }
}

/// 账本快照：参与者逐字段 + 全局发行/罚没。用于「未确认不动账本」的可比较断言。
///
/// 账户按 DID 排序，因此同一状态永远得到同一份投影。
pub fn ledger_snapshot(kernel: &Kernel, dids: &[Did]) -> Value {
    let mut accounts: Vec<Value> = dids
        .iter()
        .map(|did| {
            let account = kernel.ledger().balance(did);
            json!({
                "did": did.as_str(),
                "available": account.available.get(),
                "locked": account.locked.get(),
            })
        })
        .collect();
    accounts.sort_by(|a, b| a["did"].as_str().cmp(&b["did"].as_str()));
    json!({
        "minted": kernel.ledger().minted().get(),
        "slashed": kernel.ledger().slashed().get(),
        "accounts": accounts,
    })
}

/// 账本指纹：快照的规范哈希，便于在 JSON 证据里做等值断言。
pub fn ledger_fingerprint(kernel: &Kernel, dids: &[Did]) -> CoreResult<String> {
    canonical_hash(&ledger_snapshot(kernel, dids))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::EvidenceKind;
    use crate::setup;
    use au4a_core::Credits;
    use au4a_kernel::KernelConfig;

    struct World {
        kernel: Kernel,
        office: SafetyOffice,
        reporter: AgentKeys,
        subject: AgentKeys,
        participants: Vec<Did>,
    }

    fn world() -> World {
        let mut kernel = Kernel::new(KernelConfig::default());
        let service = setup::keys(setup::ROLE_SERVICE);
        let reporter = setup::keys(setup::ROLE_REPORTER);
        let subject = setup::keys(setup::ROLE_SUBJECT);
        let arbiter = setup::keys(setup::ROLE_ARBITER);
        for (keys, display, skill) in [
            (&service, "safety-service", "safety.api"),
            (&reporter, "reporter-agent", "audit.report"),
            (&subject, "subject-agent", "deliver.task"),
        ] {
            setup::ensure_agent(&mut kernel, keys, display, &[skill], Credits(20)).unwrap();
        }
        let config = SafetyConfig::single_arbiter(service.did(), arbiter.did());
        let office = SafetyOffice::new(config, service).unwrap();
        let participants = vec![reporter.did(), subject.did()];
        World {
            kernel,
            office,
            reporter,
            subject,
            participants,
        }
    }

    fn evidence(tag: &str) -> (EvidenceRef, Value) {
        let payload = json!({"tag": tag, "delivered": false});
        let reference = EvidenceRef::commit(EvidenceKind::Transcript, &payload).unwrap();
        (reference, payload)
    }

    #[test]
    fn a_report_opens_an_unconfirmed_case_and_grows_the_chain() {
        let mut w = world();
        let (reference, payload) = evidence("case-1");
        let report = w
            .office
            .report(
                &mut w.kernel,
                &w.reporter,
                &w.subject.did(),
                ViolationKind::NonDelivery,
                reference,
                &payload,
            )
            .unwrap();

        assert_eq!(w.office.case_count(), 1);
        let case = w.office.case(&report.id).unwrap();
        assert_eq!(case.status, CaseStatus::Reported);
        assert_eq!(case.reporter, w.reporter.did());
        assert_eq!(case.subject, w.subject.did());

        assert_eq!(w.office.event_count(), 1);
        assert_eq!(w.office.events()[0].kind, SafetyEventKind::Reported);
        let verdict = w.office.verify_chain();
        assert!(verdict.ok, "{verdict:?}");
        assert_eq!(verdict.head, w.office.chain_head());
        assert_eq!(verdict.head.len(), 64);
    }

    #[test]
    fn an_unconfirmed_report_moves_no_balance_and_no_card() {
        let mut w = world();
        let before = ledger_snapshot(&w.kernel, &w.participants);
        let before_fingerprint = ledger_fingerprint(&w.kernel, &w.participants).unwrap();
        let subject_card_before = w.kernel.card(&w.subject.did()).cloned();
        let reporter_card_before = w.kernel.card(&w.reporter.did()).cloned();

        let (reference, payload) = evidence("case-2");
        let report = w
            .office
            .report(
                &mut w.kernel,
                &w.reporter,
                &w.subject.did(),
                ViolationKind::Fraud,
                reference,
                &payload,
            )
            .unwrap();

        // 无罪不罚：逐字段相等，而不是「差值很小」。
        assert_eq!(ledger_snapshot(&w.kernel, &w.participants), before);
        assert_eq!(
            ledger_fingerprint(&w.kernel, &w.participants).unwrap(),
            before_fingerprint
        );
        assert_eq!(w.kernel.card(&w.subject.did()).cloned(), subject_card_before);
        assert_eq!(
            w.kernel.card(&w.reporter.did()).cloned(),
            reporter_card_before
        );
        assert_eq!(w.kernel.ledger().slashed(), Credits::ZERO);
        w.kernel.ledger().check_conservation().unwrap();
        // 案件仍未被确认。
        assert_eq!(w.office.status_of(&report.id), Some(CaseStatus::Reported));
    }

    #[test]
    fn forged_evidence_is_refused_and_leaves_no_trace() {
        let mut w = world();
        let before = ledger_snapshot(&w.kernel, &w.participants);
        let (honest, _) = evidence("case-3");
        let (_, other_payload) = evidence("case-3-forged");

        let result = w.office.report(
            &mut w.kernel,
            &w.reporter,
            &w.subject.did(),
            ViolationKind::FakeEvidence,
            honest,
            &other_payload,
        );
        assert_eq!(result, Err(CoreError::InvalidSignature));
        assert_eq!(w.office.case_count(), 0);
        assert_eq!(w.office.event_count(), 0);
        assert_eq!(ledger_snapshot(&w.kernel, &w.participants), before);
        let (_, refusal) = &w.kernel.refusals()[0];
        assert_eq!(refusal.code, RefusalCode::PolicyDenied);
        assert!(refusal.reason.contains("does not match"));
    }

    #[test]
    fn missing_or_malformed_evidence_is_refused() {
        let mut w = world();
        // 摘要不成形（缺失）。
        let malformed = EvidenceRef {
            kind: EvidenceKind::Transcript,
            digest: String::new(),
        };
        assert_eq!(
            w.office.report(
                &mut w.kernel,
                &w.reporter,
                &w.subject.did(),
                ViolationKind::Spam,
                malformed,
                &json!({"x": 1}),
            ),
            Err(CoreError::InvalidSignature)
        );
        // 摘要成形但证据本体缺失：无法复算。
        let (good, _) = evidence("case-4");
        assert_eq!(
            w.office.report(
                &mut w.kernel,
                &w.reporter,
                &w.subject.did(),
                ViolationKind::Spam,
                good,
                &Value::Null,
            ),
            Err(CoreError::Encoding)
        );
        assert_eq!(w.office.event_count(), 0);
        assert_eq!(w.office.case_count(), 0);
    }

    #[test]
    fn self_reports_and_unknown_subjects_are_refused() {
        let mut w = world();
        let (reference, payload) = evidence("case-5");
        assert_eq!(
            w.office.report(
                &mut w.kernel,
                &w.reporter,
                &w.reporter.did(),
                ViolationKind::Spam,
                reference,
                &payload,
            ),
            Err(CoreError::InvalidKind)
        );
        let (reference, payload) = evidence("case-6");
        let ghost = AgentKeys::from_seed(&[0x9e; 32]).did();
        assert_eq!(
            w.office.report(
                &mut w.kernel,
                &w.reporter,
                &ghost,
                ViolationKind::Spam,
                reference,
                &payload,
            ),
            Err(CoreError::UnknownAgent)
        );
        assert_eq!(w.office.event_count(), 0);
        let codes: Vec<RefusalCode> = w.kernel.refusals().iter().map(|(_, r)| r.code).collect();
        assert_eq!(
            codes,
            vec![RefusalCode::PolicyDenied, RefusalCode::StaleEpoch]
        );
    }

    #[test]
    fn an_unregistered_agent_cannot_report() {
        let mut w = world();
        let outsider = AgentKeys::from_seed(&[0x5a; 32]);
        let (reference, payload) = evidence("case-7");
        assert_eq!(
            w.office.report(
                &mut w.kernel,
                &outsider,
                &w.subject.did(),
                ViolationKind::Spam,
                reference,
                &payload,
            ),
            Err(CoreError::UnknownAgent)
        );
        let (_, refusal) = &w.kernel.refusals()[0];
        assert_eq!(refusal.code, RefusalCode::Unauthorized);
        assert_eq!(w.office.event_count(), 0);
    }

    #[test]
    fn consecutive_reports_link_into_one_chain() {
        let mut w = world();
        let third = setup::keys(setup::ROLE_ARBITER);
        setup::ensure_agent(&mut w.kernel, &third, "third", &["x"], Credits(20)).unwrap();
        for (tag, subject) in [("c-1", w.subject.did()), ("c-2", third.did())] {
            let (reference, payload) = evidence(tag);
            w.office
                .report(
                    &mut w.kernel,
                    &w.reporter,
                    &subject,
                    ViolationKind::NonDelivery,
                    reference,
                    &payload,
                )
                .unwrap();
        }
        assert_eq!(w.office.event_count(), 2);
        let events = w.office.events();
        assert_eq!(events[1].prev, events[0].hash);
        assert_eq!(w.office.verify_chain().ok, true);
    }

    #[test]
    fn a_tampered_copy_of_the_chain_is_detected() {
        let mut w = world();
        let (reference, payload) = evidence("case-8");
        w.office
            .report(
                &mut w.kernel,
                &w.reporter,
                &w.subject.did(),
                ViolationKind::Collusion,
                reference,
                &payload,
            )
            .unwrap();
        let mut tampered = w.office.events().to_vec();
        tampered[0].payload = json!({"case": "forged", "report": {}});
        let verdict = verify_chain(&tampered);
        assert!(!verdict.ok);
        assert_eq!(verdict.broken_at, Some(0));
        // 服务自身的链不受影响（篡改的是副本）。
        assert!(w.office.verify_chain().ok);
    }

    #[test]
    fn the_service_identity_must_match_the_configured_did() {
        let service = setup::keys(setup::ROLE_SERVICE);
        let service_did = service.did();
        let impostor = setup::keys(0x7f);
        let config = SafetyConfig::single_arbiter(service_did.clone(), setup::keys(setup::ROLE_ARBITER).did());
        assert_eq!(
            SafetyOffice::new(config.clone(), impostor).err(),
            Some(CoreError::InvalidSignature)
        );
        let office = SafetyOffice::new(config, service).unwrap();
        assert_eq!(office.service_did(), service_did);
    }

    #[test]
    fn ledger_snapshot_is_order_independent_and_fingerprinted() {
        let mut w = world();
        let (reference, payload) = evidence("case-9");
        w.office
            .report(
                &mut w.kernel,
                &w.reporter,
                &w.subject.did(),
                ViolationKind::Spam,
                reference,
                &payload,
            )
            .unwrap();
        let forward = ledger_snapshot(&w.kernel, &w.participants);
        let mut reversed = w.participants.clone();
        reversed.reverse();
        assert_eq!(ledger_snapshot(&w.kernel, &reversed), forward);
        assert_eq!(
            ledger_fingerprint(&w.kernel, &w.participants).unwrap(),
            ledger_fingerprint(&w.kernel, &reversed).unwrap()
        );
    }

    // ---- v1.5.3 申诉 ----

    fn appeal_evidence(tag: &str) -> (Vec<EvidenceRef>, Vec<Value>) {
        let payloads = vec![
            json!({"tag": tag, "kind": "delivery-receipt", "delivered": true}),
            json!({"tag": tag, "kind": "witness-statement", "witness": "peer"}),
        ];
        let references = payloads
            .iter()
            .map(|p| EvidenceRef::commit(EvidenceKind::Witness, p).unwrap())
            .collect();
        (references, payloads)
    }

    fn open_case(w: &mut World, tag: &str) -> String {
        let (reference, payload) = evidence(tag);
        w.office
            .report(
                &mut w.kernel,
                &w.reporter,
                &w.subject.did(),
                ViolationKind::NonDelivery,
                reference,
                &payload,
            )
            .unwrap()
            .id
    }

    #[test]
    fn the_subject_can_appeal_and_the_case_becomes_appealed() {
        let mut w = world();
        let case_id = open_case(&mut w, "appeal-1");
        let (references, payloads) = appeal_evidence("appeal-1");
        let appeal = w
            .office
            .appeal(&mut w.kernel, &w.subject, &case_id, references, &payloads)
            .unwrap();

        assert_eq!(appeal.appellant, w.subject.did());
        assert_eq!(appeal.case, case_id);
        assert_eq!(appeal.evidence_count(), 2);
        appeal.verify().unwrap();

        assert_eq!(w.office.status_of(&case_id), Some(CaseStatus::Appealed));
        assert_eq!(
            w.office.case(&case_id).unwrap().appeals,
            vec![appeal.id.clone()]
        );
        assert_eq!(w.office.appeal_count(), 1);
        assert_eq!(w.office.event_count(), 2);
        assert_eq!(w.office.events()[1].kind, SafetyEventKind::Appealed);
        assert!(w.office.verify_chain().ok);
    }

    #[test]
    fn an_appeal_moves_no_ledger_entry_and_no_card() {
        let mut w = world();
        let case_id = open_case(&mut w, "appeal-2");
        let before = ledger_snapshot(&w.kernel, &w.participants);
        let fingerprint_before = ledger_fingerprint(&w.kernel, &w.participants).unwrap();
        let subject_card_before = w.kernel.card(&w.subject.did()).cloned();

        let (references, payloads) = appeal_evidence("appeal-2");
        w.office
            .appeal(&mut w.kernel, &w.subject, &case_id, references, &payloads)
            .unwrap();

        assert_eq!(ledger_snapshot(&w.kernel, &w.participants), before);
        assert_eq!(
            ledger_fingerprint(&w.kernel, &w.participants).unwrap(),
            fingerprint_before
        );
        assert_eq!(w.kernel.card(&w.subject.did()).cloned(), subject_card_before);
        assert_eq!(w.kernel.ledger().slashed(), Credits::ZERO);
        w.kernel.ledger().check_conservation().unwrap();
    }

    #[test]
    fn a_third_party_cannot_appeal_on_behalf_of_the_subject() {
        let mut w = world();
        let case_id = open_case(&mut w, "appeal-3");
        let (references, payloads) = appeal_evidence("appeal-3");
        // 举报人替被举报人「代为申诉」：签名不是主体的，密码学上就不成立。
        assert_eq!(
            w.office
                .appeal(&mut w.kernel, &w.reporter, &case_id, references, &payloads),
            Err(CoreError::InvalidSignature)
        );
        assert_eq!(w.office.status_of(&case_id), Some(CaseStatus::Reported));
        assert_eq!(w.office.appeal_count(), 0);
        assert_eq!(w.office.event_count(), 1);
        let (_, refusal) = &w.kernel.refusals()[0];
        assert_eq!(refusal.code, RefusalCode::Unauthorized);
    }

    #[test]
    fn an_appeal_for_an_unknown_case_is_refused() {
        let mut w = world();
        let (references, payloads) = appeal_evidence("appeal-4");
        assert_eq!(
            w.office
                .appeal(&mut w.kernel, &w.subject, "no-such-case", references, &payloads),
            Err(CoreError::UnknownAgent)
        );
        let (_, refusal) = &w.kernel.refusals()[0];
        assert_eq!(refusal.code, RefusalCode::StaleEpoch);
        assert_eq!(w.office.event_count(), 0);
    }

    #[test]
    fn forged_appeal_evidence_is_refused_without_state_change() {
        let mut w = world();
        let case_id = open_case(&mut w, "appeal-5");
        let (references, _) = appeal_evidence("appeal-5");
        let other_payloads = vec![json!({"tag": "appeal-5", "forged": true})];
        assert_eq!(
            w.office
                .appeal(&mut w.kernel, &w.subject, &case_id, references, &other_payloads),
            Err(CoreError::InvalidSignature)
        );
        assert_eq!(w.office.status_of(&case_id), Some(CaseStatus::Reported));
        assert_eq!(w.office.appeal_count(), 0);
        assert_eq!(w.office.event_count(), 1);
        let (_, refusal) = &w.kernel.refusals()[0];
        assert_eq!(refusal.code, RefusalCode::PolicyDenied);
    }

    #[test]
    fn an_appeal_without_evidence_or_with_mismatched_payloads_is_refused() {
        let mut w = world();
        let case_id = open_case(&mut w, "appeal-6");
        assert_eq!(
            w.office
                .appeal(&mut w.kernel, &w.subject, &case_id, Vec::new(), &[]),
            Err(CoreError::InvalidSignature)
        );
        let (references, payloads) = appeal_evidence("appeal-6");
        assert_eq!(
            w.office
                .appeal(&mut w.kernel, &w.subject, &case_id, references, &payloads[..1]),
            Err(CoreError::InvalidSignature)
        );
        assert_eq!(w.office.appeal_count(), 0);
    }

    #[test]
    fn appeals_are_repeatable_with_new_evidence() {
        let mut w = world();
        let case_id = open_case(&mut w, "appeal-7");
        let mut ids = Vec::new();
        for tag in ["appeal-7-a", "appeal-7-b"] {
            let (references, payloads) = appeal_evidence(tag);
            let appeal = w
                .office
                .appeal(&mut w.kernel, &w.subject, &case_id, references, &payloads)
                .unwrap();
            ids.push(appeal.id);
        }
        // 终局性属于裁决，不属于申诉人：新证据可以再次申诉。
        assert_eq!(w.office.status_of(&case_id), Some(CaseStatus::Appealed));
        assert_eq!(w.office.case(&case_id).unwrap().appeals, ids);
        let indexed: Vec<String> = w
            .office
            .appeals_of(&case_id)
            .iter()
            .map(|a| a.id.clone())
            .collect();
        assert_eq!(indexed, w.office.case(&case_id).unwrap().appeals);
        assert_eq!(w.office.event_count(), 3);
        assert!(w.office.verify_chain().ok);
    }
}
