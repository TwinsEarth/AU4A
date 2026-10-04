//! v1.0.5 权限模型的集成测试（只用公开 API）。
//!
//! 内核必须能回答「这个 Agent 能做什么、不能做什么」，而且答案要满足：
//! 覆盖全部能力恰好一次、拒绝带类型化理由、权限来源里没有人类/运营方。

use au4a_core::{AgentKeys, Credits, RefusalCode};
use au4a_kernel::{
    authority_roots, Authority, Capability, Kernel, KernelConfig,
};

fn seeded(n: u8) -> Kernel {
    let mut k = Kernel::new(KernelConfig::default());
    for i in 0..n {
        k.register(
            &AgentKeys::from_seed(&[80 + i; 32]),
            format!("agent-{i}"),
            &["skill.a"],
            Credits(20),
        )
        .unwrap();
    }
    k
}

fn did(i: u8) -> AgentKeys {
    AgentKeys::from_seed(&[80 + i; 32])
}

#[test]
fn every_capability_gets_exactly_one_answer() {
    let k = seeded(3);
    for i in 0..3 {
        let report = k.permissions(&did(i).did()).unwrap();
        assert!(report.is_total_partition());
        assert_eq!(report.allowed.len() + report.denied.len(), Capability::ALL.len());
        let json = report.to_json();
        assert_eq!(json["total_partition"], true);
        let can = json["can"].as_array().map(|a| a.len()).unwrap_or(0);
        let cannot = json["cannot"].as_array().map(|a| a.len()).unwrap_or(0);
        assert_eq!(can + cannot, Capability::ALL.len());
        // 「能做什么」里不应出现「不能做什么」的项。
        for denial in &report.denied {
            assert!(!report.allows(denial.capability));
            assert!(!denial.reason.is_empty(), "拒绝必须说明理由");
            assert!(RefusalCode::ALL.contains(&denial.code));
        }
    }
}

#[test]
fn rights_come_from_objective_facts_not_from_an_operator() {
    let roots = authority_roots();
    assert_eq!(roots["human_is_a_root"], false);
    for authority in Authority::ALL {
        let name = authority.as_str();
        for forbidden in ["operator", "human", "admin", "owner"] {
            assert!(!name.contains(forbidden));
        }
    }
    assert_eq!(Authority::ALL.len(), 5);
    // 提案权来自自证身份：任何在册 Agent 都能提案，不需要任何人点头。
    let k = seeded(1);
    let report = k.permissions(&did(0).did()).unwrap();
    assert!(report.allows(Capability::ProposeMotion));
    assert!(report.authorities.contains(&Authority::SelfSovereign));
    assert!(report.authorities.contains(&Authority::Stake));
}

#[test]
fn an_outsider_gets_a_typed_unauthorized_answer_for_every_capability() {
    let k = seeded(2);
    let ghost = AgentKeys::from_seed(&[240u8; 32]).did();
    let report = k.permissions(&ghost).unwrap();
    assert!(!report.registered);
    assert!(report.allowed.is_empty());
    assert_eq!(report.denied.len(), Capability::ALL.len());
    assert!(report.has_misconduct_denial(), "身份不成立属于恶意分类");
    for denial in &report.denied {
        assert_eq!(denial.code, RefusalCode::Unauthorized);
        assert!(!denial.retryable());
    }
    // 与内核自己的分类一致：未知发送者投递 → unauthorized。
    assert_eq!(
        report.denial_for(Capability::DeliverDirect).unwrap().code,
        RefusalCode::Unauthorized
    );
}

#[test]
fn competitive_denials_stay_competitive() {
    let k = seeded(3);
    let report = k.permissions(&did(0).did()).unwrap();
    assert!(
        !report.has_misconduct_denial(),
        "在册 Agent 的拒绝不应出现恶意码：{:?}",
        report.denied
    );
    // 单独看每一条：即使被拒绝，也是可重试或可解释的竞争语义。
    for denial in &report.denied {
        assert!(!denial.code.is_misconduct(), "{}", denial.code.as_str());
    }
    // cpu-proto 的额度上限如实暴露（人类与 Agent 都能看到边界在哪）。
    assert!(report.limits.get("settle_cpu_proto").copied().unwrap_or(0) > 0);
}

#[test]
fn explaining_is_read_only_and_reproducible() {
    let k = seeded(2);
    let before_registry = k.registry_fingerprint().unwrap();
    let before_view = k.observe_json();
    let a = k.permissions(&did(0).did()).unwrap();
    let b = k.permissions(&did(0).did()).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.fingerprint().unwrap(), b.fingerprint().unwrap());
    assert_eq!(before_registry, k.registry_fingerprint().unwrap());
    assert_eq!(before_view, k.observe_json());
}

#[test]
fn capability_names_roundtrip_and_are_unique() {
    let mut names: Vec<&str> = Capability::ALL.iter().map(|c| c.as_str()).collect();
    names.sort();
    let count = names.len();
    names.dedup();
    assert_eq!(names.len(), count, "能力名必须唯一");
    for capability in Capability::ALL {
        assert_eq!(Capability::parse(capability.as_str()), Some(capability));
    }
    assert_eq!(Capability::parse("become_operator"), None);
}
