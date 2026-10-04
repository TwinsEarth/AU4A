//! v1.0.4 Agent 委员会的集成测试（只用公开 API）。
//!
//! 要证明的是治理底线：**隔离只能由 Agent 委员会对恶意行为作出**；
//! 竞争行为（限流、超时、容量、策略拒绝……）即使重复到阈值，也只能到警告动议。

use au4a_core::{AgentKeys, Credits, Envelope, Escalation, RefusalCode};
use au4a_kernel::{
    motions_for_kernel, Ballot, Council, CouncilConfig, CouncilFailure, Kernel, KernelConfig,
    Motion, MotionKind, Verdict, Vote,
};

fn seeded_kernel(n: u8) -> Kernel {
    let mut k = Kernel::new(KernelConfig::default());
    for i in 0..n {
        let keys = AgentKeys::from_seed(&[40 + i; 32]);
        k.register(&keys, format!("agent-{i}"), &["x"], Credits(20)).unwrap();
    }
    k
}

fn keys(i: u8) -> AgentKeys {
    AgentKeys::from_seed(&[40 + i; 32])
}

#[test]
fn sortition_draws_only_registered_agents_and_is_reproducible() {
    let k = seeded_kernel(8);
    let a = Council::sortition(&k, 1, CouncilConfig::default()).unwrap();
    let b = Council::sortition(&k, 1, CouncilConfig::default()).unwrap();
    assert_eq!(a.members(), b.members());
    assert_eq!(a.members().len(), 5);
    for did in a.members() {
        assert!(k.is_registered(did));
    }
    // 换一个 epoch：另一份名单，但同样可复算。
    let c = Council::sortition(&k, 2, CouncilConfig::default()).unwrap();
    let d = Council::sortition(&k, 2, CouncilConfig::default()).unwrap();
    assert_eq!(c.members(), d.members());
    assert!(c.members().iter().all(|did| k.is_registered(did)));
}

#[test]
fn only_misconduct_evidence_can_produce_a_quarantine_motion() {
    let mut k = seeded_kernel(4);
    let offender = keys(0);
    let racer = keys(1);

    // 恶意：伪造信封（改 body）→ unauthorized → 单次即恶意。
    let mut forged = Envelope::new(
        offender.did(),
        None,
        "progress.event",
        k.now() + 1,
        None,
        serde_json::json!({}),
    )
    .unwrap()
    .seal(&offender)
    .unwrap();
    forged.body = serde_json::json!({"tampered": true});
    assert!(k.send(&forged).is_err());

    // 竞争：证据闸门拒绝（policy_denied）重复到阈值 → 只到 Warn。
    let target = keys(3).did();
    for _ in 0..au4a_core::refusal::REPEAT_THRESHOLD {
        assert!(k
            .settle(&racer.did(), &target, Credits(1), au4a_core::EvidenceGrade::Unverified)
            .is_err());
    }

    let motions = motions_for_kernel(&k, 10).unwrap();
    let quarantine: Vec<&Motion> = motions
        .iter()
        .filter(|m| m.kind == MotionKind::Quarantine)
        .collect();
    assert_eq!(quarantine.len(), 1);
    assert_eq!(quarantine[0].subject, offender.did());
    assert_eq!(quarantine[0].cause, RefusalCode::Unauthorized);

    let warn: Vec<&Motion> = motions.iter().filter(|m| m.kind == MotionKind::Warn).collect();
    assert_eq!(warn.len(), 1);
    assert_eq!(warn[0].subject, racer.did());
    assert_eq!(warn[0].cause, RefusalCode::PolicyDenied);

    // 即使上游误传 Quarantine，竞争码也拿不到隔离动议。
    for code in RefusalCode::ALL.iter().filter(|c| !c.is_misconduct()) {
        assert_eq!(
            Motion::from_escalation(&racer.did(), *code, Escalation::Quarantine, 1).unwrap(),
            None,
            "竞争码 {} 不能隔离",
            code.as_str()
        );
    }
    // 隔离动议的内容寻址 id 必须自洽（不可篡改）。
    assert!(quarantine[0].is_intact());
}

#[test]
fn the_council_reaches_a_verdict_by_quorum_and_records_it() {
    let k = seeded_kernel(5);
    let mut council = Council::sortition(&k, 1, CouncilConfig::default()).unwrap();
    assert_eq!(council.quorum_required(), 3);

    let subject = keys(0).did();
    let motion = Motion::new(
        MotionKind::Warn,
        subject.clone(),
        RefusalCode::PolicyDenied,
        Some(Escalation::Warn),
        1,
    )
    .unwrap();
    council.submit(motion.clone()).unwrap();

    let members: Vec<_> = council.members().to_vec();
    for (i, voter) in members.iter().enumerate() {
        council
            .vote(Vote {
                voter: voter.clone(),
                motion: motion.id.clone(),
                ballot: if i < 3 { Ballot::Uphold } else { Ballot::Reject },
                at: i as u64,
            })
            .unwrap();
    }
    let tally = council.decide(&motion.id, 42);
    assert!(tally.quorum_met);
    assert_eq!((tally.uphold, tally.reject), (3, 2));
    assert_eq!(tally.verdict, Verdict::Upheld);
    assert_eq!(council.decisions().len(), 1);
    assert_eq!(council.decisions()[0].subject, subject);
    assert_eq!(council.decisions()[0].kind, MotionKind::Warn);

    let json = council.decisions_json().unwrap();
    assert_eq!(json["failures"].as_array().map(|a| a.len()), Some(0));
    assert!(json["fingerprint"].as_str().unwrap_or("").len() >= 16);
}

#[test]
fn non_members_and_double_voters_are_refused_with_named_failures() {
    let k = seeded_kernel(3);
    let mut council = Council::sortition(&k, 1, CouncilConfig::default()).unwrap();
    let motion = Motion::new(
        MotionKind::Quarantine,
        keys(0).did(),
        RefusalCode::Malformed,
        None,
        1,
    )
    .unwrap();
    council.submit(motion.clone()).unwrap();

    let outsider = AgentKeys::from_seed(&[99u8; 32]).did();
    assert_eq!(
        council.vote(Vote {
            voter: outsider,
            motion: motion.id.clone(),
            ballot: Ballot::Uphold,
            at: 1,
        }),
        Err(CouncilFailure::NonMemberVote)
    );
    assert!(CouncilFailure::NonMemberVote.to_refusal_code().is_misconduct());

    let member = council.members()[0].clone();
    assert!(council
        .vote(Vote {
            voter: member.clone(),
            motion: motion.id.clone(),
            ballot: Ballot::Uphold,
            at: 2,
        })
        .is_ok());
    assert_eq!(
        council.vote(Vote {
            voter: member,
            motion: motion.id.clone(),
            ballot: Ballot::Reject,
            at: 3,
        }),
        Err(CouncilFailure::DuplicateVote)
    );
    assert!(CouncilFailure::DuplicateVote.to_refusal_code().is_misconduct());
    assert_eq!(council.failures().len(), 2);

    // 竞争型失败模式不会变成恶意码。
    for failure in [
        CouncilFailure::UnknownMotion,
        CouncilFailure::DuplicateMotion,
        CouncilFailure::NoQuorum,
        CouncilFailure::TieInsufficient,
        CouncilFailure::EmptyCouncil,
    ] {
        assert!(!failure.to_refusal_code().is_misconduct(), "{}", failure.as_str());
    }
}

#[test]
fn a_council_without_agents_cannot_decide_anything() {
    let empty = Kernel::new(KernelConfig::default());
    let council = Council::sortition(&empty, 1, CouncilConfig::default()).unwrap();
    assert!(council.members().is_empty());
    let tally = council.tally("any");
    assert_eq!(tally.failure, Some(CouncilFailure::EmptyCouncil));
    assert_eq!(tally.verdict, Verdict::Inconclusive);
    assert_eq!(CouncilFailure::EmptyCouncil.to_refusal_code(), RefusalCode::Degraded);
}
