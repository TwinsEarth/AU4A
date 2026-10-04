//! v1.1.5 集成测试：查询接口与倒排索引。
//!
//! 核心断言只有一句话：**查询只扫索引给出的候选，不扫全图**，
//! 并且索引给出的结果与「遍历全图逐个过滤」的结果完全一致。

use au4a_capgraph::{
    AgentCapabilityGraph, CapGraphConfig, Capability, CapabilityQuery, Declaration, FormatId,
    SkillId,
};
use au4a_core::{AgentKeys, Credits, Did, RefusalCode};
use au4a_kernel::{Kernel, KernelConfig};

fn skill(name: &str) -> SkillId {
    SkillId::new(name).expect("valid skill")
}

fn format(name: &str) -> FormatId {
    FormatId::new(name).expect("valid format")
}

fn cap(name: &str, price: i64) -> Capability {
    Capability::new(skill(name), Credits(price))
}

/// 建一个「100 个邻居 × 2 条能力 = 200 条能力」的视图，容量足够大。
fn big_graph() -> AgentCapabilityGraph {
    let owner = AgentKeys::from_seed(&[200; 32]);
    let mut graph = AgentCapabilityGraph::new(owner.did(), CapGraphConfig::default());
    for seed in 0..100u8 {
        let peer = AgentKeys::from_seed(&[seed; 32]);
        let caps = vec![
            cap("translate.en-zh", 3),
            cap("sentiment.analyze", 2),
        ];
        let signed = Declaration::new(peer.did(), 1, 1, caps)
            .expect("coherent")
            .sign(&peer)
            .expect("signed");
        assert!(graph.apply(&signed, 1).is_applied());
    }
    graph
}

#[test]
fn a_query_scans_only_the_index_candidates() {
    let mut graph = big_graph();
    let result = graph.query(&CapabilityQuery::new(skill("translate.en-zh")), 1);
    assert_eq!(result.stats.nodes_total, 200, "视图里共 200 条能力");
    assert_eq!(result.stats.candidates, 100, "索引只给出该技能的 100 条");
    assert_eq!(result.stats.scanned, 100);
    assert!(
        result.stats.scanned * 2 == result.stats.nodes_total,
        "扫描量是索引候选，不是全图"
    );
    assert_eq!(result.stats.matched, 100);
    assert_eq!(result.matches.len(), 8, "默认 limit=8");
}

#[test]
fn index_results_equal_a_brute_force_scan() {
    let mut graph = big_graph();
    let query = CapabilityQuery::new(skill("sentiment.analyze"))
        .with_max_price(Credits(5))
        .with_min_reliability_bp(1)
        .with_limit(0);
    let indexed = graph.query(&query, 1);

    // 朴素全扫：把图里每条能力都拿来做同一个谓词。
    let mut brute: Vec<Did> = Vec::new();
    for record in graph.neighbors() {
        for capability in &record.capabilities {
            if query.matches(capability) {
                brute.push(record.did.clone());
            }
        }
    }
    brute.sort();
    brute.dedup();
    let mut from_index: Vec<Did> = indexed.matches.iter().map(|m| m.did.clone()).collect();
    from_index.sort();
    from_index.dedup();
    assert_eq!(from_index, brute, "索引结果 == 全扫结果（索引不是近似）");
    assert_eq!(indexed.stats.scanned, 100);
}

#[test]
fn every_filter_is_a_hard_condition() {
    let owner = AgentKeys::from_seed(&[210; 32]);
    let mut graph = AgentCapabilityGraph::new(owner.did(), CapGraphConfig::default());
    let cheap_bad = cap("translate.en-zh", 1).with_reliability_bp(5_000).with_latency(50, 100);
    let dear_good = cap("translate.en-zh", 50).with_reliability_bp(9_900).with_latency(500, 5_000);
    for (seed, capability) in [(211u8, cheap_bad), (212, dear_good)] {
        let peer = AgentKeys::from_seed(&[seed; 32]);
        let signed = Declaration::new(peer.did(), 1, 1, vec![capability])
            .expect("coherent")
            .sign(&peer)
            .expect("signed");
        graph.apply(&signed, 1);
    }
    let all = graph.query(&CapabilityQuery::new(skill("translate.en-zh")), 1);
    assert_eq!(all.matches.len(), 2);
    assert_eq!(all.matches[0].capability.price_per_unit, Credits(50), "可靠度优先于价格");

    let gated = graph.query(
        &CapabilityQuery::new(skill("translate.en-zh"))
            .with_max_price(Credits(10))
            .with_min_reliability_bp(9_000),
        1,
    );
    assert!(gated.is_empty(), "没有任何候选同时满足两个硬条件");
    assert_eq!(gated.refusal_code(), Some(RefusalCode::Unsupported));
    assert!(!gated.refusal_code().expect("code").is_misconduct(), "查不到不是恶意");
    assert_eq!(gated.stats.matched, 0);
    assert_eq!(gated.stats.scanned, 2, "过滤在候选集内完成");
}

#[test]
fn format_filters_use_the_intersection_of_two_indexes() {
    let owner = AgentKeys::from_seed(&[220; 32]);
    let mut graph = AgentCapabilityGraph::new(owner.did(), CapGraphConfig::default());
    let text_to_json = cap("translate.en-zh", 3)
        .with_formats(&["text/plain"], &["application/json"])
        .expect("valid");
    let json_to_json = cap("translate.en-zh", 4)
        .with_formats(&["application/json"], &["application/json"])
        .expect("valid");
    for (seed, capability) in [(221u8, text_to_json), (222, json_to_json)] {
        let peer = AgentKeys::from_seed(&[seed; 32]);
        let signed = Declaration::new(peer.did(), 1, 1, vec![capability])
            .expect("coherent")
            .sign(&peer)
            .expect("signed");
        graph.apply(&signed, 1);
    }
    let only_json = graph.query(
        &CapabilityQuery::new(skill("translate.en-zh")).with_input_format(format("application/json")),
        1,
    );
    assert_eq!(only_json.stats.candidates, 1, "两个索引求交后只剩 1 条候选");
    assert_eq!(only_json.stats.nodes_total, 2);
    assert_eq!(only_json.matches.len(), 1);
    assert_eq!(only_json.matches[0].capability.price_per_unit, Credits(4));

    let only_text = graph.query(
        &CapabilityQuery::new(skill("translate.en-zh")).with_input_format(format("text/plain")),
        1,
    );
    assert_eq!(only_text.matches[0].capability.price_per_unit, Credits(3));

    let absent = graph.query(
        &CapabilityQuery::new(skill("translate.en-zh")).with_input_format(format("audio/wav")),
        1,
    );
    assert_eq!(absent.stats.candidates, 0);
    assert!(absent.is_empty());
}

#[test]
fn the_index_follows_evictions_expiry_and_invalidation() {
    let owner = AgentKeys::from_seed(&[230; 32]);
    let mut graph = AgentCapabilityGraph::new(
        owner.did(),
        CapGraphConfig {
            neighbor_capacity: 2,
            cache_ttl_ticks: 10,
            ..CapGraphConfig::default()
        },
    );
    let peers: Vec<AgentKeys> = [231u8, 232, 233]
        .iter()
        .map(|s| AgentKeys::from_seed(&[*s; 32]))
        .collect();
    for (i, peer) in peers.iter().enumerate() {
        let signed = Declaration::new(peer.did(), 1, 0, vec![cap("translate.en-zh", 3 + i as i64)])
            .expect("coherent")
            .sign(peer)
            .expect("signed");
        assert!(graph.apply(&signed, 1).is_applied());
        assert!(graph.index_consistent(), "每次写入后索引都必须自洽");
    }
    assert_eq!(graph.neighbor_count(), 2, "容量 2：第三个邻居挤掉第一个");
    assert_eq!(graph.index_summary()["entries"].as_u64(), Some(2));
    assert!(graph.query(&CapabilityQuery::new(skill("translate.en-zh")), 1).stats.candidates == 2);

    // 过期清理 → 索引同步清空。
    assert_eq!(graph.expire_neighbors(1_000).len(), 2);
    assert!(graph.index_consistent());
    assert!(graph.query(&CapabilityQuery::new(skill("translate.en-zh")), 1).is_empty());

    // 显式失效 → 索引同步。
    let signed = Declaration::new(peers[2].did(), 2, 0, vec![cap("sentiment.analyze", 1)])
        .expect("coherent")
        .sign(&peers[2])
        .expect("signed");
    graph.apply(&signed, 1_001);
    assert!(graph.invalidate_neighbor(&peers[2].did()));
    assert!(graph.index_consistent());
    assert_eq!(graph.index_summary()["entries"].as_u64(), Some(0));
}

#[test]
fn owners_own_capabilities_are_indexed_too() {
    let owner = AgentKeys::from_seed(&[240; 32]);
    let mut graph = AgentCapabilityGraph::new(owner.did(), CapGraphConfig::default());
    let signed = Declaration::new(
        owner.did(),
        1,
        0,
        vec![cap("summarize.zh", 5), cap("translate.zh-en", 6)],
    )
    .expect("coherent")
    .sign(&owner)
    .expect("signed");
    assert!(graph.apply(&signed, 0).is_applied());
    assert!(graph.index_consistent());
    let result = graph.query(&CapabilityQuery::new(skill("summarize.zh")), 0);
    assert_eq!(result.matches.len(), 1);
    assert_eq!(result.matches[0].did, owner.did());
    assert_eq!(graph.providers_of(&skill("translate.zh-en")), vec![owner.did()]);
}

#[test]
fn providers_and_best_for_are_index_backed() {
    let mut graph = big_graph();
    let providers = graph.providers_of(&skill("sentiment.analyze"));
    assert_eq!(providers.len(), 100, "100 个提供者，去重后仍是 100");
    let best = graph.best_for(&skill("sentiment.analyze"), 1);
    assert_eq!(best.matches.len(), 1);
    assert_eq!(best.stats.candidates, 100);
    assert!(graph.best_for(&skill("nothing.here"), 1).is_empty());
}

#[test]
fn scenario_reports_index_backed_queries() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let value = au4a_capgraph::scenario(&mut kernel).expect("scenario runs");
    assert_eq!(value["index_consistent"].as_bool(), Some(true));
    let queries = value["queries"].as_array().expect("queries");
    assert_eq!(queries.len(), 2);
    let stats = &queries[0]["result"]["stats"];
    assert_eq!(stats["nodes_total"].as_u64(), Some(5), "alice + 4 邻居各 1 条能力");
    assert_eq!(stats["scanned"].as_u64(), Some(2), "translate 有 2 个提供者（bob 与 dave）");
    assert!(stats["scanned"].as_u64() < stats["nodes_total"].as_u64());
    // 排序第一关键字是负载加权可靠度：bob(9600bp) 胜过更便宜但可靠度更低的 dave(8000bp)。
    // 「便宜但格式不兼容」的取舍是 v1.1.6 路径规划的事，查询只按度量排序。
    let best = queries[0]["best"].as_str().expect("best provider");
    let agents = au4a_capgraph::demo_agents().expect("demo agents");
    assert_eq!(best, agents[1].keys.did().as_str(), "bob 的加权可靠度最高");
}
