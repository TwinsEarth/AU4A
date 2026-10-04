//! v1.1.7 集成测试：能力版本化。
//!
//! 三条主张：
//! 1. 版本号**由内容决定**：内容变则 +1，内容不变则不动（重发通告是幂等的）。
//! 2. 历史记录**内容寻址**：每条记录的 `hash` 就是那份声明的规范 JSON 指纹。
//! 3. 陈旧/冲突检测**不依赖缓存是否还活着**：缓存过期后，历史仍然抓住
//!    「同版本异内容」与「更旧版本」这两种情况。

use au4a_capgraph::{
    AgentCapabilityGraph, CapGraphConfig, Capability, Declaration, DeclareOutcome,
    SignedDeclaration, SkillId, HISTORY_CAPACITY,
};
use au4a_core::{AgentKeys, CoreError, Credits, Did, RefusalCode};
use au4a_kernel::{Kernel, KernelConfig};

fn keys(seed: u8) -> AgentKeys {
    AgentKeys::from_seed(&[seed; 32])
}

fn skill(name: &str) -> SkillId {
    SkillId::new(name).expect("valid skill")
}

fn cap(price: i64) -> Capability {
    Capability::new(skill("translate.en-zh"), Credits(price))
}

fn signed(keys: &AgentKeys, epoch: u64, price: i64) -> SignedDeclaration {
    Declaration::new(keys.did(), epoch, epoch, vec![cap(price)])
        .expect("coherent")
        .sign(keys)
        .expect("signed")
}

#[test]
fn declaring_the_same_content_twice_does_not_open_a_new_version() {
    let alice = keys(1);
    let mut graph = AgentCapabilityGraph::new(alice.did(), CapGraphConfig::default());
    let first = graph.declare(&alice, vec![cap(3)], 0).expect("declare");
    assert!(first.is_applied());
    assert_eq!(graph.own_epoch(), 1);
    let again = graph.declare(&alice, vec![cap(3)], 1).expect("declare");
    assert!(
        matches!(again, DeclareOutcome::Unchanged { .. }),
        "内容一致 → 版本不动"
    );
    assert_eq!(graph.own_epoch(), 1);
    assert_eq!(graph.history().len(), 1, "Unchanged 不写历史");
}

#[test]
fn every_content_change_bumps_the_version_and_is_content_addressed() {
    let alice = keys(2);
    let owner = alice.did();
    let mut graph = AgentCapabilityGraph::new(owner.clone(), CapGraphConfig::default());
    for (index, price) in [3i64, 4, 5].iter().enumerate() {
        let outcome = graph
            .declare(&alice, vec![cap(*price)], index as u64)
            .expect("declare");
        assert!(outcome.is_applied());
    }
    assert_eq!(graph.own_epoch(), 3);
    let history = graph.history();
    assert_eq!(history.len(), 3);
    assert_eq!(history.bumps(), 3);

    // 每条历史记录的 hash 必须等于那份**能力集合**的指纹（不是随便一个字符串）。
    for (index, price) in [3i64, 4, 5].iter().enumerate() {
        let version = index as u64 + 1;
        let expected = Declaration::new(owner.clone(), version, index as u64, vec![cap(*price)])
            .expect("coherent")
            .capabilities_fingerprint()
            .expect("hash");
        assert_eq!(history.hash_of(&owner, version), Some(expected.as_str()));
    }
    // 版本号不同 → 内容不同 → 哈希不同。
    assert_ne!(history.hash_of(&owner, 1), history.hash_of(&owner, 2));
}

#[test]
fn only_the_owner_can_declare_its_own_version() {
    let alice = keys(3);
    let bob = keys(4);
    let mut graph = AgentCapabilityGraph::new(alice.did(), CapGraphConfig::default());
    assert_eq!(
        graph.declare(&bob, vec![cap(1)], 0),
        Err(CoreError::InvalidSignature),
        "版本号的所有者必须与内容的所有者一致"
    );
    assert_eq!(graph.own_epoch(), 0);
}

#[test]
fn stale_announcements_are_refused_while_the_cache_is_alive() {
    let alice = keys(5);
    let bob = keys(6);
    let mut graph = AgentCapabilityGraph::new(alice.did(), CapGraphConfig::default());
    assert!(graph.apply(&signed(&bob, 3, 3), 0).is_applied());
    assert_eq!(
        graph.apply(&signed(&bob, 2, 3), 1).refusal(),
        Some(RefusalCode::StaleEpoch)
    );
    assert_eq!(
        graph.apply(&signed(&bob, 3, 4), 2).refusal(),
        Some(RefusalCode::Conflict)
    );
    assert!(graph.apply(&signed(&bob, 4, 4), 3).is_applied());
    assert_eq!(graph.last_version_of(&bob.did()), Some(4));
    assert_eq!(graph.neighbor(&bob.did()).expect("record").epoch, 4);
}

#[test]
fn history_catches_conflicts_after_the_cache_entry_expires() {
    let alice = keys(7);
    let bob = keys(8);
    let mut graph = AgentCapabilityGraph::new(
        alice.did(),
        CapGraphConfig {
            cache_ttl_ticks: 10,
            ..CapGraphConfig::default()
        },
    );
    assert!(graph.apply(&signed(&bob, 5, 5), 100).is_applied());
    // 缓存条目过期并被清理：此时只看缓存就"没听过" bob 了。
    assert_eq!(graph.expire_neighbors(1_000), vec![bob.did()]);
    assert!(graph.neighbor(&bob.did()).is_none());

    // 同版本异内容 → 仍然 conflict（历史兜底）。
    assert_eq!(
        graph.apply(&signed(&bob, 5, 999), 1_001).refusal(),
        Some(RefusalCode::Conflict)
    );
    // 更旧的版本 → 仍然 stale_epoch。
    assert_eq!(
        graph.apply(&signed(&bob, 4, 5), 1_002).refusal(),
        Some(RefusalCode::StaleEpoch)
    );
    // 更新的版本 → 正常接受。
    assert!(graph.apply(&signed(&bob, 6, 6), 1_003).is_applied());
    assert_eq!(graph.last_version_of(&bob.did()), Some(6));
}

#[test]
fn history_is_bounded_but_versions_keep_climbing() {
    let alice = keys(9);
    let mut graph = AgentCapabilityGraph::new(alice.did(), CapGraphConfig::default());
    let total = HISTORY_CAPACITY + 7;
    for index in 0..total {
        let outcome = graph
            .declare(&alice, vec![cap(index as i64 + 1)], index as u64)
            .expect("declare");
        assert!(outcome.is_applied());
    }
    assert_eq!(graph.history().len(), HISTORY_CAPACITY, "有界记忆");
    assert_eq!(graph.own_epoch(), total as u64);
    assert_eq!(graph.history().bumps(), total as u64);
    assert!(
        graph.history().hash_of(&alice.did(), 1).is_none(),
        "最旧的记录已被丢弃"
    );
}

#[test]
fn the_version_summary_reports_own_and_neighbors() {
    let alice = keys(10);
    let bob = keys(11);
    let carol = keys(12);
    let mut graph = AgentCapabilityGraph::new(alice.did(), CapGraphConfig::default());
    graph.declare(&alice, vec![cap(3)], 0).expect("declare");
    graph.apply(&signed(&bob, 2, 2), 0);
    graph.apply(&signed(&carol, 7, 7), 0);
    let summary = graph.version_summary();
    assert_eq!(summary["own"].as_u64(), Some(1));
    assert_eq!(summary["history"].as_u64(), Some(3));
    assert_eq!(summary["bumps"].as_u64(), Some(3));
    let neighbors = summary["neighbors"].as_object().expect("map");
    assert_eq!(
        neighbors.get(bob.did().as_str()).and_then(|v| v.as_u64()),
        Some(2)
    );
    assert_eq!(
        neighbors.get(carol.did().as_str()).and_then(|v| v.as_u64()),
        Some(7)
    );
    assert!(summary["own_hash"].as_str().map(str::len) == Some(64));
}

#[test]
fn a_broadcast_carries_the_version_and_the_receiver_detects_staleness() {
    let agents = au4a_capgraph::demo_agents().expect("demo agents");
    let alice = &agents[0];
    let bob = &agents[1];
    let mut kernel = Kernel::new(KernelConfig::default());
    for agent in &agents {
        kernel
            .register(&agent.keys, agent.display, &[], Credits(20))
            .expect("register");
    }
    let mut graph = AgentCapabilityGraph::new(alice.keys.did(), CapGraphConfig::default());

    // bob 先广播 v1，再广播 v2，最后重放 v1（陈旧）。
    for epoch in [1u64, 2] {
        let declaration = Declaration::new(bob.keys.did(), epoch, epoch, bob.capabilities.clone())
            .expect("coherent");
        let env = au4a_capgraph::announce(&bob.keys, &declaration, epoch).expect("announce");
        kernel.send(&env).expect("accepted");
        assert!(au4a_capgraph::pump(&mut kernel, &mut graph, epoch).applied == 1);
    }
    assert_eq!(graph.neighbor(&bob.keys.did()).expect("record").epoch, 2);
    let replay =
        Declaration::new(bob.keys.did(), 1, 1, bob.capabilities.clone()).expect("coherent");
    let env = au4a_capgraph::announce(&bob.keys, &replay, 3).expect("announce");
    kernel.send(&env).expect("accepted");
    let report = au4a_capgraph::pump(&mut kernel, &mut graph, 3);
    assert_eq!(report.routed, 1);
    assert_eq!(report.applied, 0, "陈旧通告不改变图");
    assert_eq!(report.refused, 1);
    let (who, refusal) = kernel.refusals().last().expect("refusal recorded");
    assert_eq!(who, &bob.keys.did());
    assert_eq!(refusal.code, RefusalCode::StaleEpoch);
    assert!(!refusal.code.is_misconduct(), "陈旧是竞争语义，不是恶意");
    assert_eq!(graph.neighbor(&bob.keys.did()).expect("record").epoch, 2);
}

#[test]
fn scenario_reports_version_bumps_and_a_refused_stale_announcement() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let value = au4a_capgraph::scenario(&mut kernel).expect("scenario runs");
    assert_eq!(value["version_bump_applied"].as_bool(), Some(true));
    assert_eq!(value["idempotent_ignored"].as_bool(), Some(true));
    assert_eq!(value["stale_rejected"].as_bool(), Some(true));
    assert_eq!(
        value["versions"]["own"].as_u64(),
        Some(2),
        "v1 建立 + v2 改价"
    );
    assert_eq!(
        value["versions"]["bumps"].as_u64(),
        Some(6),
        "2 次自有 + 4 个邻居"
    );
    assert_eq!(value["versions"]["history"].as_u64(), Some(6));
    // 版本化没有破坏 v1.1.6 的路径规划结论。
    assert_eq!(value["plan"]["outcome"].as_str(), Some("path"));
    assert_eq!(value["plan"]["pipeline"]["total_price"].as_u64(), Some(5));
    let stale_count = kernel
        .refusals()
        .iter()
        .filter(|(_, r)| r.code == RefusalCode::StaleEpoch)
        .count();
    assert_eq!(
        stale_count, 0,
        "陈旧演示不经过共享内核，避免污染其他轨道看到的拒绝记录"
    );
    let did: Did = agents_did(&value);
    assert!(did.as_str().starts_with("did:au4a:"));
}

/// 从 scenario 输出里取一个示意 DID（保证 JSON 里的 did 字段形状正确）。
fn agents_did(value: &serde_json::Value) -> Did {
    let raw = value["graph"]["owner"].as_str().expect("owner did");
    Did::parse(raw).expect("valid did")
}
