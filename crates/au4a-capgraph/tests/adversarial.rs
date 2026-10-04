//! v1.1.9 对抗用例：把「别人会怎么搞坏这张图」逐个写成会失败的尝试。
//!
//! 每一例都断言三件事：**被拒**、**拒绝码正确**、**图没有因此改变**。
//! 只有第三件是真正难的——很多系统会「拒绝但已经写了一半」。

use au4a_capgraph::{
    announce, announce_to, ingest, query_skill, AgentCapabilityGraph, CapGraphConfig, Capability,
    CapabilityQuery, Declaration, FormatId, Ingest, PipelineRequest, PipelineStep, PlanCost,
    SignedDeclaration, SkillId, PROTOCOL,
};
use au4a_core::{kinds, AgentKeys, Credits, Envelope, RefusalCode};
use au4a_kernel::{Kernel, KernelConfig};

fn skill(name: &str) -> SkillId {
    SkillId::new(name).expect("valid skill")
}

fn format(name: &str) -> FormatId {
    FormatId::new(name).expect("valid format")
}

fn keys(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

fn capability(price: i64, reliability_bp: u16) -> Capability {
    Capability::new(skill("translate.en-zh"), Credits(price))
        .with_reliability_bp(reliability_bp)
}

fn declaration(keys: &AgentKeys, epoch: u64, price: i64) -> Declaration {
    Declaration::new(keys.did(), epoch, epoch, vec![capability(price, 9_000)]).expect("coherent")
}

/// 断言：拒绝 + 码正确 + 图未变（用一个「指纹」把图状态固定下来）。
fn expect_refused(
    graph: &mut AgentCapabilityGraph,
    signed: &SignedDeclaration,
    now: u64,
    code: RefusalCode,
) {
    let before = graph.to_value().to_string();
    let outcome = graph.apply(signed, now);
    assert_eq!(outcome.refusal(), Some(code), "{:?}", outcome.to_value());
    assert_eq!(graph.to_value().to_string(), before, "被拒的声明不得改变图");
}

#[test]
fn a_forged_price_is_refused_and_changes_nothing() {
    let alice = keys(1);
    let bob = keys(2);
    let mut graph = AgentCapabilityGraph::new(alice.did(), CapGraphConfig::default());
    let honest = declaration(&bob, 1, 5);
    assert!(graph
        .apply(&honest.clone().sign(&bob).expect("signed"), 0)
        .is_applied());
    let before = graph.to_value().to_string();

    // 攻击者把价格改低，但签名覆盖的是原字节。
    let signed = honest.sign(&bob).expect("signed");
    let mut value = signed.to_value().expect("serialisable");
    value["declaration"]["capabilities"][0]["price_per_unit"] = serde_json::json!(0);
    let tampered = SignedDeclaration::from_value(&value).expect("parses");
    assert_eq!(graph.apply(&tampered, 1).refusal(), Some(RefusalCode::Unauthorized));
    assert_eq!(graph.to_value().to_string(), before);
}

#[test]
fn a_replayed_old_epoch_cannot_roll_back_a_price_change() {
    let alice = keys(3);
    let bob = keys(4);
    let mut graph = AgentCapabilityGraph::new(alice.did(), CapGraphConfig::default());
    let old = declaration(&bob, 1, 1).sign(&bob).expect("signed");
    let new = declaration(&bob, 2, 9).sign(&bob).expect("signed");
    assert!(graph.apply(&old, 0).is_applied());
    assert!(graph.apply(&new, 1).is_applied());
    expect_refused(&mut graph, &old, 2, RefusalCode::StaleEpoch);
    // 重放之后图里仍是 v2 的价格。
    assert_eq!(
        graph.capabilities_of(&bob.did()).expect("known")[0].price_per_unit,
        Credits(9)
    );
}

#[test]
fn a_same_epoch_different_content_is_a_conflict() {
    let alice = keys(5);
    let bob = keys(6);
    let mut graph = AgentCapabilityGraph::new(alice.did(), CapGraphConfig::default());
    assert!(graph
        .apply(&declaration(&bob, 4, 3).sign(&bob).expect("signed"), 0)
        .is_applied());
    expect_refused(
        &mut graph,
        &declaration(&bob, 4, 8).sign(&bob).expect("signed"),
        1,
        RefusalCode::Conflict,
    );
}

#[test]
fn an_envelope_for_someone_else_cannot_smuggle_a_declaration() {
    let alice = keys(7);
    let victim = keys(8);
    let attacker = keys(9);
    let mut graph = AgentCapabilityGraph::new(alice.did(), CapGraphConfig::default());
    let signed = declaration(&victim, 1, 5).sign(&victim).expect("signed");
    let body = serde_json::json!({"protocol": PROTOCOL, "declaration": signed.to_value().expect("v")});
    let smuggled = Envelope::new(attacker.did(), None, "capgraph.announce", 1, None, body)
        .expect("envelope")
        .seal(&attacker)
        .expect("sealed");
    let outcome: Ingest = ingest(&mut graph, &smuggled, 0);
    assert_eq!(outcome.refusal(), Some(RefusalCode::Unauthorized));
    assert_eq!(graph.capability_count(), 0);
    assert_eq!(graph.last_version_of(&victim.did()), None);
}

#[test]
fn a_flood_of_oversized_declarations_is_policy_denied() {
    let alice = keys(10);
    let bob = keys(11);
    let mut graph = AgentCapabilityGraph::new(
        alice.did(),
        CapGraphConfig {
            max_skills_per_agent: 2,
            ..CapGraphConfig::default()
        },
    );
    let caps: Vec<Capability> = (0..3)
        .map(|index| Capability::new(skill(&format!("s{index}")), Credits(1)))
        .collect();
    let bomb = Declaration::new(bob.did(), 1, 1, caps).expect("coherent");
    expect_refused(
        &mut graph,
        &bomb.sign(&bob).expect("signed"),
        0,
        RefusalCode::PolicyDenied,
    );
}

#[test]
fn a_starved_budget_yields_a_typed_no_path_not_a_bad_plan() {
    let alice = keys(12);
    let bob = keys(13);
    let carol = keys(14);
    let mut graph = AgentCapabilityGraph::new(alice.did(), CapGraphConfig::default());
    for (peer, name, price, inputs, outputs) in [
        (&bob, "translate.en-zh", 50i64, ["text/plain"], ["application/json"]),
        (&carol, "sentiment.analyze", 50, ["application/json"], ["application/json"]),
    ] {
        let capability = Capability::new(skill(name), Credits(price))
            .with_formats(&inputs, &outputs)
            .expect("valid formats");
        let signed = Declaration::new(peer.did(), 1, 0, vec![capability])
            .expect("coherent")
            .sign(peer)
            .expect("signed");
        graph.apply(&signed, 0);
    }
    let request = PipelineRequest::new(
        format("text/plain"),
        vec![
            PipelineStep::new(skill("translate.en-zh")),
            PipelineStep::new(skill("sentiment.analyze")),
        ],
    )
    .with_budget(Credits(99));
    let outcome = graph.plan(&request, &PlanCost::default(), 0);
    assert!(!outcome.is_path(), "预算 99 < 100，不该给出方案");
    assert_eq!(outcome.refusal_code(), Some(RefusalCode::PolicyDenied));
    assert!(outcome.to_value()["no_path"]["detail"].is_string());
}

#[test]
fn a_zero_capacity_neighbor_cache_refuses_without_misconduct() {
    let alice = keys(15);
    let bob = keys(16);
    let mut graph = AgentCapabilityGraph::new(
        alice.did(),
        CapGraphConfig {
            neighbor_capacity: 0,
            ..CapGraphConfig::default()
        },
    );
    let outcome = graph.apply(&declaration(&bob, 1, 1).sign(&bob).expect("signed"), 0);
    assert_eq!(outcome.refusal(), Some(RefusalCode::ResourceExhausted));
    assert!(
        !outcome.refusal().expect("code").is_misconduct(),
        "容量是配置问题，不能算成对端作恶"
    );
    assert!(
        outcome.refusal().expect("code").retryable(),
        "容量属于竞争语义：可重试（等容量释放或改配置），而不是判死"
    );
}

#[test]
fn a_hostile_query_cannot_widen_its_own_scope() {
    // 极端 limit 与不可能的过滤条件：结果要么为空，要么被 limit 截断，
    // 且扫描量仍然等于索引候选数（不能因为查询"看起来很贵"就退化成全图扫描）。
    let alice = keys(17);
    let bob = keys(18);
    let mut graph = AgentCapabilityGraph::new(alice.did(), CapGraphConfig::default());
    let signed = declaration(&bob, 1, 3).sign(&bob).expect("signed");
    graph.apply(&signed, 0);

    let huge = graph.query(&CapabilityQuery::new(skill("translate.en-zh")).with_limit(usize::MAX), 0);
    assert_eq!(huge.matches.len(), 1);
    assert_eq!(huge.stats.scanned, 1);

    let impossible = graph.query(
        &CapabilityQuery::new(skill("translate.en-zh")).with_max_price(Credits(0)),
        0,
    );
    assert!(impossible.is_empty());
    assert_eq!(impossible.stats.scanned, 1, "过滤在候选集内完成，没有扩大扫描范围");
    assert_eq!(impossible.refusal_code(), Some(RefusalCode::Unsupported));
}

#[test]
fn a_wrong_kind_or_wrong_protocol_is_refused_at_the_channel() {
    let alice = keys(19);
    let bob = keys(20);
    let mut graph = AgentCapabilityGraph::new(alice.did(), CapGraphConfig::default());
    let foreign = Envelope::new(bob.did(), None, kinds::AGENT_CARD, 1, None, serde_json::json!({}))
        .expect("envelope")
        .seal(&bob)
        .expect("sealed");
    assert_eq!(
        ingest(&mut graph, &foreign, 0).refusal(),
        Some(RefusalCode::Unsupported)
    );
    let stale_protocol = Envelope::new(
        bob.did(),
        None,
        "capgraph.announce",
        1,
        None,
        serde_json::json!({"protocol": "au4a.capgraph/0"}),
    )
    .expect("envelope")
    .seal(&bob)
    .expect("sealed");
    assert_eq!(
        ingest(&mut graph, &stale_protocol, 0).refusal(),
        Some(RefusalCode::Unsupported)
    );

    // 合法的询问不能被当成通告处理。
    let query = query_skill(&bob, &skill("translate.en-zh"), 1, None).expect("query");
    assert_eq!(
        ingest(&mut graph, &query, 0).refusal(),
        Some(RefusalCode::Unsupported)
    );
    assert_eq!(graph.capability_count(), 0);
}

#[test]
fn a_malicious_sender_gets_quarantined_while_victims_stay_clean() {
    let alice = keys(21);
    let bob = keys(22);
    let victim = keys(23);
    let mut kernel = Kernel::new(KernelConfig::default());
    for agent in [&alice, &bob, &victim] {
        kernel
            .register(agent, "agent", &[], Credits(20))
            .expect("register");
    }
    // bob 用自己的信封携带受害者的声明（身份错配）。
    let signed = declaration(&victim, 1, 5).sign(&victim).expect("signed");
    let body = serde_json::json!({"protocol": PROTOCOL, "declaration": signed.to_value().expect("v")});
    let smuggled = Envelope::new(bob.did(), None, "capgraph.announce", 1, None, body)
        .expect("envelope")
        .seal(&bob)
        .expect("sealed");
    kernel.send(&smuggled).expect("信封本身合法");
    let mut graph = AgentCapabilityGraph::new(alice.did(), CapGraphConfig::default());
    let report = au4a_capgraph::pump(&mut kernel, &mut graph, 1);
    assert_eq!(report.refused, 1);
    assert_eq!(report.applied, 0);
    let (who, refusal) = kernel.refusals().last().expect("refusal");
    assert_eq!(who, &bob.did(), "拒绝记在伪造者头上");
    assert_eq!(refusal.code, RefusalCode::Unauthorized);
    assert!(!kernel
        .refusals()
        .iter()
        .any(|(did, _)| did == &victim.did()), "受害者不应被连带记录");
    assert_eq!(graph.capability_count(), 0);
}

#[test]
fn a_direct_unicast_to_an_unknown_recipient_is_not_delivered() {
    let alice = keys(24);
    let bob = keys(25);
    let ghost = keys(26);
    let mut kernel = Kernel::new(KernelConfig::default());
    kernel.register(&alice, "a", &[], Credits(20)).expect("register");
    kernel.register(&bob, "b", &[], Credits(20)).expect("register");
    let env = announce_to(&bob, &declaration(&bob, 1, 1), ghost.did(), 1, None).expect("announce");
    assert!(kernel.send(&env).is_err(), "未知收件人必须被内核拒绝");
    assert_eq!(kernel.queue_len(), 0);
    let env = announce(&bob, &declaration(&bob, 1, 1), 1).expect("announce");
    assert!(kernel.send(&env).is_ok(), "广播仍然可用");
}
