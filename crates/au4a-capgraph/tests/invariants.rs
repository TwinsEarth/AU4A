//! v1.1.9 不变式测试：在一串**确定性伪随机**操作序列上，断言这张图永远成立的性质。
//!
//! 用的是自带的线性同余发生器（不引入外部 crate，也不用墙钟）：
//! 种子固定 → 序列固定 → 失败可复现。这里没有任何「大概」「通常」，
//! 每条不变式都是「任意操作序列之后都必须为真」。
//!
//! 覆盖的不变式：
//! I1 邻居缓存容量上限；I2 版本历史容量上限；
//! I3 索引与图一致（`index_consistent`）；
//! I4 每个 Agent 的版本号在历史里单调不减；
//! I5 查询候选 ⊆ 索引候选，且 `scanned <= nodes_total`；
//! I6 规划结论要么可被独立校验，要么给出带拒绝码的明确无路径；
//! I7 自有能力与自身声明永远一致（`own_epoch` 与历史末条一致）。

use au4a_capgraph::{
    verify_pipeline, AgentCapabilityGraph, CapGraphConfig, Capability, CapabilityQuery,
    Declaration, FormatId, PipelineRequest, PipelineStep, PlanCost, PlanOutcome, SkillId,
    HISTORY_CAPACITY,
};
use au4a_core::{AgentKeys, CoreError, Credits};

/// 确定性线性同余发生器（数值取自 Numerical Recipes，只用于测试）。
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1))
    }

    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            0
        } else {
            self.next() % bound
        }
    }
}

fn skill(name: &str) -> SkillId {
    SkillId::new(name).expect("valid skill")
}

fn format(name: &str) -> FormatId {
    FormatId::new(name).expect("valid format")
}

/// 一个「世界」：一个 owner + 若干邻居密钥。
struct World {
    owner: AgentKeys,
    peers: Vec<AgentKeys>,
    graph: AgentCapabilityGraph,
}

impl World {
    fn new(seed: u64, peers: u8, capacity: usize) -> Self {
        let mut lcg = Lcg::new(seed);
        let owner = AgentKeys::from_seed(&[(lcg.below(200) as u8); 32]);
        let peers = (0..peers)
            .map(|_| AgentKeys::from_seed(&[(lcg.below(200) as u8); 32]))
            .collect();
        let graph = AgentCapabilityGraph::new(
            owner.did(),
            CapGraphConfig {
                neighbor_capacity: capacity,
                cache_ttl_ticks: 8,
                query_cache_capacity: 4,
                ..CapGraphConfig::default()
            },
        );
        Self {
            owner,
            peers,
            graph,
        }
    }

    fn capability(&self, lcg: &mut Lcg) -> Result<Capability, CoreError> {
        let skills = ["translate.en-zh", "sentiment.analyze", "summarize.zh"];
        let formats = [
            (["text/plain"], ["text/plain"]),
            (["text/plain"], ["application/json"]),
            (["application/json"], ["application/json"]),
        ];
        let pick = lcg.below(skills.len() as u64) as usize;
        let (inputs, outputs) = formats[lcg.below(formats.len() as u64) as usize];
        Capability::new(skill(skills[pick]), Credits(lcg.below(9) as i64 + 1))
            .with_latency(lcg.below(400) as u32 + 1, lcg.below(900) as u32 + 500)
            .with_load_bp(lcg.below(10_001) as u16)
            .with_reliability_bp(lcg.below(10_001) as u16)
            .with_formats(&inputs, &outputs)
    }
}

fn assert_invariants(world: &World, step: usize) {
    let graph = &world.graph;
    // I1 容量上限。
    assert!(
        graph.neighbor_count() <= graph.cache_capacity(),
        "步骤 {step}：邻居数 {} 超过容量 {}",
        graph.neighbor_count(),
        graph.cache_capacity()
    );
    // I2 历史上限。
    assert!(
        graph.history().len() <= HISTORY_CAPACITY,
        "步骤 {step}：版本历史超过上限"
    );
    // I3 索引一致。
    assert!(graph.index_consistent(), "步骤 {step}：索引与图不一致");
    // I4 版本单调（同一 Agent 的历史版本号非递减）。
    for record in graph.neighbors() {
        let versions: Vec<u64> = graph
            .history()
            .of(&record.did)
            .iter()
            .map(|r| r.version)
            .collect();
        for pair in versions.windows(2) {
            assert!(
                pair[0] <= pair[1],
                "步骤 {step}：{} 的版本历史回退 {:?}",
                record.did,
                versions
            );
        }
    }
    // I7 自有能力与历史末条一致。
    if let Some(last) = graph.history().of(graph.owner()).last() {
        assert_eq!(
            last.version,
            graph.own_epoch(),
            "步骤 {step}：own_epoch 与历史末条不一致"
        );
    }
}

#[test]
fn invariants_hold_across_a_deterministic_random_workload() {
    for seed in 1..=8u64 {
        let mut lcg = Lcg::new(seed);
        let mut world = World::new(seed, 12, 6);
        for step in 0..60usize {
            let choice = lcg.below(6);
            match choice {
                0 => {
                    // 自有声明（内容随机变化）。
                    let mut capabilities = Vec::new();
                    for _ in 0..(lcg.below(3) + 1) {
                        capabilities.push(world.capability(&mut lcg).expect("valid capability"));
                    }
                    // 同一技能不能重复：按技能去重，保留第一条。
                    capabilities.sort_by(|a, b| a.skill.cmp(&b.skill));
                    capabilities.dedup_by(|a, b| a.skill == b.skill);
                    let _ = world.graph.declare(&world.owner, capabilities, step as u64);
                }
                1 => {
                    // 邻居声明（随机 epoch + 随机内容）。
                    let peer = &world.peers[lcg.below(world.peers.len() as u64) as usize];
                    let epoch = lcg.below(6) + 1;
                    let mut capabilities = Vec::new();
                    for _ in 0..(lcg.below(2) + 1) {
                        capabilities.push(world.capability(&mut lcg).expect("valid capability"));
                    }
                    capabilities.sort_by(|a, b| a.skill.cmp(&b.skill));
                    capabilities.dedup_by(|a, b| a.skill == b.skill);
                    if let Ok(declaration) =
                        Declaration::new(peer.did(), epoch, step as u64, capabilities)
                    {
                        if let Ok(signed) = declaration.sign(peer) {
                            let _ = world.graph.apply(&signed, step as u64);
                        }
                    }
                }
                2 => {
                    let _ = world.graph.expire_neighbors(step as u64 * 3);
                }
                3 => {
                    let peer = &world.peers[lcg.below(world.peers.len() as u64) as usize];
                    let _ = world.graph.invalidate_neighbor(&peer.did());
                }
                4 | 5 => {
                    let mut query = CapabilityQuery::new(skill("translate.en-zh"))
                        .with_limit(lcg.below(4) as usize);
                    if lcg.below(2) == 0 {
                        query = query.with_max_price(Credits(lcg.below(5) as i64 + 1));
                    }
                    let result = world.graph.query(&query, step as u64);
                    // I5：候选 ≤ 全图能力数，扫描数 == 候选数，命中 ≤ 扫描数。
                    assert!(result.stats.candidates <= result.stats.nodes_total);
                    assert_eq!(result.stats.scanned, result.stats.candidates);
                    assert!(result.stats.matched <= result.stats.scanned);
                    if query.limit > 0 {
                        assert!(result.matches.len() <= query.limit);
                    }
                }
                _ => unreachable!(),
            }
            assert_invariants(&world, step);
        }
        // 收尾：显式重建索引后仍然自洽。
        world.graph.reindex();
        assert!(world.graph.index_consistent());
    }
}

#[test]
fn a_planned_pipeline_is_always_independently_verifiable() {
    // 构造一个多提供者的图：每条技能 3 个提供者，格式各异。
    let owner = AgentKeys::from_seed(&[170; 32]);
    let mut graph = AgentCapabilityGraph::new(owner.did(), CapGraphConfig::default());
    let shapes: [(u8, &str, i64, [&str; 1], [&str; 1]); 6] = [
        (
            1,
            "translate.en-zh",
            3,
            ["text/plain"],
            ["application/json"],
        ),
        (2, "translate.en-zh", 1, ["text/plain"], ["text/html"]),
        (
            3,
            "translate.en-zh",
            5,
            ["application/json"],
            ["application/json"],
        ),
        (
            4,
            "sentiment.analyze",
            2,
            ["application/json"],
            ["application/json"],
        ),
        (
            5,
            "sentiment.analyze",
            1,
            ["text/plain"],
            ["application/json"],
        ),
        (
            6,
            "sentiment.analyze",
            4,
            ["application/json"],
            ["text/plain"],
        ),
    ];
    for (seed, name, price, inputs, outputs) in shapes {
        let keys = AgentKeys::from_seed(&[seed; 32]);
        let capability = Capability::new(skill(name), Credits(price))
            .with_formats(&inputs, &outputs)
            .expect("valid formats")
            .with_latency(50 * u32::from(seed), 200 * u32::from(seed));
        let signed = Declaration::new(keys.did(), 1, 0, vec![capability])
            .expect("coherent")
            .sign(&keys)
            .expect("signed");
        assert!(graph.apply(&signed, 0).is_applied());
    }

    let requests = [
        PipelineRequest::new(
            format("text/plain"),
            vec![
                PipelineStep::new(skill("translate.en-zh")),
                PipelineStep::new(skill("sentiment.analyze")),
            ],
        ),
        PipelineRequest::new(
            format("text/plain"),
            vec![PipelineStep::new(skill("sentiment.analyze"))],
        )
        .with_final_output(format("application/json")),
        PipelineRequest::new(
            format("application/json"),
            vec![
                PipelineStep::new(skill("translate.en-zh")),
                PipelineStep::new(skill("sentiment.analyze")),
            ],
        )
        .with_budget(Credits(6)),
        PipelineRequest::new(
            format("text/plain"),
            vec![
                PipelineStep::new(skill("sentiment.analyze")),
                PipelineStep::new(skill("translate.en-zh")),
            ],
        ),
    ];

    let mut planned = 0;
    for request in &requests {
        match graph.plan(request, &PlanCost::default(), 0) {
            PlanOutcome::Path(pipeline) => {
                planned += 1;
                // I6：任何被返回的流水线都必须能被独立校验器验通。
                verify_pipeline(&graph, request, &PlanCost::default(), &pipeline).unwrap_or_else(
                    |err| panic!("校验失败：{err}（请求 {:?}）", request.to_value()),
                );
                if let Some(budget) = request.max_total_price {
                    assert!(pipeline.total_price <= budget);
                }
                if let Some(deadline) = request.deadline_ms {
                    assert!(pipeline.total_latency_ms <= u64::from(deadline));
                }
                for node in &pipeline.nodes {
                    assert!(node.cost > 0, "代价必须为正");
                }
            }
            PlanOutcome::NoPath(no_path) => {
                assert!(
                    !no_path.reason.detail().is_empty(),
                    "无路径必须给出具体原因"
                );
            }
        }
    }
    assert!(planned >= 3, "至少三条请求应当可行");
}

#[test]
fn no_path_and_relaxed_path_are_consistent() {
    let owner = AgentKeys::from_seed(&[180; 32]);
    let mut graph = AgentCapabilityGraph::new(owner.did(), CapGraphConfig::default());
    for (seed, name, price, inputs, outputs) in [
        (
            181u8,
            "translate.en-zh",
            3i64,
            ["text/plain"],
            ["application/json"],
        ),
        (
            182,
            "sentiment.analyze",
            2,
            ["application/json"],
            ["application/json"],
        ),
    ] {
        let keys = AgentKeys::from_seed(&[seed; 32]);
        let capability = Capability::new(skill(name), Credits(price))
            .with_formats(&inputs, &outputs)
            .expect("valid formats")
            .with_latency(100, 300);
        let signed = Declaration::new(keys.did(), 1, 0, vec![capability])
            .expect("coherent")
            .sign(&keys)
            .expect("signed");
        graph.apply(&signed, 0);
    }
    let base = PipelineRequest::new(
        format("text/plain"),
        vec![
            PipelineStep::new(skill("translate.en-zh")),
            PipelineStep::new(skill("sentiment.analyze")),
        ],
    );
    // 无约束 → 有路径。
    assert!(graph.plan(&base, &PlanCost::default(), 0).is_path());
    // 预算 1 → 无路径，且放宽后确实有路径（说明是预算剪掉的，不是格式不通）。
    let tight = base.clone().with_budget(Credits(1));
    let outcome = graph.plan(&tight, &PlanCost::default(), 0);
    assert!(!outcome.is_path());
    assert_eq!(
        outcome.refusal_code(),
        Some(au4a_core::RefusalCode::PolicyDenied),
        "预算问题属于策略拒绝"
    );
    assert!(
        graph.plan(&base, &PlanCost::default(), 0).is_path(),
        "放宽后仍有路径"
    );

    // 期限 1ms → 无路径，拒绝码是 timeout。
    let rushed = base.clone().with_deadline_ms(1);
    assert_eq!(
        graph.plan(&rushed, &PlanCost::default(), 0).refusal_code(),
        Some(au4a_core::RefusalCode::Timeout)
    );
}

#[test]
fn the_version_and_history_stay_bounded_under_long_workloads() {
    let owner = AgentKeys::from_seed(&[190; 32]);
    let mut graph = AgentCapabilityGraph::new(owner.did(), CapGraphConfig::default());
    for round in 0..(HISTORY_CAPACITY + 20) {
        let capability = Capability::new(skill("translate.en-zh"), Credits(round as i64 % 7 + 1))
            .with_load_bp((round % 10_001) as u16);
        let outcome = graph
            .declare(&owner, vec![capability], round as u64)
            .expect("declare");
        assert!(outcome.is_applied());
    }
    assert_eq!(graph.history().len(), HISTORY_CAPACITY, "有界记忆");
    assert_eq!(graph.own_epoch(), (HISTORY_CAPACITY + 20) as u64);
    assert!(graph.index_consistent());
    assert_eq!(graph.query_perf().index_rebuilds, 0, "长负载也不整表重建");
}

#[test]
fn an_agent_can_only_be_known_through_its_own_signature() {
    let owner = AgentKeys::from_seed(&[200; 32]);
    let victim = AgentKeys::from_seed(&[201; 32]);
    let attacker = AgentKeys::from_seed(&[202; 32]);
    let mut graph = AgentCapabilityGraph::new(owner.did(), CapGraphConfig::default());

    // 1) 攻击者想替受害者声明：签名这一步就失败。
    let forged = Declaration::new(
        victim.did(),
        1,
        0,
        vec![Capability::new(skill("translate.en-zh"), Credits(1))],
    )
    .expect("coherent");
    assert_eq!(forged.sign(&attacker), Err(CoreError::InvalidSignature));

    // 2) 攻击者把受害者写成声明的 agent、塞一个假签名 → 必须判 unauthorized，
    //    而且图里不会因此留下任何关于受害者的记录。
    let payload = serde_json::json!({
        "declaration": {
            "agent": victim.did().as_str(),
            "epoch": 1,
            "issued_at": 0,
            "capabilities": [],
        },
        "sig": "00".repeat(64),
    });
    let smuggled = au4a_capgraph::SignedDeclaration::from_value(&payload).expect("parses");
    let outcome = graph.apply(&smuggled, 0);
    assert_eq!(
        outcome.refusal(),
        Some(au4a_core::RefusalCode::Unauthorized)
    );
    assert!(outcome.refusal().expect("code").is_misconduct());
    assert_eq!(graph.capability_count(), 0);
    assert_eq!(
        graph.last_version_of(&victim.did()),
        None,
        "图里没有任何关于受害者的记录"
    );
    assert_eq!(graph.neighbor_count(), 0);
}
