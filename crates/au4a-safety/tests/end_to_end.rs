//! v1.5.8 端到端：从权限查询到终局裁决的完整案件生命周期。
//!
//! 这里刻意**只用公开 API**（`au4a_safety::*` + `au4a_kernel::*`），
//! 不用 crate 内部结构，因此它同时是对外接口的可达性证明：
//! 文档里写的每一步，外部调用者都能照着做出来。

use au4a_core::{AgentKeys, Credits};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_safety::{
    classify_safety_message, ensure_agent, ledger_snapshot, query_envelope, role_keys, setup,
    ArbitrationRequest, ArbitrationVerdict, CaseStatus, EvidenceKind, EvidenceRef, PenaltyOrder,
    Permission, PermissionBoundary, SafetyConfig, SafetyOffice, SanctionKind, VerdictOutcome,
    ViolationKind, ViolationReport,
};
use serde_json::json;

struct World {
    kernel: Kernel,
    office: SafetyOffice,
    reporter: AgentKeys,
    subject: AgentKeys,
    arbiter: AgentKeys,
}

fn world() -> World {
    let mut kernel = Kernel::new(KernelConfig::default());
    let service = role_keys(setup::ROLE_SERVICE);
    let reporter = role_keys(setup::ROLE_REPORTER);
    let subject = role_keys(setup::ROLE_SUBJECT);
    let arbiter = role_keys(setup::ROLE_ARBITER);
    for (keys, display, skill) in [
        (&service, "safety-service", "safety.api"),
        (&reporter, "reporter-agent", "audit.report"),
        (&subject, "subject-agent", "deliver.task"),
    ] {
        ensure_agent(&mut kernel, keys, display, &[skill], Credits(20)).unwrap();
    }
    let config = SafetyConfig::single_arbiter(service.did(), arbiter.did());
    let office = SafetyOffice::new(config, service).unwrap();
    World {
        kernel,
        office,
        reporter,
        subject,
        arbiter,
    }
}

#[test]
fn the_full_lifecycle_reaches_an_arbitrated_case() {
    let mut w = world();
    let participants = vec![w.reporter.did(), w.subject.did()];

    // 1) 权限查询：Agent 自己看边界（免费、结构化）。
    let boundary = PermissionBoundary::of(
        &w.kernel,
        w.office.config(),
        &w.reporter.did(),
    );
    assert!(boundary.allows(Permission::ReportViolation));
    assert!(!boundary.allows(Permission::IssueVerdict));

    // 2) 查询走 PMB：真实信封 + 服务回执。
    let query = query_envelope(
        &w.reporter,
        &w.office.service_did(),
        w.kernel.now(),
        &w.reporter.did(),
    )
    .unwrap();
    w.kernel.send(&query).unwrap();
    let queued = w.kernel.drain();
    assert_eq!(queued.len(), 1);
    let receipt = w.office.handle(&mut w.kernel, &queued[0]).unwrap().unwrap();
    receipt.verify().unwrap();
    assert_eq!(receipt.in_reply_to.as_deref(), Some(query.id.as_str()));
    assert_eq!(receipt.body["result"]["boundary"]["registered"], json!(true));

    // 3) 未确认举报：不动账本。
    let before_report = ledger_snapshot(&w.kernel, &participants);
    let evidence_payload = json!({"task": "deliver-7", "delivered": false});
    let evidence = EvidenceRef::commit(EvidenceKind::Transcript, &evidence_payload).unwrap();
    let report = w
        .office
        .report(
            &mut w.kernel,
            &w.reporter,
            &w.subject.did(),
            ViolationKind::NonDelivery,
            evidence,
            &evidence_payload,
        )
        .unwrap();
    assert_eq!(w.office.status_of(&report.id), Some(CaseStatus::Reported));
    assert_eq!(ledger_snapshot(&w.kernel, &participants), before_report);
    assert_eq!(w.kernel.ledger().slashed(), Credits::ZERO);

    // 4) 通知：举报人订阅本案状态。
    let subscription = w
        .office
        .subscribe(
            &mut w.kernel,
            &w.reporter,
            &report.id,
            vec![CaseStatus::Reported, CaseStatus::Penalized, CaseStatus::Arbitrated],
        )
        .unwrap();
    assert_eq!(w.office.inbox(&w.reporter.did()).len(), 1, "订阅即回放当前状态");

    // 5) 处罚：仲裁者签名的数据契约，账本唯一改动入口。
    let order = PenaltyOrder::new(
        report.id.clone(),
        w.subject.did(),
        SanctionKind::StakeSlash,
        Credits(5),
        w.arbiter.did(),
        w.kernel.now(),
    )
    .sign(&w.arbiter)
    .unwrap();
    let penalty = w.office.apply_penalty_order(&mut w.kernel, order).unwrap();
    assert_eq!(penalty.applied, Credits(5));
    assert_eq!(w.kernel.ledger().slashed(), Credits(5));
    assert_eq!(w.office.status_of(&report.id), Some(CaseStatus::Penalized));

    // 6) 申诉：主体本人签名，只提交证据、推状态，不动账本。
    let appeal_payload = json!({"receipt": "signed-by-receiver"});
    let appeal_evidence = vec![
        EvidenceRef::commit(EvidenceKind::Witness, &appeal_payload).unwrap(),
    ];
    let after_penalty = ledger_snapshot(&w.kernel, &participants);
    let appeal = w
        .office
        .appeal(
            &mut w.kernel,
            &w.subject,
            &report.id,
            appeal_evidence,
            &[appeal_payload],
        )
        .unwrap();
    assert_eq!(w.office.status_of(&report.id), Some(CaseStatus::Appealed));
    assert_eq!(ledger_snapshot(&w.kernel, &participants), after_penalty);

    // 7) 仲裁：事实交出去（数据契约），裁决带仲裁者签名回来。
    let request: ArbitrationRequest = w.office.arbitration_request(&report.id).unwrap();
    assert_eq!(request.case, report.id);
    assert_eq!(request.penalties, vec![penalty.id.clone()]);
    assert_eq!(request.appeals, vec![appeal.id.clone()]);
    assert_eq!(
        ArbitrationRequest::from_json(&request.to_json().unwrap()).unwrap(),
        request
    );

    let rationale = au4a_core::canonical_hash(&json!({"reason": "appeal receipt stands"})).unwrap();
    let verdict = ArbitrationVerdict::new(
        request.case.clone(),
        VerdictOutcome::Upheld,
        SanctionKind::StakeSlash,
        Credits(3),
        rationale,
        w.arbiter.did(),
        w.kernel.now(),
    )
    .sign(&w.arbiter)
    .unwrap();
    let outcome = w.office.resolve(&mut w.kernel, verdict).unwrap();

    // 8) 终局：状态、结论、罚没、通知、链、守恒。
    assert_eq!(outcome.outcome, VerdictOutcome::Upheld);
    assert_eq!(outcome.status, CaseStatus::Arbitrated);
    let case = w.office.case(&report.id).unwrap();
    assert_eq!(case.status, CaseStatus::Arbitrated);
    assert_eq!(case.outcome, Some(VerdictOutcome::Upheld));
    assert_eq!(case.appeals.len(), 1);
    assert_eq!(case.penalties.len(), 2);
    assert_eq!(w.kernel.ledger().slashed(), Credits(8));
    assert_eq!(w.office.slashed_total().unwrap(), Credits(8));
    assert!(w.office.verify_chain().ok);
    assert_eq!(w.office.event_count(), 5, "reported/subscribed/penalized/appealed/arbitrated");
    w.kernel.ledger().check_conservation().unwrap();

    let delivered: Vec<&str> = w
        .office
        .inbox(&w.reporter.did())
        .iter()
        .map(|n| n.status.as_str())
        .collect();
    assert_eq!(delivered, vec!["reported", "penalized", "arbitrated"]);
    assert_eq!(subscription.statuses.len(), 3);
}

#[test]
fn a_rejected_appeal_returns_the_stake_and_keeps_the_chain_intact() {
    let mut w = world();
    let evidence_payload = json!({"task": "deliver-9", "delivered": false});
    let reference = EvidenceRef::commit(EvidenceKind::Transcript, &evidence_payload).unwrap();
    let report = w
        .office
        .report(
            &mut w.kernel,
            &w.reporter,
            &w.subject.did(),
            ViolationKind::Fraud,
            reference,
            &evidence_payload,
        )
        .unwrap();

    let locked_before = w.kernel.ledger().balance(&w.subject.did()).locked;
    let order = PenaltyOrder::new(
        report.id.clone(),
        w.subject.did(),
        SanctionKind::StakeSlash,
        Credits(12),
        w.arbiter.did(),
        w.kernel.now(),
    )
    .sign(&w.arbiter)
    .unwrap();
    w.office.apply_penalty_order(&mut w.kernel, order).unwrap();
    assert_eq!(
        w.kernel.ledger().balance(&w.subject.did()).locked,
        Credits(8)
    );

    let rationale = au4a_core::canonical_hash(&json!({"reason": "no violation"})).unwrap();
    let verdict = ArbitrationVerdict::new(
        report.id.clone(),
        VerdictOutcome::Rejected,
        SanctionKind::Warning,
        Credits::ZERO,
        rationale,
        w.arbiter.did(),
        w.kernel.now(),
    )
    .sign(&w.arbiter)
    .unwrap();
    let outcome = w.office.resolve(&mut w.kernel, verdict).unwrap();

    assert_eq!(outcome.refunded, Credits(12));
    assert_eq!(
        w.kernel.ledger().balance(&w.subject.did()).locked,
        locked_before,
        "归还必须回到锁定质押，而不是变成可用余额"
    );
    assert_eq!(w.office.slashed_total().unwrap(), Credits::ZERO);
    assert_eq!(w.office.refunded_total().unwrap(), Credits(12));
    // 归还后仍然保有全部权限：被误判不会导致二次误伤。
    let boundary = PermissionBoundary::of(&w.kernel, w.office.config(), &w.subject.did());
    assert!(boundary.allows(Permission::ReportViolation));
    assert!(w.office.verify_chain().ok);
    w.kernel.ledger().check_conservation().unwrap();
}

#[test]
fn the_pmb_path_and_the_local_path_accept_the_same_record() {
    let mut w = world();
    let payload = json!({"task": "deliver-11", "delivered": false});
    let reference = EvidenceRef::commit(EvidenceKind::Transcript, &payload).unwrap();
    let report = ViolationReport::new(
        w.reporter.did(),
        w.subject.did(),
        ViolationKind::Spam,
        reference,
        w.kernel.now(),
    )
    .sign(&w.reporter)
    .unwrap();
    let envelope = au4a_safety::report_envelope(
        &w.reporter,
        &w.office.service_did(),
        w.kernel.now(),
        &report,
        &payload,
    )
    .unwrap();
    assert_eq!(
        classify_safety_message(&envelope).unwrap().kind(),
        au4a_safety::safety_kinds::SAFETY_REPORT
    );
    w.office
        .accept_report(&mut w.kernel, &report, &payload)
        .unwrap();
    assert_eq!(w.office.status_of(&report.id), Some(CaseStatus::Reported));    // 同一份举报再走一次：案件 id 相同（内容寻址），但事件链各自追加。
    w.office.handle(&mut w.kernel, &envelope).unwrap();
    assert_eq!(w.office.event_count(), 2);
    assert!(w.office.verify_chain().ok);
}
