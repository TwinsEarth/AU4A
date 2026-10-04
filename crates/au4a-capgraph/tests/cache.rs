//! v1.1.4 集成测试：邻居缓存层。
//!
//! 覆盖：容量硬上限、确定性 LRU 淘汰、逻辑刻 TTL 失效、显式失效、
//! 以及「缓存满了也照样接受新信息」这条反直觉但正确的语义。

use au4a_capgraph::{
    announce, demo_agents, ingest, pump, AgentCapabilityGraph, CapGraphConfig, Capability,
    CapabilityCache, Declaration, NeighborRecord, SkillId,
};
use au4a_core::{AgentKeys, Credits, Did};
use au4a_kernel::{Kernel, KernelConfig};

fn did(seed: u8) -> Did {
    AgentKeys::from_seed(&[seed; 32]).did()
}

fn cap(name: &str) -> Capability {
    Capability::new(SkillId::new(name).expect("valid"), Credits(2))
}

fn record(seed: u8, epoch: u64, at: u64) -> NeighborRecord {
    NeighborRecord {
        did: did(seed),
        epoch,
        at,
        fingerprint: format!("fp-{seed}-{epoch}"),
        capabilities: vec![cap("translate.en-zh")],
    }
}

fn declare(keys: &AgentKeys, epoch: u64, skills: &[&str]) -> Declaration {
    Declaration::new(
        keys.did(),
        epoch,
        epoch * 10,
        skills.iter().map(|s| cap(s)).collect(),
    )
    .expect("coherent")
}

#[test]
fn the_capacity_bound_holds_under_pressure() {
    let mut cache = CapabilityCache::new(8, 0);
    let mut peak = 0;
    for seed in 0..64u8 {
        cache.insert(record(seed, 1, 0), 0);
        peak = peak.max(cache.len());
    }
    assert!(peak <= 8, "任何时刻都不得超过容量：peak={peak}");
    assert_eq!(cache.len(), 8);
    assert_eq!(cache.stats().inserts, 64);
    assert_eq!(cache.stats().evictions, 56);
    // 最后写入的 8 个一定在（LRU 不会把自己刚写的淘汰掉）。
    for seed in 56..64u8 {
        assert!(cache.peek(&did(seed)).is_some(), "seed={seed}");
    }
}

#[test]
fn lru_eviction_picks_the_least_recently_used_deterministically() {
    let mut cache = CapabilityCache::new(3, 0);
    for seed in [1u8, 2, 3] {
        cache.insert(record(seed, 1, 0), 0);
    }
    // 触碰 1 与 2：3 成为最久未用。
    assert!(cache.get(&did(1), 0).is_some());
    assert!(cache.get(&did(2), 0).is_some());
    assert_eq!(cache.lru_order(), vec![did(3), did(1), did(2)]);
    let outcome = cache.insert(record(4, 1, 0), 0);
    assert_eq!(outcome.evicted, vec![did(3)]);

    // 同样的操作序列 → 同样的淘汰顺序（可重放）。
    let mut again = CapabilityCache::new(3, 0);
    for seed in [1u8, 2, 3] {
        again.insert(record(seed, 1, 0), 0);
    }
    again.get(&did(1), 0);
    again.get(&did(2), 0);
    assert_eq!(again.insert(record(4, 1, 0), 0).evicted, vec![did(3)]);
}

#[test]
fn ttl_expiry_is_driven_by_logical_ticks() {
    let mut cache = CapabilityCache::new(4, 20);
    cache.insert(record(1, 1, 1_000), 1_000);
    assert_eq!(cache.live_len(1_020), 1, "at+ttl 之内仍存活");
    assert_eq!(cache.live_len(1_021), 0, "at+ttl 之外失效");
    let expired = cache.expire(1_021);
    assert_eq!(expired, vec![did(1)]);
    assert_eq!(cache.len(), 0);
    assert_eq!(cache.stats().expirations, 1);

    // ttl = 0 表示永不过期。
    let mut forever = CapabilityCache::new(4, 0);
    forever.insert(record(1, 1, 0), 0);
    assert_eq!(forever.live_len(u64::MAX / 2), 1);
}

#[test]
fn an_expired_neighbor_is_removed_from_the_cache() {
    let owner = AgentKeys::from_seed(&[50; 32]);
    let peer = AgentKeys::from_seed(&[51; 32]);
    let mut graph = AgentCapabilityGraph::new(
        owner.did(),
        CapGraphConfig {
            cache_ttl_ticks: 10,
            ..CapGraphConfig::default()
        },
    );
    let first = declare(&peer, 3, &["translate.en-zh"]).sign(&peer).expect("signed");
    assert!(graph.apply(&first, 100).is_applied());
    // 同 epoch 异内容：没过期时是 conflict。
    let changed = declare(&peer, 3, &["sentiment.analyze"]).sign(&peer).expect("signed");
    assert_eq!(graph.apply(&changed, 105).refusal(), Some(au4a_core::RefusalCode::Conflict));
    // 过期之后缓存里没有数据了 —— 但 v1.1.7 起版本历史仍然记得 v3 的内容，
    // 所以「同版本异内容」依旧被拒；要换内容必须递增版本。
    assert_eq!(graph.expire_neighbors(200), vec![peer.did()]);
    assert!(graph.capabilities_of(&peer.did()).is_none(), "缓存确实被清掉了");
    assert_eq!(graph.apply(&changed, 200).refusal(), Some(au4a_core::RefusalCode::Conflict));
    let upgraded = declare(&peer, 4, &["sentiment.analyze"]).sign(&peer).expect("signed");
    assert!(graph.apply(&upgraded, 201).is_applied());
    assert_eq!(
        graph
            .capabilities_of(&peer.did())
            .expect("known")
            .iter()
            .map(|c| c.skill.as_str())
            .collect::<Vec<_>>(),
        vec!["sentiment.analyze"]
    );
}

#[test]
fn explicit_invalidation_removes_a_neighbor_without_touching_the_rest() {
    let owner = AgentKeys::from_seed(&[60; 32]);
    let a = AgentKeys::from_seed(&[61; 32]);
    let b = AgentKeys::from_seed(&[62; 32]);
    let mut graph = AgentCapabilityGraph::new(owner.did(), CapGraphConfig::default());
    graph.apply(&declare(&a, 1, &["x"]).sign(&a).expect("s"), 0);
    graph.apply(&declare(&b, 1, &["y"]).sign(&b).expect("s"), 0);
    assert_eq!(graph.neighbor_count(), 2);
    assert!(graph.invalidate_neighbor(&a.did()));
    assert!(!graph.invalidate_neighbor(&a.did()), "重复失效是空操作");
    assert_eq!(graph.neighbor_count(), 1);
    assert!(graph.capabilities_of(&b.did()).is_some());
    assert_eq!(graph.cache_stats().invalidations, 1);
    assert_eq!(graph.capability_count(), 1);
}

#[test]
fn a_cache_of_zero_capacity_refuses_with_a_capacity_code() {
    let owner = AgentKeys::from_seed(&[70; 32]);
    let peer = AgentKeys::from_seed(&[71; 32]);
    let mut graph = AgentCapabilityGraph::new(
        owner.did(),
        CapGraphConfig {
            neighbor_capacity: 0,
            ..CapGraphConfig::default()
        },
    );
    let signed = declare(&peer, 1, &["x"]).sign(&peer).expect("signed");
    let outcome = graph.apply(&signed, 0);
    assert_eq!(outcome.refusal(), Some(au4a_core::RefusalCode::ResourceExhausted));
    assert!(!outcome.refusal().expect("code").is_misconduct(), "容量是竞争语义，不是恶意");
    assert_eq!(graph.neighbor_count(), 0);
}

#[test]
fn hit_and_miss_counters_only_move_on_get() {
    let owner = AgentKeys::from_seed(&[80; 32]);
    let peer = AgentKeys::from_seed(&[81; 32]);
    let mut graph = AgentCapabilityGraph::new(owner.did(), CapGraphConfig::default());
    let signed = declare(&peer, 1, &["x"]).sign(&peer).expect("signed");
    graph.apply(&signed, 5);
    assert!(graph.neighbor(&peer.did()).is_some());
    assert_eq!(graph.cache_stats().hits, 0, "peek 不算命中");
    assert!(graph.get_neighbor(&peer.did(), 5).is_some());
    assert!(graph.get_neighbor(&did(200), 5).is_none());
    let stats = graph.cache_stats();
    assert_eq!((stats.hits, stats.misses), (1, 1));
}

#[test]
fn a_small_cache_bounds_the_graph_under_a_real_broadcast() {
    let agents = demo_agents().expect("demo agents");
    let mut kernel = Kernel::new(KernelConfig::default());
    for agent in &agents {
        kernel
            .register(&agent.keys, agent.display, &[], Credits(20))
            .expect("register");
    }
    for agent in &agents[1..] {
        let env = announce(
            &agent.keys,
            &Declaration::new(agent.keys.did(), 1, 0, agent.capabilities.clone()).expect("coherent"),
            1,
        )
        .expect("announce");
        kernel.send(&env).expect("accepted");
    }

    let observer = AgentKeys::from_seed(&[90; 32]);
    let mut graph = AgentCapabilityGraph::new(
        observer.did(),
        CapGraphConfig {
            neighbor_capacity: 2,
            cache_ttl_ticks: 0,
            ..CapGraphConfig::default()
        },
    );
    let report = pump(&mut kernel, &mut graph, 1);
    assert_eq!(report.routed, 4);
    assert_eq!(report.applied, 4, "满容量时仍然全部接受");
    assert_eq!(graph.neighbor_count(), 2);
    assert_eq!(graph.cache_stats().evictions, 2);
    assert!(graph.cache_capacity() == 2);

    // 缓存淘汰也走 `ingest` 这条真实路径，而不是直接调缓存 API。
    let mut again = AgentCapabilityGraph::new(
        observer.did(),
        CapGraphConfig {
            neighbor_capacity: 2,
            cache_ttl_ticks: 0,
            ..CapGraphConfig::default()
        },
    );
    for agent in &agents[1..] {
        let env = announce(
            &agent.keys,
            &Declaration::new(agent.keys.did(), 1, 0, agent.capabilities.clone()).expect("coherent"),
            1,
        )
        .expect("announce");
        assert!(ingest(&mut again, &env, 1).is_routed());
    }
    assert_eq!(again.lru_order(), graph.lru_order(), "淘汰结果可重放");
}

#[test]
fn scenario_reports_a_bounded_cache_demo() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let value = au4a_capgraph::scenario(&mut kernel).expect("scenario runs");
    let bounded = &value["bounded_cache"];
    assert_eq!(bounded["capacity"].as_u64(), Some(2));
    assert_eq!(bounded["retained"].as_u64(), Some(2));
    assert_eq!(bounded["evictions"].as_u64(), Some(2));
    assert_eq!(bounded["expired_at_tick_100"].as_u64(), Some(2));
    assert_eq!(bounded["retained_after_expiry"].as_u64(), Some(0));
    // 主图（默认容量）必须留全部 4 个邻居，否则后续版本的路径规划会缺提供者。
    assert_eq!(value["graph"]["neighbor_capacity"].as_u64(), Some(128));
    assert_eq!(value["pump"]["applied"].as_u64(), Some(4));
}
