//! v1.1.8 集成测试：性能优化（操作计数证据）。
//!
//! 三件事都必须**可证伪**：
//! 1. 正常读写路径从不整表重建索引（`index_rebuilds == 0`），写入次数与入图能力数一致。
//! 2. 有界 top-k 与「全排序后截断」逐位相同，且统计里 `full_sorts == 0`。
//! 3. 查询缓存按「查询指纹 + 图修订号」失效：命中结果与未命中结果必须一致。
//!
//! 这里没有任何 `Instant`：本 crate 禁止读墙钟，所以「快」只能靠数操作来主张，
//! 而数出来的东西必须是**确定性**的（同一负载两次运行得到逐字节相同的计数）。

use au4a_capgraph::{
    index::rank_and_truncate, rank_top_k, AgentCapabilityGraph, CapGraphConfig, Capability,
    CapabilityQuery, Declaration, QueryCache, QueryPerf, QueryResult, SkillId,
};
use au4a_core::{AgentKeys, Credits, Did};
use au4a_kernel::{Kernel, KernelConfig};

fn skill(name: &str) -> SkillId {
    SkillId::new(name).expect("valid skill")
}

fn cap(price: i64) -> Capability {
    Capability::new(skill("translate.en-zh"), Credits(price))
}

/// 建一个 `agents` 个邻居、每个 2 条能力的图。
fn graph_with(agents: u8, capacity: usize) -> (AgentCapabilityGraph, Vec<Did>) {
    let owner = AgentKeys::from_seed(&[240; 32]);
    let mut graph = AgentCapabilityGraph::new(
        owner.did(),
        CapGraphConfig {
            neighbor_capacity: capacity,
            cache_ttl_ticks: 0,
            ..CapGraphConfig::default()
        },
    );
    let mut dids = Vec::new();
    for seed in 0..agents {
        let peer = AgentKeys::from_seed(&[seed; 32]);
        let caps = vec![
            cap(3).with_reliability_bp(9_000 - u16::from(seed)),
            Capability::new(skill("sentiment.analyze"), Credits(2)),
        ];
        let signed = Declaration::new(peer.did(), 1, 1, caps)
            .expect("coherent")
            .sign(&peer)
            .expect("signed");
        assert!(graph.apply(&signed, 1).is_applied());
        dids.push(peer.did());
    }
    (graph, dids)
}

#[test]
fn index_updates_are_incremental_not_full_rebuilds() {
    let (graph, _) = graph_with(80, 128);
    let perf = graph.query_perf();
    assert_eq!(perf.index_rebuilds, 0, "正常路径不得整表重建");
    assert!(graph.index_consistent());
    let index = graph.index_summary();
    assert_eq!(index["entries"].as_u64(), Some(160), "80 个邻居 × 2 条能力");
    assert_eq!(index["agents"].as_u64(), Some(80));
    assert_eq!(index["skills"].as_u64(), Some(2));
    // 写入次数 == 入图能力数：每次更新只碰「受影响的 Agent」。
    assert_eq!(index["writes"].as_u64(), Some(160));
}

#[test]
fn a_full_neighbor_cache_keeps_the_index_in_step() {
    let (graph, dids) = graph_with(10, 4);
    assert_eq!(graph.neighbor_count(), 4, "容量上限是硬的");
    assert!(graph.index_consistent(), "淘汰后索引必须同步");
    let index = graph.index_summary();
    assert_eq!(index["entries"].as_u64(), Some(8), "4 个邻居 × 2 条能力");
    // 被淘汰的邻居的键必须消失。
    let surviving: Vec<Did> = graph.neighbors().map(|r| r.did.clone()).collect();
    for did in &dids {
        let indexed = graph.providers_of(&skill("translate.en-zh")).contains(did);
        assert_eq!(
            indexed,
            surviving.contains(did),
            "索引与邻居集合不一致：{did}"
        );
    }
    assert_eq!(graph.query_perf().index_rebuilds, 0);
}

#[test]
fn bounded_top_k_is_identical_to_full_sort_and_does_not_full_sort() {
    let (mut graph, _) = graph_with(64, 128);
    let query = CapabilityQuery::new(skill("translate.en-zh"));
    let bounded = graph.query(&query.clone().with_limit(3), 1);
    let perf_after_bounded = graph.query_perf();
    assert!(
        bounded.stats.bounded_selection,
        "limit < 命中数时应走有界选择"
    );
    assert_eq!(perf_after_bounded.bounded_selections, 1);
    assert_eq!(perf_after_bounded.full_sorts, 0, "有界选择不做全排序");

    let full = graph.query(&query.with_limit(0), 1);
    assert!(!full.stats.bounded_selection);
    let expected: Vec<&Did> = full.matches.iter().take(3).map(|m| &m.did).collect();
    let actual: Vec<&Did> = bounded.matches.iter().map(|m| &m.did).collect();
    assert_eq!(actual, expected, "优化不得改变语义");

    // 直接比较两个实现（rank_top_k vs rank_and_truncate）。
    let mut perf = QueryPerf::default();
    let bounded_direct = rank_top_k(full.matches.clone(), 3, &mut perf);
    let reference = rank_and_truncate(full.matches.clone(), 3);
    assert_eq!(
        bounded_direct
            .iter()
            .map(|m| m.did.as_str())
            .collect::<Vec<_>>(),
        reference.iter().map(|m| m.did.as_str()).collect::<Vec<_>>()
    );
    assert_eq!(perf.bounded_selections, 1);
    assert_eq!(perf.full_sorts, 0);
}

#[test]
fn query_cache_hits_and_invalidates_on_every_mutation() {
    let (mut graph, _) = graph_with(8, 128);
    let query = CapabilityQuery::new(skill("translate.en-zh")).with_limit(2);
    let first = graph.query_cached(&query, 1);
    let revision_after_first = graph.revision();
    let second = graph.query_cached(&query, 1);
    assert_eq!(first, second, "命中缓存的结果必须与首次一致");
    assert_eq!(graph.query_cache_stats().hits, 1);
    assert_eq!(graph.query_cache_stats().misses, 1);
    assert_eq!(
        graph.revision(),
        revision_after_first,
        "只读查询不推进修订号"
    );

    // 任何一次数据变更都推进修订号 → 旧键自然失效。
    let owner = AgentKeys::from_seed(&[241; 32]);
    let mut graph2 = graph.clone();
    graph2
        .declare(&owner, vec![cap(9)], 2)
        .expect_err("非 owner 不能声明");
    let peer = AgentKeys::from_seed(&[250; 32]);
    // 故意给新邻居很低的可靠度：它不该挤进前 2 名，否则「结果不变」就没有意义。
    let added = Declaration::new(peer.did(), 1, 2, vec![cap(7).with_reliability_bp(1_000)])
        .expect("coherent")
        .sign(&peer)
        .expect("signed");
    assert!(graph.apply(&added, 2).is_applied());
    assert!(graph.revision() > revision_after_first);
    let third = graph.query_cached(&query, 2);
    assert_eq!(third.matches.len(), first.matches.len(), "limit 仍为 2");
    assert!(
        third.matches.iter().all(|m| m.did != peer.did()),
        "低可靠度新邻居不该进前 2"
    );
    assert_eq!(graph.query_cache_stats().invalidated, 1, "旧条目被作废");
}

#[test]
fn a_disabled_query_cache_still_returns_correct_results() {
    let owner = AgentKeys::from_seed(&[242; 32]);
    let peer = AgentKeys::from_seed(&[243; 32]);
    let mut graph = AgentCapabilityGraph::new(
        owner.did(),
        CapGraphConfig {
            query_cache_capacity: 0,
            ..CapGraphConfig::default()
        },
    );
    let signed = Declaration::new(peer.did(), 1, 1, vec![cap(3)])
        .expect("coherent")
        .sign(&peer)
        .expect("signed");
    graph.apply(&signed, 1);
    let query = CapabilityQuery::new(skill("translate.en-zh"));
    let direct = graph.query(&query, 1);
    let cached = graph.query_cached(&query, 1);
    assert_eq!(direct.matches, cached.matches);
    assert_eq!(graph.query_cache_stats().hits, 0);
    assert_eq!(graph.query_cache_capacity(), 0);
}

#[test]
fn reindex_is_explicit_and_counted() {
    let (mut graph, _) = graph_with(6, 128);
    assert_eq!(graph.query_perf().index_rebuilds, 0);
    assert!(graph.index_consistent());
    graph.reindex();
    assert_eq!(graph.query_perf().index_rebuilds, 1, "显式重建才计数");
    assert!(graph.index_consistent(), "重建后索引仍然自洽");
    assert_eq!(graph.index_summary()["entries"].as_u64(), Some(12));
    // 重建后查询结果不变。
    let query = CapabilityQuery::new(skill("translate.en-zh")).with_limit(0);
    assert_eq!(graph.query(&query, 1).matches.len(), 6);
}

#[test]
fn perf_evidence_is_deterministic_across_identical_workloads() {
    let workload = || -> String {
        let (mut graph, _) = graph_with(16, 128);
        let query = CapabilityQuery::new(skill("translate.en-zh")).with_limit(4);
        for _ in 0..3 {
            let _ = graph.query_cached(&query, 1);
        }
        let _ = graph.query(&CapabilityQuery::new(skill("sentiment.analyze")), 1);
        graph.perf_summary().to_string()
    };
    assert_eq!(workload(), workload(), "证据必须与时间无关、可重放");
}

#[test]
fn the_cache_module_is_bounded_and_revision_aware() {
    let mut cache = QueryCache::new(2);
    let result = QueryResult::empty(9);
    cache.insert("k1".to_string(), 1, result.clone());
    cache.insert("k2".to_string(), 1, result.clone());
    assert!(cache.get("k1", 1).is_some());
    cache.insert("k3".to_string(), 1, result.clone());
    assert_eq!(cache.len(), 2);
    assert_eq!(cache.stats().evictions, 1);
    assert!(cache.get("k2", 1).is_none(), "最久未用被淘汰");
    // 修订号不匹配 → 作废。
    assert!(cache.get("k1", 2).is_none());
    assert_eq!(cache.stats().invalidated, 1);
}

#[test]
fn scenario_reports_operation_counts_and_a_cache_hit() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let value = au4a_capgraph::scenario(&mut kernel).expect("scenario runs");
    assert_eq!(value["query_cache_hit"].as_bool(), Some(true));
    let perf = &value["perf"];
    assert_eq!(
        perf["queries"]["index_rebuilds"].as_u64(),
        Some(0),
        "正常路径不整表重建"
    );
    assert!(
        perf["queries"]["full_sorts"].as_u64().unwrap_or(0) >= 1,
        "规划内部使用 limit=0（要全部候选），走全排序——诚实计数"
    );
    assert!(
        perf["queries"]["bounded_selections"].as_u64().unwrap_or(0) >= 1,
        "limit=1 的查询在 2 个候选中取前 1，走有界选择"
    );
    assert!(
        perf["queries"]["queries"].as_u64().unwrap_or(0) >= 5,
        "查询次数：translate(命中缓存不计) + best + sentiment + 两次规划"
    );
    assert_eq!(
        perf["index"]["entries"].as_u64(),
        Some(5),
        "5 个 Agent 各 1 条能力"
    );
    assert!(
        perf["query_cache"]["misses"].as_u64().unwrap_or(0) >= 1,
        "第一次查询必然未命中"
    );
    assert!(
        perf["query_cache"]["entries"].as_u64().unwrap_or(0) >= 1,
        "缓存里应有条目"
    );
    assert!(perf["revision"].as_u64().is_some());
}
