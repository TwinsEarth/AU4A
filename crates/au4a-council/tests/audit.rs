//! v1.7.10 审计——集成测试。
//!
//! 断言两件事：**哈希链不可悄悄改写**（改一条、删一条、换顺序都会断链，并指出第一处位置），
//! 以及**只读导出**确实只是快照（导出不改变治理状态，且人类能读到提案/否决/紧急指令）。

use au4a_core::{AgentKeys, Credits, Did};
use au4a_council::audit::{AuditEntry, AuditLog};
use au4a_council::{
    Action, AgentIdentity, ChainBinding, Choice, CommitteeKind, Council, CouncilConfig,
    ElectionBallot, ElectionConfig, EmergencyApproval, EmergencyStatus, GovernorToken,
    HumanObserver, ProposalDraft, Vote,
};
use au4a_kernel::{Kernel, KernelConfig};

fn keys(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

struct Scene {
    council: Council,
}

/// 一个有内容的治理现场：一条已执行的动议、一条被否决阻断的动议、一条已确认的紧急指令。
fn scene() -> Scene {
    let mut kernel = Kernel::new(KernelConfig::default());
    let mut council = Council::new(CouncilConfig {
        election: ElectionConfig {
            seats: 4,
            ..ElectionConfig::default()
        },
        ..CouncilConfig::default()
    });
    let agents: Vec<AgentKeys> = (0..4).map(keys).collect();
    for k in &agents {
        kernel
            .register(k, "t", &["governance.vote"], Credits(20))
            .expect("register");
        council.note_reputation(&k.did(), 5_000);
        council.note_uptime(&k.did(), 300);
    }
    let picks: Vec<Did> = agents.iter().map(|k| k.did()).collect();
    for kind in CommitteeKind::ALL {
        let ballots: Vec<ElectionBallot> = agents
            .iter()
            .map(|k| ElectionBallot::cast(k, kind, &picks).expect("cast"))
            .collect();
        council.elect(&mut kernel, kind, &ballots).expect("elect");
    }
    let identity = AgentIdentity::from_keys(&agents[0]);

    // 动议甲：提案 → 表决 → 执行。
    let draft = ProposalDraft::by(
        &agents[0],
        CommitteeKind::Task,
        "审计用动议甲",
        Action::SetPolicy {
            key: String::from("audit_a"),
            value: 1,
        },
    )
    .expect("draft");
    let first = council
        .propose(&mut kernel, &identity, draft)
        .expect("propose");
    let round = council.open_round(&mut kernel, &first.id).expect("open");
    for i in 0..3 {
        let vote = Vote::cast(&agents[i], &first.id, round.round, Choice::Yes).expect("cast");
        council.cast_vote(&mut kernel, vote).expect("vote");
    }
    council
        .execute(&mut kernel, &identity, &first.id)
        .expect("execute");

    // 动议乙：提案 → 表决 → 人类否决。
    let draft = ProposalDraft::by(
        &agents[0],
        CommitteeKind::Task,
        "审计用动议乙",
        Action::SetPolicy {
            key: String::from("audit_b"),
            value: 2,
        },
    )
    .expect("draft");
    let second = council
        .propose(&mut kernel, &identity, draft)
        .expect("propose");
    let round = council.open_round(&mut kernel, &second.id).expect("open");
    for i in 0..3 {
        let vote = Vote::cast(&agents[i], &second.id, round.round, Choice::Yes).expect("cast");
        council.cast_vote(&mut kernel, vote).expect("vote");
    }
    let human = HumanObserver::new("auditor");
    let veto = human
        .veto(&council, &second.id, "审计测试：理由公开")
        .expect("veto");
    council.apply_veto(&mut kernel, &veto).expect("apply");

    // 紧急指令：安全委员会下发 + 全体确认。
    let directive = council
        .issue_emergency(
            &mut kernel,
            &identity,
            "audit_freeze",
            1,
            "审计测试：紧急冻结",
        )
        .expect("issue");
    let approvals: Vec<EmergencyApproval> = agents
        .iter()
        .map(|k| EmergencyApproval::cast(k, &directive.id, true).expect("cast"))
        .collect();
    council
        .confirm_emergency(&mut kernel, &directive.id, &approvals)
        .expect("confirm");

    Scene { council }
}

#[test]
fn the_audit_chain_rebuilds_from_events_and_verifies() {
    let s = scene();
    let log = s.council.audit_log().expect("audit_log");
    assert_eq!(log.len(), s.council.events().len());
    assert!(log.len() > 5);
    assert_eq!(log.root().len(), 64);
    let verdict = log.verify();
    assert!(verdict.ok, "{}", verdict.reason);
    assert_eq!(verdict.checked, log.len());
    assert_eq!(verdict.broken_at, None);
    // 第 0 条的前驱是创世哈希。
    assert_eq!(log.entries()[0].prev_hash, au4a_council::audit::GENESIS);
    // 重建是确定性的。
    let again = s.council.audit_log().expect("audit_log");
    assert_eq!(log, again);
}

#[test]
fn tampering_with_any_entry_breaks_the_chain_at_that_position() {
    let s = scene();
    let log = s.council.audit_log().expect("audit_log");
    for index in [0usize, 1, log.len() / 2, log.len() - 1] {
        let mut entries: Vec<AuditEntry> = log.entries().to_vec();
        entries[index].detail = String::from("被篡改的细节");
        let tampered = AuditLog::from_entries(entries);
        let verdict = tampered.verify();
        assert!(!verdict.ok, "第 {index} 条被改写后必须断链");
        assert_eq!(verdict.broken_at, Some(index), "必须指出第一处断裂位置");
    }
}

#[test]
fn deleting_or_reordering_entries_breaks_the_chain() {
    let s = scene();
    let log = s.council.audit_log().expect("audit_log");

    // 删掉中间一条：后续 seq 与前驱哈希都会对不上。
    let mut dropped: Vec<AuditEntry> = log.entries().to_vec();
    dropped.remove(2);
    let verdict = AuditLog::from_entries(dropped).verify();
    assert!(!verdict.ok);
    assert_eq!(verdict.broken_at, Some(2));

    // 交换两条：第一条就对不上。
    let mut swapped: Vec<AuditEntry> = log.entries().to_vec();
    swapped.swap(1, 2);
    let verdict = AuditLog::from_entries(swapped).verify();
    assert!(!verdict.ok);
    assert_eq!(verdict.broken_at, Some(1));

    // 改写历史里的某个字段（subject）同样断链。
    let mut subject_changed: Vec<AuditEntry> = log.entries().to_vec();
    subject_changed[0].subject = String::from("幽灵动议");
    let verdict = AuditLog::from_entries(subject_changed).verify();
    assert!(!verdict.ok);
    assert_eq!(verdict.broken_at, Some(0));
}

#[test]
fn an_untouched_empty_chain_verifies() {
    let empty = AuditLog::from_entries(Vec::new());
    assert!(empty.is_empty());
    assert_eq!(empty.root(), au4a_council::audit::GENESIS);
    let verdict = empty.verify();
    assert!(verdict.ok);
    assert_eq!(verdict.checked, 0);
}

#[test]
fn the_export_is_read_only_and_carries_the_whole_governance_record() {
    let s = scene();
    let before = au4a_council::invariants::state_digest(&s.council).expect("digest");
    let exported = s.council.audit_export().expect("export");
    let after = au4a_council::invariants::state_digest(&s.council).expect("digest");
    assert_eq!(before, after, "只读导出不得改变治理状态");

    assert!(exported.chain_ok);
    assert!(exported.is_healthy(), "{:?}", exported.invariants);
    assert_eq!(exported.track, "1.7");
    assert_eq!(exported.audit_entries, s.council.events().len());
    assert_eq!(exported.audit_root.len(), 64);
    assert_eq!(exported.state_digest, before);

    // 提案快照：一条 executed、一条 blocked。
    assert_eq!(exported.proposals.len(), 2);
    assert_eq!(
        exported.proposals[0].state,
        au4a_council::ProposalState::Executed
    );
    assert_eq!(
        exported.proposals[1].state,
        au4a_council::ProposalState::Blocked
    );
    assert!(exported.proposals[0].action.contains("audit_a=1"));

    // 否决快照：理由公开、观察者是人类标签（不是 DID）。
    assert_eq!(exported.vetoes.len(), 1);
    assert_eq!(exported.vetoes[0].observer, "auditor");
    assert_eq!(exported.vetoes[0].reason, "审计测试：理由公开");
    assert!(!exported.vetoes[0].observer.starts_with("did:"));

    // 紧急指令快照：状态与确认摘要。
    assert_eq!(exported.emergency.len(), 1);
    assert_eq!(
        exported.emergency[0].status,
        EmergencyStatus::Confirmed.as_str()
    );
    assert!(exported.emergency[0]
        .confirmation
        .as_deref()
        .unwrap_or_default()
        .contains("quorum="));

    // 执行收据与策略表。
    assert_eq!(exported.executions.len(), 1);
    assert!(exported.executions[0].conservation_ok);
    assert!(exported
        .policies
        .iter()
        .any(|(k, v)| k == "audit_a" && *v == 1));

    // JSON 投影可序列化并包含全部字段。
    let value = exported.to_json().expect("json");
    assert_eq!(value["track"], "1.7");
    assert_eq!(value["chain_ok"], true);
    assert_eq!(value["vetoes"][0]["reason"], "审计测试：理由公开");
    assert_eq!(value["emergency"][0]["key"], "audit_freeze");
}

#[test]
fn audit_checks_and_self_check_carry_the_chain_evidence() {
    let s = scene();
    let checks = au4a_council::audit::audit_checks(&s.council);
    assert_eq!(checks.len(), 2);
    assert!(au4a_core::all_passed(&checks), "{checks:?}");
    assert!(checks.iter().any(|c| c.name == "council.audit.chain"));
    assert!(checks.iter().any(|c| c.name == "council.audit.export"));

    let all = au4a_council::self_check();
    assert!(au4a_core::all_passed(&all), "self_check 未全绿：{all:?}");
    assert!(all.iter().any(|c| c.name == "council.audit.chain"));
    assert!(all.iter().any(|c| c.name == "council.audit.export"));

    let results = au4a_council::results_json().expect("results");
    assert_eq!(results["audit"]["chain_ok"], true);
    assert_eq!(results["audit"]["entries"], results["events"]);
    assert_eq!(results["audit"]["export_proposals"], 1);
}

#[test]
fn the_export_surfaces_emergency_state_in_the_governor_mapping() {
    // 紧急指令改的是策略，不是动议；GovernorToken 只映射动议，二者互不混淆。
    let s = scene();
    let tokens: Vec<GovernorToken> = au4a_council::project_all(&s.council).expect("tokens");
    assert_eq!(tokens.len(), 2);
    assert!(au4a_council::no_false_chain_claims(&tokens));
    assert_eq!(tokens[0].binding, ChainBinding::default());
    let exported = s.council.audit_export().expect("export");
    assert_eq!(exported.proposals.len(), tokens.len());
}
