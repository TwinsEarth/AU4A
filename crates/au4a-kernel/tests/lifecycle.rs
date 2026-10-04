//! v1.0.7 生命周期状态机的集成测试（只用公开 API）。
//!
//! 底线：**竞争只能降级（可恢复），隔离只来自恶意证据或委员会决定**；退役是终态。

use au4a_core::{all_passed, AgentKeys, Credits, Envelope, RefusalCode};
use au4a_kernel::{
    AgentState, Decision, Kernel, KernelConfig, LifecycleEvent, Motion, MotionKind, Verdict,
};

fn keys(tag: u8) -> AgentKeys {
    AgentKeys::from_seed(&[tag; 32])
}

fn seeded(n: u8) -> Kernel {
    let mut k = Kernel::new(KernelConfig::default());
    for i in 0..n {
        k.register(&keys(190 + i), format!("agent-{i}"), &["x"], Credits(20))
            .unwrap();
    }
    k
}

#[test]
fn registration_admits_agents_and_the_audit_tracks_them() {
    let k = seeded(3);
    assert_eq!(k.lifecycles().len(), 3);
    assert_eq!(k.lifecycles().count(AgentState::Active), 3);
    for i in 0..3 {
        let life = k.lifecycle_of(&keys(190 + i).did()).unwrap();
        assert_eq!(life.state(), AgentState::Active);
        assert_eq!(life.history().len(), 1, "注册即一条 Admitted 记录");
        assert!(life.state().is_participating());
    }
    let audit = k.audit();
    assert!(audit.is_clean(), "{:?}", audit.failures());
    assert!(audit
        .findings
        .iter()
        .any(|f| f.name == "lifecycle.tracked" && f.ok));
    assert!(all_passed(&k.self_check()));
}

#[test]
fn competition_degrades_and_recovers_but_never_quarantines() {
    let mut k = seeded(2);
    let did = keys(190).did();
    for i in 0..20u64 {
        let outcome = k
            .apply_lifecycle(&did, LifecycleEvent::Refused(RefusalCode::RateLimited))
            .unwrap();
        assert!(outcome.applied);
        assert_eq!(outcome.to, AgentState::Degraded);
        assert!(outcome.code.is_none());
        let _ = i;
    }
    assert_eq!(k.lifecycle_of(&did).unwrap().state(), AgentState::Degraded);
    assert_eq!(k.lifecycles().count(AgentState::Quarantined), 0, "竞争不隔离");

    // 恢复不需要任何外部批准。
    let recovered = k.apply_lifecycle(&did, LifecycleEvent::Recovered).unwrap();
    assert!(recovered.applied);
    assert_eq!(recovered.to, AgentState::Active);

    // 穷举：除了恶意 2 码，任何码都推不进隔离。
    for code in RefusalCode::ALL.iter().filter(|c| !c.is_misconduct()) {
        let mut probe = seeded(1);
        let target = keys(190).did();
        let outcome = probe
            .apply_lifecycle(&target, LifecycleEvent::Refused(*code))
            .unwrap();
        assert_ne!(outcome.to, AgentState::Quarantined, "{} 不能隔离", code.as_str());
        assert_eq!(outcome.to, AgentState::Degraded);
    }
}

#[test]
fn misconduct_evidence_quarantines_and_only_a_council_reprieve_restores() {
    let mut k = seeded(2);
    let offender = keys(190);
    // 恶意：伪造信封 → unauthorized。
    let mut forged = Envelope::new(
        offender.did(),
        None,
        "progress.event",
        0,
        None,
        serde_json::json!({}),
    )
    .unwrap()
    .seal(&offender)
    .unwrap();
    forged.body = serde_json::json!({"tampered": true});
    assert!(k.send(&forged).is_err());
    let code = k.refusals().last().map(|(_, r)| r.code).unwrap();
    assert!(code.is_misconduct());

    let outcome = k
        .apply_lifecycle(&offender.did(), LifecycleEvent::Refused(code))
        .unwrap();
    assert_eq!(outcome.to, AgentState::Quarantined);
    assert!(k.lifecycle_of(&offender.did()).unwrap().state().is_restricted());

    // 委员会通过的解除隔离决定是唯一出口。
    let motion = Motion::new(
        MotionKind::Reprieve,
        offender.did(),
        RefusalCode::Unauthorized,
        None,
        1,
    )
    .unwrap();
    let decision = Decision {
        motion: motion.id.clone(),
        kind: MotionKind::Reprieve,
        subject: offender.did(),
        verdict: Verdict::Upheld,
        at: 2,
    };
    let applied = k.apply_council_decision(&motion, &decision).unwrap().unwrap();
    assert!(applied.applied);
    assert_eq!(applied.to, AgentState::Active);

    // 未通过的决定没有执行力。
    let rejected = Decision {
        verdict: Verdict::Rejected,
        ..decision.clone()
    };
    assert!(k.apply_council_decision(&motion, &rejected).unwrap().is_none());
    // 决定与动议不匹配是调用错误。
    let mut wrong = decision.clone();
    wrong.subject = keys(191).did();
    assert!(k.apply_council_decision(&motion, &wrong).is_err());
}

#[test]
fn a_council_quarantine_motion_quarantines_the_subject() {
    let mut k = seeded(3);
    let subject = keys(190).did();
    let motion = Motion::new(
        MotionKind::Quarantine,
        subject.clone(),
        RefusalCode::Malformed,
        Some(au4a_core::Escalation::Quarantine),
        1,
    )
    .unwrap();
    let decision = Decision {
        motion: motion.id.clone(),
        kind: MotionKind::Quarantine,
        subject: subject.clone(),
        verdict: Verdict::Upheld,
        at: 2,
    };
    let outcome = k.apply_council_decision(&motion, &decision).unwrap().unwrap();
    assert_eq!(outcome.to, AgentState::Quarantined);
    assert_eq!(k.lifecycles().count(AgentState::Quarantined), 1);
    assert_eq!(k.lifecycles().count(AgentState::Active), 2);
}

#[test]
fn retired_is_terminal_at_the_kernel_level() {
    let mut k = seeded(1);
    let did = keys(190).did();
    k.apply_lifecycle(&did, LifecycleEvent::Retired).unwrap();
    assert_eq!(k.lifecycle_of(&did).unwrap().state(), AgentState::Retired);
    for event in [
        LifecycleEvent::Admitted,
        LifecycleEvent::WorkStarted,
        LifecycleEvent::Recovered,
        LifecycleEvent::CouncilQuarantine,
        LifecycleEvent::CouncilReprieve,
    ] {
        let outcome = k.apply_lifecycle(&did, event).unwrap();
        assert!(!outcome.applied, "{event:?} 不能复活退役 Agent");
        assert_eq!(outcome.code, Some(RefusalCode::StaleEpoch));
        assert_eq!(outcome.to, AgentState::Retired);
    }
    // 重复退役是幂等的。
    assert!(k.apply_lifecycle(&did, LifecycleEvent::Retired).unwrap().applied);
}

#[test]
fn unknown_agents_cannot_be_moved_and_replay_is_reproducible() {
    let mut k = seeded(1);
    let ghost = AgentKeys::from_seed(&[250u8; 32]).did();
    assert!(k.apply_lifecycle(&ghost, LifecycleEvent::Admitted).is_err());

    let run = || {
        let mut k = seeded(2);
        let did = keys(190).did();
        k.apply_lifecycle(&did, LifecycleEvent::WorkStarted).unwrap();
        k.apply_lifecycle(&did, LifecycleEvent::WorkFinished).unwrap();
        k.apply_lifecycle(&did, LifecycleEvent::Refused(RefusalCode::Timeout)).unwrap();
        k.apply_lifecycle(&did, LifecycleEvent::Recovered).unwrap();
        k.lifecycles().fingerprint().unwrap()
    };
    assert_eq!(run(), run());
    assert!(k.audit().is_clean());
}
