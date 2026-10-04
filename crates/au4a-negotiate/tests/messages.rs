//! v1.2.1 集成测试：六种协商消息作为已签名 PMB 信封真实投递。
//!
//! 覆盖：正常路径（双方自主注册并交换全部六种消息）、拒绝路径（伪造/篡改/未注册）、
//! 不变式（内容寻址 id 自洽、场景可重复）。

use au4a_core::{AgentKeys, CoreError, Credits, Envelope, EvidenceGrade, RefusalCode};
use au4a_kernel::{Kernel, KernelConfig};
use au4a_negotiate::{kinds, msg, BreachKind, NegotiationMsg, Terms, ALL_KINDS};

fn kernel() -> Kernel {
    Kernel::new(KernelConfig::default())
}

fn agent(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

fn terms(price: i64) -> Terms {
    Terms::new("summarize.zh", Credits(price), 40, EvidenceGrade::Verified).unwrap()
}

#[test]
fn two_agents_negotiate_six_message_types_over_pmb() {
    let mut k = kernel();
    let a = agent(1);
    let b = agent(2);
    k.register(&a, "proposer", &["summarize.zh"], Credits(20))
        .unwrap();
    k.register(&b, "responder", &["summarize.zh"], Credits(20))
        .unwrap();

    let session = msg::session_id(&a.did(), &b.did(), &terms(120)).unwrap();
    let h = terms(100).hash().unwrap();
    // `AgentKeys` 不可克隆（私钥不出结构体），所以每一轮用同一个种子重建同一把密钥。
    let outbound = vec![
        (
            agent(1),
            b.did(),
            NegotiationMsg::request(&session, terms(120)).unwrap(),
        ),
        (
            agent(2),
            a.did(),
            NegotiationMsg::counter(&session, 1, terms(100)).unwrap(),
        ),
        (
            agent(1),
            b.did(),
            NegotiationMsg::accept(&session, 1, &h).unwrap(),
        ),
        (
            agent(2),
            a.did(),
            NegotiationMsg::reject(&session, 2, "deadline too tight").unwrap(),
        ),
        (
            agent(1),
            b.did(),
            NegotiationMsg::sign_contract("c-1", &h).unwrap(),
        ),
        (
            agent(2),
            a.did(),
            NegotiationMsg::breach("c-1", &h, BreachKind::LateDelivery, "delivered late").unwrap(),
        ),
    ];

    let mut previous: Option<String> = None;
    let mut sent = 0usize;
    for (from, to, m) in &outbound {
        let ts = k.tick();
        let env = m.signed(from, to, ts, previous.clone()).unwrap();
        assert!(k.send(&env).unwrap().accepted, "{}", m.kind());
        previous = Some(env.id.clone());
        sent += 1;
    }
    assert_eq!(sent, ALL_KINDS.len());
    assert_eq!(k.queue_len(), 6);

    let delivered = k.drain();
    assert_eq!(delivered.len(), 6);
    for (env, (_, _, expected)) in delivered.iter().zip(outbound.iter()) {
        env.verify().unwrap();
        assert_eq!(NegotiationMsg::from_env(env).unwrap(), *expected);
        assert_eq!(env.id, env.compute_id().unwrap());
    }
    let kinds_seen: Vec<&str> = delivered.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds_seen, ALL_KINDS.to_vec());
    assert!(k.ledger().check_conservation().is_ok());
}

#[test]
fn a_forged_negotiation_message_is_refused_as_misconduct() {
    let mut k = kernel();
    let a = agent(3);
    let b = agent(4);
    k.register(&a, "a", &[], Credits(20)).unwrap();
    k.register(&b, "b", &[], Credits(20)).unwrap();

    let mut env = NegotiationMsg::request("s-forged", terms(10))
        .unwrap()
        .signed(&a, &b.did(), 1, None)
        .unwrap();
    env.body = NegotiationMsg::counter("s-forged", 1, terms(1))
        .unwrap()
        .body()
        .unwrap();

    assert_eq!(k.send(&env), Err(CoreError::InvalidSignature));
    assert_eq!(k.queue_len(), 0);
    let (who, refusal) = &k.refusals()[0];
    assert_eq!(who, &a.did());
    assert_eq!(refusal.code, RefusalCode::Unauthorized);
    assert!(refusal.code.is_misconduct());
    assert_eq!(
        k.escalation_for(&a.did(), RefusalCode::Unauthorized),
        au4a_core::Escalation::Quarantine
    );
}

#[test]
fn an_unregistered_sender_cannot_negotiate() {
    let mut k = kernel();
    let b = agent(6);
    k.register(&b, "b", &[], Credits(20)).unwrap();
    let outsider = agent(5);
    let env = NegotiationMsg::request("s1", terms(10))
        .unwrap()
        .signed(&outsider, &b.did(), 1, None)
        .unwrap();
    assert_eq!(k.send(&env), Err(CoreError::UnknownAgent));
    assert_eq!(k.refusals()[0].1.code, RefusalCode::Unauthorized);
}

#[test]
fn a_reply_must_reference_an_existing_parent_envelope() {
    let mut k = kernel();
    let a = agent(7);
    let b = agent(8);
    k.register(&a, "a", &[], Credits(20)).unwrap();
    k.register(&b, "b", &[], Credits(20)).unwrap();
    let request = NegotiationMsg::request("s1", terms(10)).unwrap();
    let env = request.signed(&a, &b.did(), 1, None).unwrap();
    k.send(&env).unwrap();
    let reply = NegotiationMsg::counter("s1", 1, terms(9))
        .unwrap()
        .signed(&b, &a.did(), 2, Some(env.id.clone()))
        .unwrap();
    assert_eq!(reply.in_reply_to.as_deref(), Some(env.id.as_str()));
    k.send(&reply).unwrap();
    let delivered = k.drain();
    assert_eq!(
        delivered[1].in_reply_to.as_deref(),
        Some(delivered[0].id.as_str())
    );
}

#[test]
fn unsigned_envelopes_never_reach_the_queue() {
    let mut k = kernel();
    let a = agent(9);
    let b = agent(10);
    k.register(&a, "a", &[], Credits(20)).unwrap();
    k.register(&b, "b", &[], Credits(20)).unwrap();
    let unsealed = Envelope::new(
        a.did(),
        Some(b.did()),
        kinds::NEGOTIATE_REQUEST,
        1,
        None,
        NegotiationMsg::request("s1", terms(10))
            .unwrap()
            .body()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(k.send(&unsealed), Err(CoreError::NotSealed));
    assert_eq!(k.refusals()[0].1.code, RefusalCode::Unauthorized);
}

#[test]
fn scenario_is_repeatable_and_idempotent() {
    let mut k1 = kernel();
    let first = au4a_negotiate::scenario(&mut k1).unwrap();
    let second = au4a_negotiate::scenario(&mut k1).unwrap();

    // 同一个内核上再跑一次：逻辑时钟已经前进，信封 id 必然不同（时间进了签名载荷），
    // 但会话、消息种类与投递数必须完全一致——场景是结构可重复的，不是靠冻结时钟伪装出来的。
    assert_eq!(first["session"], second["session"]);
    assert_eq!(first["delivered"], second["delivered"]);
    assert_eq!(
        au4a_negotiate::transcript_kinds(&first),
        au4a_negotiate::transcript_kinds(&second)
    );
    assert_ne!(first["transcript"][0]["id"], second["transcript"][0]["id"]);
    assert!(
        second["transcript"][0]["ts"].as_u64().unwrap()
            > first["transcript"][0]["ts"].as_u64().unwrap()
    );

    // 换一个全新的内核、同样的种子：逐字段相同（brief §2 的可重复要求）。
    let mut k2 = kernel();
    let other = au4a_negotiate::scenario(&mut k2).unwrap();
    assert_eq!(first, other, "同样的种子给同样的结果");

    let kinds_seen = au4a_negotiate::transcript_kinds(&first);
    assert_eq!(
        kinds_seen,
        vec![
            kinds::NEGOTIATE_REQUEST.to_string(),
            kinds::NEGOTIATE_COUNTER.to_string(),
            kinds::NEGOTIATE_REJECT.to_string(),
            kinds::NEGOTIATE_COUNTER.to_string(),
            kinds::NEGOTIATE_ACCEPT.to_string(),
            kinds::CONTRACT_SIGN.to_string(),
            kinds::CONTRACT_SIGN.to_string()
        ]
    );
    let (p, r) = au4a_negotiate::scenario_dids();
    assert!(first["session"].as_str().unwrap().len() == 64);
    assert_ne!(p, r);
    assert!(au4a_core::all_passed(&au4a_negotiate::self_check()));
    assert_eq!(
        au4a_negotiate::results_json().unwrap()["kinds"][0],
        kinds::NEGOTIATE_REQUEST
    );
}
