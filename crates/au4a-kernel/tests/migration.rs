//! v1.0.8 迁移适配器的集成测试（只用公开 API）。
//!
//! 底线：旧插件的「运营方审批」必须被**具名拒绝**（而不是静默丢掉），
//! 不可映射的宿主权限必须如实记录；执行计划时 fail-closed（被拒能力不发消息）。

use au4a_core::{Credits, RefusalCode};
use au4a_kernel::{
    adapt, apply_plan, hook_route, permission_capability, Kernel, KernelConfig, MigrationLimits,
    MigrationRefusal, V3PluginManifest,
};

fn manifest(name: &str, hooks: &[&str], permissions: &[&str]) -> V3PluginManifest {
    V3PluginManifest {
        name: name.to_string(),
        entry: format!("plugins/{name}.wasm"),
        hooks: hooks.iter().map(|s| (*s).to_string()).collect(),
        permissions: permissions.iter().map(|s| (*s).to_string()).collect(),
        requires_operator_approval: false,
        owner: None,
    }
}

fn keys(tag: u8) -> au4a_core::AgentKeys {
    au4a_core::AgentKeys::from_seed(&[tag; 32])
}

#[test]
fn every_mapped_hook_gets_a_real_pmb_route() {
    let hooks = [
        "on_start",
        "on_announce",
        "on_tick",
        "on_progress",
        "on_offer",
        "on_decline",
        "on_settle",
        "on_council_motion",
        "on_council_vote",
    ];
    for hook in hooks {
        let route = hook_route(hook).unwrap_or_else(|| panic!("{hook} 应当有路由"));
        assert!(!route.message_kind.is_empty());
        assert!(au4a_kernel::classify_kind(&route.message_kind) != au4a_kernel::MessageClass::Unknown);
    }
    assert!(hook_route("on_teleport").is_none());
    // 权限映射：能力表里的每一项都能往返。
    for permission in [
        "write:progress",
        "message:direct",
        "settle:verified",
        "settle:cpu-proto",
        "negotiate:offer",
        "governance:vote",
        "governance:propose",
        "skills:declare",
        "stake:withdraw",
    ] {
        assert!(permission_capability(permission).is_some(), "{permission}");
    }
    assert!(permission_capability("net:http").is_none());
}

#[test]
fn the_reference_projects_operator_approval_is_refused_with_a_reason() {
    let mut legacy = manifest("legacy-notes", &["on_start"], &["write:progress"]);
    legacy.requires_operator_approval = true;
    let err = adapt(&legacy, &MigrationLimits::default()).unwrap_err();
    assert_eq!(err, MigrationRefusal::OperatorApprovalRequired);
    assert_eq!(err.to_refusal_code(), RefusalCode::PolicyDenied);
    assert!(!err.to_refusal_code().is_misconduct(), "策略问题不是恶意");
    assert!(err.detail().contains("运营方"));

    // owner-scoped 授权同样被拒。
    let scoped = manifest("scoped", &["on_start"], &["owner:transfer"]);
    assert!(scoped.has_operator_dependency());
    assert_eq!(
        adapt(&scoped, &MigrationLimits::default()).unwrap_err(),
        MigrationRefusal::OperatorApprovalRequired
    );
}

#[test]
fn host_level_permissions_are_recorded_not_silently_granted() {
    let plan = adapt(
        &manifest(
            "netty",
            &["on_progress", "on_settle"],
            &["write:progress", "net:http", "fs:read", "exec:shell"],
        ),
        &MigrationLimits::default(),
    )
    .unwrap();
    assert!(plan.is_partial());
    assert_eq!(plan.refused_permissions.len(), 3);
    assert!(plan.is_intact());
    assert_eq!(plan.evidence.as_str(), "cpu-proto", "适配器是纯 CPU 语义");
    let json = plan.to_json();
    assert_eq!(json["refused"].as_array().map(|a| a.len()), Some(3));
    for refusal in json["refused"].as_array().unwrap_or(&vec![]) {
        assert_eq!(refusal["code"], "unsupported");
    }
}

#[test]
fn applying_a_plan_produces_signed_envelopes_and_respects_the_permission_model() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let host = keys(220);
    let peer = keys(221);
    kernel.register(&host, "host", &["x"], Credits(20)).unwrap();
    kernel.register(&peer, "peer", &["y"], Credits(20)).unwrap();

    let plan = adapt(
        &manifest(
            "notes",
            &["on_start", "on_offer", "on_settle"],
            &["write:progress", "settle:verified"],
        ),
        &MigrationLimits::default(),
    )
    .unwrap();
    let application = apply_plan(&mut kernel, &host, &plan).unwrap();
    assert_eq!(application.sent, 3);
    assert!(application.denied.is_empty());
    assert_eq!(application.plugin, "notes");
    for env in kernel.queued_envelopes() {
        assert!(env.verify().is_ok());
        assert_eq!(env.from, host.did());
    }
    assert!(kernel.audit().is_clean());

    // 篡改过的计划不执行（fail-closed）。
    let mut tampered = plan.clone();
    tampered.plugin = "evil".to_string();
    assert!(apply_plan(&mut kernel, &host, &tampered).is_err());
}

#[test]
fn denied_capabilities_block_their_routes() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let tight = keys(222);
    // 质押 == 准入下限 → withdraw_stake 在权限模型里被拒。
    kernel.register(&tight, "tight", &["x"], Credits(10)).unwrap();
    let plan = adapt(
        &manifest("tight", &["on_start"], &["stake:withdraw"]),
        &MigrationLimits::default(),
    )
    .unwrap();
    let application = apply_plan(&mut kernel, &tight, &plan).unwrap();
    assert!(application
        .denied
        .iter()
        .any(|(cap, code)| cap == "withdraw_stake" && *code == RefusalCode::PolicyDenied));
    assert_eq!(application.sent, 1, "on_start 的能力是被允许的");
    assert_eq!(kernel.queued_envelopes().len(), 1);
}

#[test]
fn adaptation_is_reproducible_and_content_addressed() {
    let m = manifest(
        "notes",
        &["on_start", "on_tick", "on_settle"],
        &["write:progress", "settle:verified"],
    );
    let a = adapt(&m, &MigrationLimits::default()).unwrap();
    let b = adapt(&m, &MigrationLimits::default()).unwrap();
    assert_eq!(a.id, b.id);
    assert_eq!(a.fingerprint().unwrap(), b.fingerprint().unwrap());
    assert!(a.is_intact());
    // 三个钩子 + 两条权限去重后只有两种内核能力。
    assert_eq!(a.capabilities().len(), 2);
    assert!(a.capabilities().contains(&au4a_kernel::Capability::PublishCard));
    assert!(a.capabilities().contains(&au4a_kernel::Capability::SettleVerified));
}
