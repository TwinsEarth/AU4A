//! v1.5.8 对抗用例：每一个「拒绝路径」都必须留下证据，且不能带来任何副作用。
//!
//! 判定标准是三条同时成立：
//! 1. 调用失败（类型化错误）；
//! 2. 世界没变（账本快照逐字段相等、案件数/事件数不增）；
//! 3. 拒绝被记录（`kernel.refusals()` 里有对应的 `RefusalCode`）。

use au4a_core::{AgentKeys, Credits, RefusalCode};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_safety::{
    ensure_agent, ledger_snapshot, role_keys, setup, verify_chain, ArbitrationVerdict, CaseStatus,
    EvidenceKind, EvidenceRef, PenaltyOrder, SafetyConfig, SafetyOffice, SanctionKind,
    VerdictOutcome, ViolationKind,
};
use serde_json::json;

struct World {
    kernel: Kernel,
    office: SafetyOffice,
    reporter: AgentKeys,
    subject: AgentKeys,
    arbiter: AgentKeys,
    participants: Vec<au4a_core::Did>,
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
    let participants = vec![reporter.did(), subject.did()];
    World {
        kernel,
        office,
        reporter,
        subject,
        arbiter,
        participants,
    }
}

fn open_case(w: &mut World, tag: &str) -> String {
    let payload = json!({"tag": tag, "delivered": false});
    let reference = EvidenceRef::commit(EvidenceKind::Transcript, &payload).unwrap();
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
fn a_forged_evidence_digest_is_refused_without_any_side_effect() {
    let mut w = world();
    let before = ledger_snapshot(&w.kernel, &w.participants);
    let honest = json!({"tag": "forged", "delivered": false});
    let other = json!({"tag": "forged", "delivered": true});
    let reference = EvidenceRef::commit(EvidenceKind::Transcript, &honest).unwrap();

    assert_eq!(
        w.office.report(
            &mut w.kernel,
            &w.reporter,
            &w.subject.did(),
            ViolationKind::FakeEvidence,
            reference,
            &other,
        ),
        Err(au4a_core::CoreError::InvalidSignature)
    );
    assert_eq!(ledger_snapshot(&w.kernel, &w.participants), before);
    assert_eq!(w.office.case_count(), 0);
    assert_eq!(w.office.event_count(), 0);
    let (_, refusal) = w.kernel.refusals().last().unwrap();
    assert_eq!(refusal.code, RefusalCode::PolicyDenied);
}

#[test]
fn a_third_party_appeal_and_a_third_party_unsubscribe_are_both_refused() {
    let mut w = world();
    let case_id = open_case(&mut w, "adv-appeal");
    let subscription = w
        .office
        .subscribe(
            &mut w.kernel,
            &w.reporter,
            &case_id,
            vec![CaseStatus::Appealed],
        )
        .unwrap();
    let before = ledger_snapshot(&w.kernel, &w.participants);

    let payload = json!({"receipt": true});
    let evidence = vec![EvidenceRef::commit(EvidenceKind::Witness, &payload).unwrap()];
    // 举报人替被举报人申诉：签名不是主体的，不成立。
    assert!(w
        .office
        .appeal(&mut w.kernel, &w.reporter, &case_id, evidence, &[payload])
        .is_err());
    // 举报人替被举报人退订：订阅不是他的。
    assert!(w
        .office
        .unsubscribe(&mut w.kernel, &w.subject, &subscription.id)
        .is_err());
    assert_eq!(w.office.status_of(&case_id), Some(CaseStatus::Reported));
    assert_eq!(w.office.subscription_count(), 1);
    assert_eq!(ledger_snapshot(&w.kernel, &w.participants), before);
    assert!(w
        .kernel
        .refusals()
        .iter()
        .any(|(_, r)| r.code == RefusalCode::Unauthorized));
}

#[test]
fn a_penalty_order_without_arbiter_authority_changes_nothing() {
    let mut w = world();
    let case_id = open_case(&mut w, "adv-penalty");
    let before = ledger_snapshot(&w.kernel, &w.participants);

    let impostor = AgentKeys::from_seed(&[0xA1; 32]);
    let order = PenaltyOrder::new(
        case_id.clone(),
        w.subject.did(),
        SanctionKind::StakeSlash,
        Credits(5),
        impostor.did(),
        w.kernel.now(),
    )
    .sign(&impostor)
    .unwrap();
    assert!(w.office.apply_penalty_order(&mut w.kernel, order).is_err());
    assert_eq!(w.office.status_of(&case_id), Some(CaseStatus::Reported));
    assert_eq!(w.office.penalty_count(), 0);
    assert_eq!(ledger_snapshot(&w.kernel, &w.participants), before);
    assert_eq!(w.kernel.ledger().slashed(), Credits::ZERO);
}

#[test]
fn a_forged_verdict_cannot_move_the_ledger_or_close_the_case() {
    let mut w = world();
    let case_id = open_case(&mut w, "adv-verdict");
    let before = ledger_snapshot(&w.kernel, &w.participants);

    let impostor = AgentKeys::from_seed(&[0xA2; 32]);
    let rationale = au4a_core::canonical_hash(&json!({"reason": "trust me"})).unwrap();
    let verdict = ArbitrationVerdict::new(
        case_id.clone(),
        VerdictOutcome::Upheld,
        SanctionKind::StakeSlash,
        Credits(20),
        rationale,
        impostor.did(),
        w.kernel.now(),
    )
    .sign(&impostor)
    .unwrap();
    assert!(w.office.resolve(&mut w.kernel, verdict).is_err());
    assert_eq!(w.office.status_of(&case_id), Some(CaseStatus::Reported));
    assert_eq!(ledger_snapshot(&w.kernel, &w.participants), before);
    assert_eq!(w.kernel.ledger().slashed(), Credits::ZERO);
}

#[test]
fn a_tampered_chain_is_detected_at_every_position() {
    let mut w = world();
    open_case(&mut w, "adv-chain-1");
    let case_two = open_case(&mut w, "adv-chain-2");
    w.office
        .subscribe(
            &mut w.kernel,
            &w.reporter,
            &case_two,
            vec![CaseStatus::Reported],
        )
        .unwrap();
    let events = w.office.events().to_vec();
    assert!(verify_chain(&events).ok);

    for position in 0..events.len() {
        let mut tampered = events.clone();
        tampered[position].payload = json!({"forged": position});
        let verdict = verify_chain(&tampered);
        assert!(!verdict.ok, "第 {position} 条被篡改必须被发现");
        assert_eq!(verdict.broken_at, Some(position as u64));
    }
}

#[test]
fn an_unconfirmed_report_never_touches_balances_or_cards() {
    let mut w = world();
    let before = ledger_snapshot(&w.kernel, &w.participants);
    let card_before = w.kernel.card(&w.subject.did()).cloned();
    let case_id = open_case(&mut w, "adv-unconfirmed");
    let payload = json!({"receipt": "late"});
    let evidence = vec![EvidenceRef::commit(EvidenceKind::Witness, &payload).unwrap()];
    w.office
        .appeal(&mut w.kernel, &w.subject, &case_id, evidence, &[payload])
        .unwrap();

    assert_eq!(w.office.status_of(&case_id), Some(CaseStatus::Appealed));
    assert_eq!(ledger_snapshot(&w.kernel, &w.participants), before);
    assert_eq!(w.kernel.card(&w.subject.did()).cloned(), card_before);
    assert_eq!(w.kernel.ledger().slashed(), Credits::ZERO);
    w.kernel.ledger().check_conservation().unwrap();
}

#[test]
fn subscriptions_do_not_leak_between_agents_or_statuses() {
    let mut w = world();
    let case_id = open_case(&mut w, "adv-notify");
    let watcher = AgentKeys::from_seed(&[0xA3; 32]);
    ensure_agent(
        &mut w.kernel,
        &watcher,
        "watcher",
        &["observe"],
        Credits(20),
    )
    .unwrap();
    w.office
        .subscribe(
            &mut w.kernel,
            &w.reporter,
            &case_id,
            vec![CaseStatus::Penalized],
        )
        .unwrap();
    w.office
        .subscribe(
            &mut w.kernel,
            &watcher,
            &case_id,
            vec![CaseStatus::Arbitrated],
        )
        .unwrap();

    // 只有 reporter 订阅了 penalized：处罚后两人都不该收到 arbitrated。
    let order = PenaltyOrder::new(
        case_id.clone(),
        w.subject.did(),
        SanctionKind::StakeSlash,
        Credits(2),
        w.arbiter.did(),
        w.kernel.now(),
    )
    .sign(&w.arbiter)
    .unwrap();
    w.office.apply_penalty_order(&mut w.kernel, order).unwrap();
    assert_eq!(w.office.inbox(&w.reporter.did()).len(), 1);
    assert_eq!(w.office.inbox(&watcher.did()).len(), 0);
    assert_eq!(w.office.inbox(&w.subject.did()).len(), 0);
}
