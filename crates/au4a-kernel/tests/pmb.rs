//! v1.0.6 PMB 协议扩展的集成测试（只用公开 API）。
//!
//! 要证明：扩展后的协议**仍然是冻结基元的信封**（真签名、真分帧、真上限），
//! 而且路由判定的拒绝分类与内核一致——准入通过的，内核也必须收下。

use au4a_core::{AgentKeys, Envelope};
use au4a_core::{Credits, EvidenceGrade, RefusalCode, MAX_FRAME};
use au4a_kernel::{
    announce_card, classify_kind, council_vote, decode_and_verify, encode_checked, kinds_ext,
    progress, settle_request, Kernel, KernelConfig, MessageClass, PmbRouter,
};

fn keys(tag: u8) -> AgentKeys {
    AgentKeys::from_seed(&[tag; 32])
}

fn seeded(n: u8) -> Kernel {
    let mut k = Kernel::new(KernelConfig::default());
    for i in 0..n {
        k.register(
            &keys(150 + i),
            format!("agent-{i}"),
            &["skill.a"],
            Credits(20),
        )
        .unwrap();
    }
    k
}

#[test]
fn anything_the_router_admits_the_kernel_also_accepts() {
    let mut k = seeded(3);
    let card = k.card(&keys(150).did()).cloned().unwrap();
    let envelopes = vec![
        announce_card(&keys(150), &card, 0).unwrap(),
        progress(
            &keys(150),
            None,
            0,
            "heartbeat",
            serde_json::json!({"ok": true}),
        )
        .unwrap(),
        settle_request(
            &keys(150),
            keys(151).did(),
            0,
            Credits(4),
            EvidenceGrade::Verified,
        )
        .unwrap(),
        council_vote(&keys(151), keys(152).did(), 0, "m1", "uphold").unwrap(),
    ];
    let mut router = PmbRouter::new();
    for env in &envelopes {
        let decision = router.admit(&k, env);
        assert!(
            decision.accepted,
            "{} 应当准入：{}",
            env.kind.as_str(),
            decision.reason
        );
        // 路由通过 ⇒ 内核也必须收下（否则两条路径的语义就不一致了）。
        assert!(
            k.send(env).is_ok(),
            "路由准入但内核拒绝：{}",
            env.kind.as_str()
        );
    }
    assert_eq!(router.stats().admitted, envelopes.len() as u64);
    assert_eq!(router.stats().refused, 0);
    assert!(k.audit().is_clean());
}

#[test]
fn tampering_is_misconduct_while_replaying_is_competition() {
    let k = seeded(2);
    let mut router = PmbRouter::new();

    let mut tampered =
        announce_card(&keys(150), &k.card(&keys(150).did()).cloned().unwrap(), 0).unwrap();
    tampered.body = serde_json::json!({"skills": ["everything"]});
    let decision = router.admit(&k, &tampered);
    assert_eq!(decision.code, Some(RefusalCode::Unauthorized));
    assert!(decision.code.map(|c| c.is_misconduct()).unwrap_or(false));

    let good = progress(&keys(150), None, 0, "ok", serde_json::json!({})).unwrap();
    assert!(router.admit(&k, &good).accepted);
    let replay = router.admit(&k, &good);
    assert_eq!(replay.code, Some(RefusalCode::Conflict));
    assert!(
        !replay.code.map(|c| c.is_misconduct()).unwrap_or(true),
        "重传不是恶意"
    );
    assert!(
        replay.code.map(|c| c.retryable()).unwrap_or(false),
        "冲突可重试"
    );
    assert_eq!(router.stats().replays, 1);
}

#[test]
fn routing_never_mutates_the_kernel() {
    let k = seeded(2);
    let before_registry = k.registry_fingerprint().unwrap();
    let before_view = k.observe_json();
    let before_queue = k.queue_len();
    let mut router = PmbRouter::new();
    let env = progress(&keys(150), None, 0, "noop", serde_json::json!({})).unwrap();
    assert!(router.admit(&k, &env).accepted);
    assert_eq!(before_registry, k.registry_fingerprint().unwrap());
    assert_eq!(before_view, k.observe_json());
    assert_eq!(before_queue, k.queue_len(), "路由是判定，不写队列");
}

#[test]
fn wire_format_is_the_frozen_frame_with_real_signatures() {
    let card = au4a_kernel::AgentCard {
        did: keys(150).did(),
        display: "frame-test".to_string(),
        skills: vec!["skill.a".to_string()],
        stake: Credits(20),
        evidence: EvidenceGrade::Verified,
    };
    let env = announce_card(&keys(150), &card, 5).unwrap();
    let frame = encode_checked(&env).unwrap();
    assert!(frame.len() <= MAX_FRAME + 4);
    let decoded = decode_and_verify(&frame).unwrap();
    assert_eq!(decoded.id, env.id);
    assert_eq!(decoded.sig, env.sig);
    assert!(decoded.verify().is_ok());
    // 改一个字节：验签必须失败。
    let mut broken = frame.clone();
    let last = broken.len() - 1;
    broken[last] ^= 0x01;
    assert!(decode_and_verify(&broken).is_err());
    // 1 MiB 上限：前缀超限在分配之前就被拒绝。
    let mut oversized = Vec::from((MAX_FRAME as u32 + 1).to_be_bytes());
    oversized.extend_from_slice(b"{}");
    assert!(decode_and_verify(&oversized).is_err());
}

#[test]
fn governance_must_be_direct_while_telemetry_may_broadcast() {
    let k = seeded(3);
    let mut router = PmbRouter::new();
    assert_eq!(
        classify_kind(kinds_ext::COUNCIL_VOTE),
        MessageClass::Governance
    );
    assert!(!MessageClass::Governance.broadcastable());
    let broadcast_motion = Envelope::new(
        keys(150).did(),
        None,
        kinds_ext::COUNCIL_MOTION,
        0,
        None,
        serde_json::json!({"motion": "m1"}),
    )
    .unwrap()
    .seal(&keys(150))
    .unwrap();
    let decision = router.admit(&k, &broadcast_motion);
    assert_eq!(decision.code, Some(RefusalCode::PolicyDenied));
    assert!(!decision.code.map(|c| c.is_misconduct()).unwrap_or(true));

    let direct_vote = council_vote(&keys(150), keys(151).did(), 0, "m1", "uphold").unwrap();
    assert!(router.admit(&k, &direct_vote).accepted);
}

#[test]
fn identical_runs_produce_identical_decisions_and_stats() {
    let run = || {
        let k = seeded(4);
        let mut router = PmbRouter::new();
        let mut decisions = Vec::new();
        decisions.push(router.admit(
            &k,
            &progress(&keys(150), None, 0, "a", serde_json::json!({})).unwrap(),
        ));
        decisions.push(router.admit(
            &k,
            &progress(&keys(150), None, 0, "a", serde_json::json!({})).unwrap(),
        ));
        decisions.push(
            router.admit(
                &k,
                &settle_request(
                    &keys(151),
                    keys(152).did(),
                    0,
                    Credits(2),
                    EvidenceGrade::CpuProto,
                )
                .unwrap(),
            ),
        );
        decisions.push(
            router.admit(
                &k,
                &Envelope::new(
                    keys(152).did(),
                    None,
                    "nope.nope",
                    0,
                    None,
                    serde_json::json!({}),
                )
                .unwrap()
                .seal(&keys(152))
                .unwrap(),
            ),
        );
        (decisions, router.stats().clone())
    };
    let a = run();
    let b = run();
    assert_eq!(a, b);
    assert_eq!(a.1.admitted, 2);
    assert_eq!(a.1.refused, 2);
    assert_eq!(a.1.replays, 1);
}
