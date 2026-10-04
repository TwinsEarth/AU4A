//! v1.1.2 集成测试：声明 API。
//!
//! 覆盖：自己的声明入图、别人的声明进邻居视图、替他人声明被拒、
//! 篡改被判 `unauthorized`（恶意码）、陈旧/冲突版本被拒、幂等重放不推进版本。

use au4a_capgraph::{
    AgentCapabilityGraph, CapGraphConfig, Capability, Declaration, DeclareOutcome,
    SignedDeclaration, SkillId,
};
use au4a_core::{AgentKeys, CoreError, Credits, RefusalCode};
use au4a_kernel::{Kernel, KernelConfig};

fn keys(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

fn cap(name: &str, price: i64) -> Capability {
    Capability::new(SkillId::new(name).expect("valid skill"), Credits(price))
}

fn declare(keys: &AgentKeys, epoch: u64, skills: &[&str]) -> SignedDeclaration {
    let caps: Vec<Capability> = skills.iter().map(|s| cap(s, 3)).collect();
    Declaration::new(keys.did(), epoch, epoch * 100, caps)
        .expect("coherent declaration")
        .sign(keys)
        .expect("self-signed")
}

#[test]
fn an_agent_can_write_its_own_capabilities_without_any_registry() {
    let a = keys(1);
    let mut graph = AgentCapabilityGraph::new(a.did(), CapGraphConfig::default());
    let outcome = graph.apply(
        &declare(&a, 1, &["translate.en-zh", "sentiment.analyze"]),
        0,
    );
    assert!(outcome.is_applied(), "{:?}", outcome.to_value());
    assert_eq!(graph.own_epoch(), 1);
    assert_eq!(
        graph
            .own_capabilities()
            .iter()
            .map(|c| c.skill.as_str())
            .collect::<Vec<_>>(),
        vec!["sentiment.analyze", "translate.en-zh"],
        "声明内部按技能名升序，规范字节唯一"
    );
    assert_eq!(graph.to_value()["owner"].as_str(), Some(a.did().as_str()));
}

#[test]
fn declaration_epochs_are_monotonic_and_conflicts_are_named() {
    let a = keys(2);
    let b = keys(3);
    let mut graph = AgentCapabilityGraph::new(a.did(), CapGraphConfig::default());
    assert!(graph.apply(&declare(&b, 7, &["x"]), 0).is_applied());
    assert_eq!(
        graph.apply(&declare(&b, 6, &["x"]), 1).refusal(),
        Some(RefusalCode::StaleEpoch)
    );
    assert_eq!(
        graph.apply(&declare(&b, 7, &["y"]), 2).refusal(),
        Some(RefusalCode::Conflict)
    );
    assert_eq!(graph.neighbor(&b.did()).expect("record").skills().len(), 1);
    // 真升级：epoch 递增 → 接受，且旧能力被完整替换（声明是完整状态，不是增量）。
    assert!(graph.apply(&declare(&b, 8, &["y", "z"]), 3).is_applied());
    assert_eq!(
        graph
            .capabilities_of(&b.did())
            .expect("known")
            .iter()
            .map(|c| c.skill.as_str())
            .collect::<Vec<_>>(),
        vec!["y", "z"]
    );
}

#[test]
fn replaying_the_same_declaration_is_idempotent() {
    let a = keys(4);
    let b = keys(5);
    let mut graph = AgentCapabilityGraph::new(a.did(), CapGraphConfig::default());
    let signed = declare(&b, 1, &["x"]);
    assert!(graph.apply(&signed, 0).is_applied());
    for now in 1..5 {
        assert!(matches!(
            graph.apply(&signed, now),
            DeclareOutcome::Unchanged { .. }
        ));
    }
    assert_eq!(graph.neighbor(&b.did()).expect("record").epoch, 1);
    assert_eq!(
        graph.neighbor(&b.did()).expect("record").at,
        0,
        "重放不刷新接受时刻"
    );
}

#[test]
fn nobody_declares_on_behalf_of_someone_else() {
    let a = keys(6);
    let b = keys(7);
    let declaration = Declaration::new(a.did(), 1, 1, vec![cap("x", 3)]).expect("coherent");
    assert_eq!(
        declaration.clone().sign(&b),
        Err(CoreError::InvalidSignature)
    );
    // 即使对方拿到了 A 的公钥与一份合法载荷，也签不出 A 的签名。
    let signed = declaration.sign(&a).expect("A signs");
    let mut value = signed.to_value().expect("serialisable");
    value["declaration"]["agent"] = serde_json::json!(b.did().as_str());
    let mut graph = AgentCapabilityGraph::new(a.did(), CapGraphConfig::default());
    let swapped = SignedDeclaration::from_value(&value).expect("parses");
    assert_eq!(
        graph.apply(&swapped, 0).refusal(),
        Some(RefusalCode::Unauthorized)
    );
}

#[test]
fn a_tampered_declaration_is_misconduct_grade_refusal() {
    let a = keys(8);
    let b = keys(9);
    let mut graph = AgentCapabilityGraph::new(a.did(), CapGraphConfig::default());
    let signed = declare(&b, 1, &["translate.en-zh"]);
    let mut value = signed.to_value().expect("serialisable");
    value["declaration"]["capabilities"][0]["current_load_bp"] = serde_json::json!(0);
    value["declaration"]["capabilities"][0]["price_per_unit"] = serde_json::json!(0);
    let tampered = SignedDeclaration::from_value(&value).expect("parses");
    let outcome = graph.apply(&tampered, 0);
    let code = outcome.refusal().expect("refused");
    assert_eq!(code, RefusalCode::Unauthorized);
    assert!(
        code.is_misconduct(),
        "签名不成立一次即恶意（au4a-core 的拒绝分类）"
    );
    assert!(!code.retryable(), "伪造不可重试");
    assert_eq!(graph.capability_count(), 0);
}

#[test]
fn the_per_agent_skill_cap_is_enforced_on_both_paths() {
    let a = keys(10);
    let b = keys(11);
    let config = CapGraphConfig {
        max_skills_per_agent: 2,
        ..CapGraphConfig::default()
    };
    let mut graph = AgentCapabilityGraph::new(a.did(), config);
    let too_many = declare(&a, 1, &["a1", "a2", "a3"]);
    assert_eq!(
        graph.apply(&too_many, 0).refusal(),
        Some(RefusalCode::PolicyDenied)
    );
    assert_eq!(
        graph.apply(&declare(&a, 1, &["a1", "a2"]), 0).is_applied(),
        true
    );
    assert_eq!(
        graph
            .apply(&declare(&b, 1, &["b1", "b2", "b3"]), 0)
            .refusal(),
        Some(RefusalCode::PolicyDenied)
    );
    assert_eq!(graph.capability_count(), 2, "被拒的声明不留痕");
}

#[test]
fn a_declaration_from_a_neighbor_does_not_touch_own_state() {
    let a = keys(12);
    let b = keys(13);
    let mut graph = AgentCapabilityGraph::new(a.did(), CapGraphConfig::default());
    graph.apply(&declare(&a, 1, &["mine"]), 0);
    graph.apply(&declare(&b, 1, &["theirs"]), 0);
    assert_eq!(graph.own_capabilities().len(), 1);
    assert_eq!(graph.capabilities_of(&a.did()).expect("own").len(), 1);
    assert_eq!(graph.capabilities_of(&b.did()).expect("neighbor").len(), 1);
    assert_eq!(graph.known_agents(), 2);
    assert_eq!(graph.capability_count(), 2);
}

#[test]
fn scenario_is_replayable_and_carries_a_version() {
    // scenario 的形状随小版本演进（每版多做一件真事），这里只断言跨版本稳定的契约：
    // 能跑、不 panic、带版本号、同样输入给同样输出。
    let mut kernel = Kernel::new(KernelConfig::default());
    let value = au4a_capgraph::scenario(&mut kernel).expect("scenario runs");
    assert!(value["version"]
        .as_str()
        .unwrap_or_default()
        .starts_with("v1.1."));
    let mut other = Kernel::new(KernelConfig::default());
    let again = au4a_capgraph::scenario(&mut other).expect("scenario runs");
    assert_eq!(value, again, "逻辑时钟 + 固定种子 → 可重放");
}
