//! v1.0.1 宿主内核重构的集成测试（只用公开 API）。
//!
//! 这一版要证明的三件事：
//! 1. 宿主内核的公开闭环（注册 → 投递 → 结算 → 审计 → 观察）可以被脚本化重放，两次跑出同一结果；
//! 2. 重构没有破坏冻结接口：`register/send/drain/settle/observe` 的语义逐条断言；
//! 3. 拒绝分类在宿主层仍然成立：竞争码重复只升级到 Warn，恶意码单次即 Quarantine。

use au4a_core::{all_passed, AgentKeys, Credits, Envelope, EvidenceGrade, RefusalCode};
use au4a_kernel::{bootstrap, results_json, scenario, self_check, Kernel, KernelConfig, TRACK};

fn agent(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

fn two_agents() -> (Kernel, AgentKeys, AgentKeys) {
    let mut k = Kernel::new(KernelConfig::default());
    let a = agent(11);
    let b = agent(12);
    k.register(&a, "a", &["x"], Credits(20)).unwrap();
    k.register(&b, "b", &["y"], Credits(20)).unwrap();
    (k, a, b)
}

#[test]
fn scenario_drives_the_shared_kernel_and_is_reproducible() {
    let first = scenario(&mut Kernel::new(KernelConfig::default())).unwrap();
    let second = scenario(&mut Kernel::new(KernelConfig::default())).unwrap();
    assert_eq!(first, second, "同样的种子必须给出同样的摘要");
    assert_eq!(first["track"], TRACK);
    assert_eq!(first["settled_verified"], 1);
    assert_eq!(first["settled_cpu_proto"], 1);
    assert_eq!(first["refused_unverified"], 1);
    assert_eq!(first["audit"]["clean"], true);
    assert!(first["registry_fingerprint"].as_str().unwrap_or("").len() >= 16);
}

#[test]
fn scenario_is_safe_to_run_twice_on_one_kernel() {
    let mut k = Kernel::new(KernelConfig::default());
    let a = scenario(&mut k).unwrap();
    let b = scenario(&mut k).unwrap();
    // 第二次不会重复注册（复用名片），但会继续投递与结算：摘要反映的是内核真实状态。
    assert_eq!(a["agents"], b["agents"]);
    assert_eq!(b["audit"]["clean"], true);
    assert!(all_passed(&k.self_check()));
}

#[test]
fn bootstrap_is_deterministic_and_audit_clean() {
    let a = bootstrap(KernelConfig::default()).unwrap();
    let b = bootstrap(KernelConfig::default()).unwrap();
    assert_eq!(a.agent_count(), 3);
    assert_eq!(
        a.registry_fingerprint().unwrap(),
        b.registry_fingerprint().unwrap()
    );
    assert!(a.audit().is_clean(), "{:?}", a.audit().failures());
    assert!(a
        .agents()
        .any(|c| c.skills.iter().any(|s| s == "kernel.host")));
    assert_eq!(a.agents_with_skill("kernel.settle").len(), 1);
    assert!(a.agents_with_skill("nobody.declares.this").is_empty());
}

#[test]
fn frozen_host_loop_semantics_are_unchanged() {
    let (mut k, a, b) = two_agents();
    assert!(k.is_registered(&a.did()));
    assert_eq!(
        k.card(&a.did()).map(|c| c.display.clone()),
        Some("a".to_string())
    );

    let env = Envelope::new(
        a.did(),
        Some(b.did()),
        "progress.event",
        k.now() + 1,
        None,
        serde_json::json!({"p": 1}),
    )
    .unwrap()
    .seal(&a)
    .unwrap();
    let report = k.send(&env).unwrap();
    assert!(report.accepted);
    assert_eq!(report.queue_len, 1);
    assert_eq!(k.queued_envelopes().len(), 1);
    assert_eq!(k.drain().len(), 1);
    assert_eq!(k.drain().len(), 0);

    k.settle(&a.did(), &b.did(), Credits(10), EvidenceGrade::Verified)
        .unwrap();
    k.ledger().check_conservation().unwrap();
    assert!(k.audit().is_clean());
}

#[test]
fn observing_does_not_change_the_kernel() {
    let (k, _, _) = two_agents();
    let before = k.registry_fingerprint().unwrap();
    let v1 = k.observe(); // &Kernel：可以同时持有多个只读借用
    let v2 = k.observe();
    assert_eq!(v1, v2);
    assert_eq!(before, k.registry_fingerprint().unwrap());
    assert_eq!(v1.agents.len(), 2);
    assert_eq!(v1.network_id, "au4a-local");
    assert!(!v1.progress.is_empty());
}

#[test]
fn one_unauthorized_envelope_quarantines_but_repeated_races_only_warn() {
    let (mut k, a, _b) = two_agents();
    // 恶意：签名与内容不符（伪造），单次即恶意证据。
    let mut forged = Envelope::new(
        a.did(),
        None,
        "progress.event",
        1,
        None,
        serde_json::json!({}),
    )
    .unwrap()
    .seal(&a)
    .unwrap();
    forged.body = serde_json::json!({"tampered": true});
    assert!(k.send(&forged).is_err());
    assert_eq!(
        k.escalation_for(&a.did(), RefusalCode::Unauthorized),
        au4a_core::Escalation::Quarantine
    );

    // 竞争：证据闸门拒绝（policy_denied），单次只记警告，重复到阈值升级为 Warn，绝不隔离。
    let c = agent(13);
    k.register(&c, "c", &["z"], Credits(20)).unwrap();
    for _ in 0..au4a_core::refusal::REPEAT_THRESHOLD {
        assert!(k
            .settle(&c.did(), &a.did(), Credits(1), EvidenceGrade::Unverified)
            .is_err());
    }
    assert_eq!(
        k.escalation_for(&c.did(), RefusalCode::PolicyDenied),
        au4a_core::Escalation::Warn
    );
    assert_ne!(
        k.escalation_for(&c.did(), RefusalCode::PolicyDenied),
        au4a_core::Escalation::Quarantine,
        "因为竞争而隔离，等于因为竞争而误伤"
    );
    assert!(
        k.escalation_for(&c.did(), RefusalCode::RateLimited) < au4a_core::Escalation::Warn,
        "没发生过的码不应升级"
    );
    assert!(k.audit().is_clean(), "拒绝记录本身不能让内核不自洽");
}

#[test]
fn track_level_self_check_and_results_are_real() {
    let checks = self_check();
    assert!(all_passed(&checks), "{checks:?}");
    assert!(checks.iter().any(|c| c.name == "host.audit"));
    assert!(checks.iter().any(|c| c.name == "registry.deterministic"));
    let results = results_json().unwrap();
    assert_eq!(results["track"], "1.0");
    assert_eq!(results["all_passed"], true);
    assert!(results["checks_total"].as_u64().unwrap_or(0) >= 3);
}

#[test]
fn audit_reports_a_clean_host_as_a_stable_json_value() {
    let (k, _, _) = two_agents();
    let a = k.audit().to_json();
    let b = k.audit().to_json();
    assert_eq!(a, b);
    assert_eq!(a["clean"], true);
    assert!(a["checks"].as_u64().unwrap_or(0) >= 7);
    assert_eq!(a["failed"], 0);
}

#[test]
fn kernel_config_is_honoured() {
    let mut config = KernelConfig::default();
    config.min_stake = Credits(50);
    config.genesis_mint = Credits(500);
    let mut k = Kernel::new(config);
    let a = agent(21);
    assert!(k.register(&a, "poor", &[], Credits(49)).is_err());
    assert_eq!(k.refusals().len(), 1);
    assert_eq!(k.refusals()[0].1.code, RefusalCode::PolicyDenied);
    assert!(k.register(&a, "rich", &[], Credits(50)).is_ok());
    assert_eq!(k.ledger().balance(&a.did()).locked, Credits(50));
    assert!(k.audit().is_clean());
}
