//! v1.0.9 测试框架的集成测试（只用公开 API）。
//!
//! 框架要能证明两件事：**同样的脚本 → 同样的世界**，以及**日志可以重放出同一个世界**。

use au4a_core::{all_passed, Credits, EvidenceGrade, RefusalCode};
use au4a_kernel::{
    invariant_suite, AutonomyPolicy, Harness, KernelConfig, LifecycleEvent,
};

fn script(base_tag: u8) -> Harness {
    let mut h = Harness::new(KernelConfig::default(), base_tag);
    h.add_agents(3, &["skill.a", "skill.b"], Credits(20)).unwrap();
    h.announce(0).unwrap();
    h.announce(1).unwrap();
    h.offer(0, 1, "skill.a", Credits(5)).unwrap();
    assert!(h.settle(0, 1, Credits(7), EvidenceGrade::Verified).unwrap());
    assert!(!h.settle(0, 1, Credits(7), EvidenceGrade::Unverified).unwrap());
    h.autonomy_turn(0, AutonomyPolicy::default()).unwrap();
    h.lifecycle(1, LifecycleEvent::WorkStarted).unwrap();
    h.lifecycle(1, LifecycleEvent::Refused(RefusalCode::RateLimited))
        .unwrap();
    h.lifecycle(1, LifecycleEvent::Recovered).unwrap();
    assert!(h.attempt_forgery(2).unwrap().is_misconduct());
    h
}

#[test]
fn the_same_script_produces_the_same_journal_and_world() {
    let a = script(21);
    let b = script(21);
    assert_eq!(a.journal_fingerprint().unwrap(), b.journal_fingerprint().unwrap());
    assert_eq!(a.outcome().unwrap(), b.outcome().unwrap());
    assert_eq!(a.journal().len(), b.journal().len());
    assert!(a.journal().iter().any(|entry| !entry.ok), "日志必须如实记录失败");
}

#[test]
fn replaying_the_journal_rebuilds_an_identical_world() {
    let h = script(22);
    let outcome = h.verify_replay().unwrap();
    let live = h.outcome().unwrap();
    assert_eq!(outcome.registry_fingerprint, live.registry_fingerprint);
    assert_eq!(outcome.lifecycle_fingerprint, live.lifecycle_fingerprint);
    assert_eq!(outcome.minted, live.minted);
    assert_eq!(outcome.refusals, live.refusals);
    assert_eq!(outcome.journal_len, h.journal().len());

    // 重放出来的世界同样满足全部不变式。
    let replayed = h.replay().unwrap();
    let checks = invariant_suite(&replayed);
    assert!(all_passed(&checks), "{:?}", checks.iter().filter(|c| !c.passed).collect::<Vec<_>>());
    assert_eq!(replayed.agent_count(), h.kernel().agent_count());
    assert_eq!(
        replayed.ledger().minted(),
        h.kernel().ledger().minted(),
        "双轨守恒：重放后发行量一致"
    );
}

#[test]
fn the_invariant_suite_covers_host_and_harness_level_properties() {
    let h = script(23);
    let checks = invariant_suite(h.kernel());
    assert!(all_passed(&checks), "{checks:?}");
    let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
    for expected in [
        "ledger.conservation",
        "stake.covered",
        "registry.consistent",
        "queue.membership",
        "refusal.classification",
        "lifecycle.tracked",
        "clock.monotonic",
        "harness.quarantine_justified",
        "harness.registry_lifecycle_bijection",
        "harness.queue_sealed",
    ] {
        assert!(names.contains(&expected), "缺少不变式 {expected}");
    }
    for check in &checks {
        assert!(!check.detail.is_empty(), "{} 必须说明断言了什么", check.name);
    }
}

#[test]
fn the_harness_drives_real_kernel_effects() {
    let h = script(24);
    let kernel = h.kernel();
    assert_eq!(kernel.agent_count(), 3);
    assert_eq!(kernel.lifecycles().len(), 3);
    assert!(kernel.queue_len() >= 3, "通告/报价信封应当入队");
    for env in kernel.queued_envelopes() {
        assert!(env.verify().is_ok());
    }
    assert!(kernel.ledger().minted() > Credits::ZERO);
    assert!(kernel.refusals().iter().any(|(_, r)| r.code.is_misconduct()));
    assert!(kernel
        .progress_events()
        .iter()
        .any(|e| e.kind == "network.genesis"));
    assert!(kernel.audit().is_clean());
    // 未知下标必须报错而不是 panic。
    let empty = Harness::new(KernelConfig::default(), 26);
    assert!(empty.did_of(99).is_err());
    assert!(empty.journal().is_empty());
}

#[test]
fn journals_are_json_serializable_and_stable() {
    let h = script(25);
    let json = h.journal_json();
    let entries = json.as_array().cloned().unwrap_or_default();
    assert_eq!(entries.len(), h.journal().len());
    for entry in &entries {
        assert!(entry["step"].is_object(), "每一步都要带上参数");
        assert!(entry["at"].is_u64());
        assert!(entry["ok"].is_boolean());
    }
    let again = script(25);
    assert_eq!(json, again.journal_json());
}
