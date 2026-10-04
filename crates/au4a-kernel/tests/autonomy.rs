//! v1.0.2 Agent 自治层的集成测试（只用公开 API）。
//!
//! 要证明的是理念层面的东西，而不只是「函数能跑」：
//! * Agent 自己注册、自己通告、自己出价、自己结算，全程没有任何人类入口；
//! * 竞争性拒绝 → 退避（不隔离），恶意性拒绝 → 自我暂停（不等判决）；
//! * 同种子的运行时在两台内核上产生完全相同的日志（可重放）。

use au4a_core::{AgentKeys, Credits, Envelope, RefusalCode};
use au4a_kernel::{
    AutonomyLayer, AutonomyPolicy, IntentKind, Kernel, KernelConfig, results_json,
};

fn seed(tag: u8) -> AgentKeys {
    AgentKeys::from_seed(&[tag; 32])
}

fn join(kernel: &mut Kernel, tag: u8, skill: &str) -> AutonomyLayer {
    let mut layer = AutonomyLayer::new(seed(tag), AutonomyPolicy::default());
    layer.join(kernel, "autonomous", &[skill], Credits(20)).unwrap();
    layer
}

#[test]
fn an_agent_registers_announces_and_offers_without_any_human_step() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let mut host = join(&mut kernel, 31, "kernel.host");
    let peer = join(&mut kernel, 32, "kernel.settle");

    // 第一回合：自己决定通告（广播，先签名后发送）。
    let first = host.turn(&mut kernel).unwrap();
    assert_eq!(first.planned, vec![IntentKind::Announce]);
    assert_eq!(first.sent, 1);
    assert!(kernel.queued_envelopes().iter().all(|e| e.verify().is_ok()));

    // 对端自主发来报价请求 → 第二回合 host 自己决定回价（不需要任何人批准）。
    let request = Envelope::new(
        peer.did(),
        Some(host.did()),
        "negotiate.offer",
        kernel.now() + 1,
        None,
        serde_json::json!({"skill": "kernel.host"}),
    )
    .unwrap()
    .seal(&seed(32))
    .unwrap();
    kernel.send(&request).unwrap();

    kernel.tick();
    kernel.tick();
    kernel.tick();
    kernel.tick();
    let second = host.turn(&mut kernel).unwrap();
    assert!(second.planned.contains(&IntentKind::Announce));
    assert!(second.planned.contains(&IntentKind::Offer));
    assert_eq!(second.sent, 2);
    assert!(!host.state().halted);

    // 没有任何意图是「等待人类批准」这种形状，决策输入里也没有人类通道。
    let context = host.context(&kernel);
    let keys: Vec<String> = serde_json::to_value(&context)
        .unwrap()
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    for key in keys {
        let lower = key.to_lowercase();
        assert!(!lower.contains("human") && !lower.contains("approv") && !lower.contains("operator"));
    }
    for kind in IntentKind::ALL {
        let name = kind.as_str();
        assert!(!name.contains("approv") && !name.contains("human"));
    }
}

#[test]
fn a_race_causes_backoff_while_misconduct_causes_self_suspension() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let mut layer = join(&mut kernel, 33, "kernel.settle");
    let payee = seed(34);
    kernel.register(&payee, "payee", &["x"], Credits(20)).unwrap();

    // 竞争路径：对端请求一笔 unverified 结算 → 证据闸门拒绝 → 退避，绝不隔离。
    let request = Envelope::new(
        payee.did(),
        Some(layer.did()),
        "settle.request",
        kernel.now() + 1,
        None,
        serde_json::json!({"amount": 9, "grade": "unverified"}),
    )
    .unwrap()
    .seal(&payee)
    .unwrap();
    kernel.send(&request).unwrap();
    let turn = layer.turn(&mut kernel).unwrap();
    assert!(turn.refused.contains(&RefusalCode::PolicyDenied));
    assert_eq!(layer.state().defer_until, Some(turn.at + 3), "竞争 → 退避");
    assert!(!layer.state().halted);
    assert_ne!(
        kernel.escalation_for(&layer.did(), RefusalCode::PolicyDenied),
        au4a_core::Escalation::Quarantine,
        "竞争性拒绝绝不能升级为隔离"
    );

    // 恶意路径：有人用同一个 DID 的种子伪造信封 → 内核记 unauthorized → Agent 自我暂停。
    let mut forged = Envelope::new(
        layer.did(),
        None,
        "progress.event",
        kernel.now() + 1,
        None,
        serde_json::json!({"forged": true}),
    )
    .unwrap()
    .seal(&seed(33))
    .unwrap();
    forged.body = serde_json::json!({"forged": "tampered"});
    assert!(kernel.send(&forged).is_err());

    let halted_turn = layer.turn(&mut kernel).unwrap();
    assert_eq!(halted_turn.planned, vec![IntentKind::Suspend]);
    assert_eq!(halted_turn.sent, 0, "自停后不再产生任何流量");
    assert!(layer.state().halted);
    assert!(kernel.audit().is_clean());
}

#[test]
fn same_seed_produces_the_same_autonomous_journal() {
    let run = || {
        let mut kernel = Kernel::new(KernelConfig::default());
        let mut layer = join(&mut kernel, 35, "kernel.host");
        let mut turns = Vec::new();
        for _ in 0..6 {
            kernel.tick();
            turns.push(layer.turn(&mut kernel).unwrap());
        }
        (turns, layer.state().clone(), kernel.registry_fingerprint().unwrap())
    };
    let (a_turns, a_state, a_fp) = run();
    let (b_turns, b_state, b_fp) = run();
    assert_eq!(a_turns, b_turns);
    assert_eq!(a_state, b_state);
    assert_eq!(a_fp, b_fp);
    assert!(a_state.announcements >= 2, "心跳通告会重复发生");
}

#[test]
fn the_journal_accounts_for_every_plan_without_a_human_gate() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let mut layer = join(&mut kernel, 36, "kernel.host");
    for _ in 0..5 {
        layer.turn(&mut kernel).unwrap();
    }
    let journal = layer.journal();
    assert_eq!(journal.len(), layer.state().intents_planned as usize);
    for record in journal {
        assert!(IntentKind::ALL.contains(&record.intent.kind()));
    }
    assert!(journal.iter().any(|r| r.acted));
    assert!(journal.iter().any(|r| !r.acted), "Idle/Defer 只是决定，不动内核");
    assert!(results_json().unwrap()["all_passed"].as_bool().unwrap_or(false));
}
