//! v1.1.3 集成测试：广播协议。
//!
//! 覆盖：真签名验签（含篡改）、信封身份与声明身份必须一致、类型/协议版本拒绝、
//! 广播（`to: None`）与单播、分帧往返、`pump` 的「只吃自己的、转发别人的」。

use au4a_capgraph::{
    announce, announce_to, demo_agents, ingest, parse_announcement, parse_query, pump, query_skill,
    AgentCapabilityGraph, CapGraphConfig, Capability, Declaration, Ingest, KIND_ANNOUNCE,
    KIND_REQUEST, PROTOCOL,
};
use au4a_core::{kinds, AgentKeys, Credits, Envelope, RefusalCode};
use au4a_kernel::{Kernel, KernelConfig};

fn keys(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

fn declaration(k: &AgentKeys, epoch: u64) -> Declaration {
    Declaration::new(
        k.did(),
        epoch,
        epoch * 10,
        vec![Capability::new(
            au4a_capgraph::SkillId::new("translate.en-zh").expect("valid"),
            Credits(4),
        )],
    )
    .expect("coherent")
}

#[test]
fn a_sealed_announcement_survives_the_wire_and_the_frame_codec() {
    let a = keys(1);
    let env = announce(&a, &declaration(&a, 1), 7).expect("announce");
    env.verify().expect("sealed by the sender");
    assert_eq!(env.to, None, "广播：to = None");
    assert_eq!(env.kind.as_str(), KIND_ANNOUNCE);
    assert_eq!(env.body["protocol"].as_str(), Some(PROTOCOL));

    let frame = au4a_core::encode_frame(&env).expect("frames");
    assert_eq!(
        &frame[..4],
        &(frame.len() as u32 - 4).to_be_bytes(),
        "4 字节大端长度前缀"
    );
    let decoded = au4a_core::decode_frame(&frame).expect("decodes");
    assert_eq!(decoded, env);
    assert_eq!(decoded.id, env.id, "内容寻址 id 不因传输改变");
}

#[test]
fn a_tampered_announcement_is_refused_as_misconduct_evidence() {
    for field in [
        "price_per_unit",
        "throughput_per_min",
        "reliability_bp",
        "current_load_bp",
    ] {
        let a = keys(2);
        let mut env = announce(&a, &declaration(&a, 1), 3).expect("announce");
        env.body["declaration"]["declaration"]["capabilities"][0][field] = serde_json::json!(1);
        let (code, reason) = parse_announcement(&env).expect_err("tampered");
        assert_eq!(code, RefusalCode::Unauthorized, "{field}");
        assert!(code.is_misconduct(), "签名不成立一次即恶意：{reason}");
    }
}

#[test]
fn the_envelope_sender_must_be_the_declaration_author() {
    let a = keys(3);
    let b = keys(4);
    let signed = declaration(&a, 1).sign(&a).expect("A signs");
    let body =
        serde_json::json!({"protocol": PROTOCOL, "declaration": signed.to_value().expect("v")});
    let smuggled = Envelope::new(b.did(), None, KIND_ANNOUNCE, 1, None, body)
        .expect("envelope")
        .seal(&b)
        .expect("sealed by B");
    smuggled.verify().expect("信封确实是 B 签的");
    let (code, reason) = parse_announcement(&smuggled).expect_err("identity mismatch");
    assert_eq!(code, RefusalCode::Unauthorized);
    assert!(reason.contains("carries a declaration by"), "{reason}");

    let mut graph = AgentCapabilityGraph::new(keys(5).did(), CapGraphConfig::default());
    assert!(!ingest(&mut graph, &smuggled, 0).is_routed());
    assert_eq!(graph.capability_count(), 0);
}

#[test]
fn wrong_kind_or_wrong_protocol_never_reaches_the_graph() {
    let a = keys(6);
    let b = keys(7);
    let mut graph = AgentCapabilityGraph::new(a.did(), CapGraphConfig::default());

    let foreign_kind = Envelope::new(
        b.did(),
        None,
        kinds::PROGRESS_EVENT,
        1,
        None,
        serde_json::json!({}),
    )
    .expect("envelope")
    .seal(&b)
    .expect("sealed");
    assert_eq!(
        ingest(&mut graph, &foreign_kind, 0).refusal(),
        Some(RefusalCode::Unsupported)
    );

    let old_protocol = Envelope::new(
        b.did(),
        None,
        KIND_ANNOUNCE,
        1,
        None,
        serde_json::json!({"protocol": "au4a.capgraph/0"}),
    )
    .expect("envelope")
    .seal(&b)
    .expect("sealed");
    assert_eq!(
        ingest(&mut graph, &old_protocol, 0).refusal(),
        Some(RefusalCode::Unsupported)
    );
    assert_eq!(graph.capability_count(), 0);
}

#[test]
fn a_full_broadcast_round_trip_lands_in_the_neighbor_view() {
    let agents = demo_agents().expect("demo agents");
    let mut kernel = Kernel::new(KernelConfig::default());
    for agent in &agents {
        kernel
            .register(&agent.keys, agent.display, &[], Credits(20))
            .expect("register");
    }
    let alice = &agents[0];
    let mut graph = AgentCapabilityGraph::new(alice.keys.did(), CapGraphConfig::default());
    graph.apply(
        &Declaration::new(alice.keys.did(), 1, 0, alice.capabilities.clone())
            .expect("coherent")
            .sign(&alice.keys)
            .expect("signed"),
        0,
    );

    for agent in &agents[1..] {
        let env = announce(
            &agent.keys,
            &Declaration::new(agent.keys.did(), 1, 0, agent.capabilities.clone())
                .expect("coherent"),
            1,
        )
        .expect("announce");
        kernel
            .send(&env)
            .expect("kernel accepts the sealed broadcast");
    }
    let report = pump(&mut kernel, &mut graph, 1);
    assert_eq!(report.routed, 4);
    assert_eq!(report.applied, 4);
    assert_eq!(report.refused, 0);
    assert_eq!(report.forwarded, 0);
    assert_eq!(graph.known_agents(), 5);
    assert_eq!(graph.capability_count(), 5);
    assert!(kernel.refusals().is_empty(), "合法广播不应产生拒绝记录");
}

#[test]
fn pump_leaves_other_tracks_messages_in_the_queue() {
    let a = keys(8);
    let b = keys(9);
    let mut kernel = Kernel::new(KernelConfig::default());
    kernel
        .register(&a, "a", &[], Credits(20))
        .expect("register a");
    kernel
        .register(&b, "b", &[], Credits(20))
        .expect("register b");

    let mine = announce(&b, &declaration(&b, 1), 1).expect("announce");
    let theirs = Envelope::new(
        b.did(),
        None,
        kinds::SAFETY_REPORT,
        1,
        None,
        serde_json::json!({"x": 1}),
    )
    .expect("envelope")
    .seal(&b)
    .expect("sealed");
    kernel.send(&mine).expect("accepted");
    kernel.send(&theirs).expect("accepted");

    let mut graph = AgentCapabilityGraph::new(a.did(), CapGraphConfig::default());
    let report = pump(&mut kernel, &mut graph, 1);
    assert_eq!(report.routed, 1);
    assert_eq!(report.forwarded, 1);
    let left = kernel.drain();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].kind.as_str(), kinds::SAFETY_REPORT);
    assert_eq!(left[0].id, theirs.id);
}

#[test]
fn a_malicious_announcement_is_recorded_in_the_kernel_refusals() {
    let a = keys(10);
    let b = keys(11);
    let mut kernel = Kernel::new(KernelConfig::default());
    kernel
        .register(&a, "a", &[], Credits(20))
        .expect("register a");
    kernel
        .register(&b, "b", &[], Credits(20))
        .expect("register b");

    // 伪造者用自己的私钥签一个「不是我声明的声明」：信封层就被拒。
    let signed = declaration(&a, 1).sign(&a).expect("A signs");
    let body =
        serde_json::json!({"protocol": PROTOCOL, "declaration": signed.to_value().expect("v")});
    let forged = Envelope::new(b.did(), None, KIND_ANNOUNCE, 1, None, body)
        .expect("envelope")
        .seal(&b)
        .expect("sealed by b");
    kernel.send(&forged).expect("信封本身是合法的");

    let mut graph = AgentCapabilityGraph::new(a.did(), CapGraphConfig::default());
    let report = pump(&mut kernel, &mut graph, 1);
    assert_eq!(report.refused, 1);
    assert_eq!(report.applied, 0);
    let (who, refusal) = &kernel.refusals()[0];
    assert_eq!(who, &b.did(), "拒绝记在伪造者头上，不是被冒名者");
    assert_eq!(refusal.code, RefusalCode::Unauthorized);
    assert!(refusal.code.is_misconduct());
    assert_eq!(
        kernel.escalation_for(&b.did(), RefusalCode::Unauthorized),
        au4a_core::Escalation::Quarantine
    );
    assert_eq!(graph.capability_count(), 0);
}

#[test]
fn unicast_announcements_and_skill_queries_round_trip() {
    let a = keys(12);
    let b = keys(13);
    let env = announce_to(&a, &declaration(&a, 2), b.did(), 5, None).expect("unicast");
    env.verify().expect("sealed");
    assert_eq!(env.to, Some(b.did()));
    let parsed = parse_announcement(&env).expect("parses");
    assert_eq!(parsed.declaration.epoch, 2);

    let skill = au4a_capgraph::SkillId::new("translate.en-zh").expect("valid");
    let query = query_skill(&a, &skill, 6, None).expect("query");
    query.verify().expect("sealed");
    assert_eq!(query.kind.as_str(), KIND_REQUEST);
    let (from, parsed_skill) = parse_query(&query).expect("parses");
    assert_eq!(from, a.did());
    assert_eq!(parsed_skill, skill);
    assert_eq!(
        parse_query(&env).expect_err("kind mismatch").0,
        RefusalCode::Unsupported
    );
}

#[test]
fn ingest_reports_the_graph_layer_outcome() {
    let a = keys(14);
    let b = keys(15);
    let mut graph = AgentCapabilityGraph::new(a.did(), CapGraphConfig::default());
    let first = announce(&b, &declaration(&b, 1), 1).expect("announce");
    assert!(matches!(
        ingest(&mut graph, &first, 1),
        Ingest::Routed { .. }
    ));
    assert!(ingest(&mut graph, &first, 1).is_routed());
    assert!(
        !ingest(&mut graph, &first, 1).is_applied(),
        "重放是 Unchanged 不是 Applied"
    );
    let newer = announce(&b, &declaration(&b, 2), 2).expect("announce");
    assert!(ingest(&mut graph, &newer, 2).is_applied());
    assert_eq!(graph.neighbor(&b.did()).expect("record").epoch, 2);
}

#[test]
fn scenario_broadcasts_and_reports_a_rejected_tamper() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let value = au4a_capgraph::scenario(&mut kernel).expect("scenario runs");
    assert_eq!(
        value["version"].as_str().map(|v| v.starts_with("v1.1.")),
        Some(true)
    );
    assert_eq!(value["announcements_sent"].as_u64(), Some(4));
    assert_eq!(value["send_failures"].as_u64(), Some(0));
    assert_eq!(value["pump"]["routed"].as_u64(), Some(4));
    assert_eq!(value["pump"]["applied"].as_u64(), Some(4));
    assert_eq!(value["tampered_rejected"].as_bool(), Some(true));
    assert_eq!(value["graph"]["capability_count"].as_u64(), Some(5));
    assert_eq!(kernel.agent_count(), 5);

    // 同样的输入 → 同样的输出（可重放）。
    let mut other = Kernel::new(KernelConfig::default());
    assert_eq!(
        au4a_capgraph::scenario(&mut other).expect("scenario runs"),
        value
    );
}
