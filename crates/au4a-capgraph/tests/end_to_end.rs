//! v1.1.9 端到端测试：五个 Agent 声明 → 广播 → 索引查询 → 路径规划 → 独立校验。
//!
//! 这一版把整条链路串起来，并回答三个「只有端到端才问得出」的问题：
//! 1. 同样的输入能不能给出**逐字节相同**的结论（可重放）？
//! 2. 规划出的流水线，**换一个 Agent 的图**能不能独立验通（不靠信任提出方）？
//! 3. 被篡改/陈旧的输入，在链路末端是否一定表现为**有类型的拒绝**？

use au4a_capgraph::{
    announce, demo_agents, ingest, pump, verify_pipeline, AgentCapabilityGraph, CapGraphConfig,
    Capability, CapabilityQuery, Declaration, FormatId, PipelineRequest, PipelineStep, PlanCost,
    PlanOutcome, SignedDeclaration, SkillId,
};
use au4a_core::{AgentKeys, Credits, RefusalCode};
use au4a_kernel::{Kernel, KernelConfig};

fn skill(name: &str) -> SkillId {
    SkillId::new(name).expect("valid skill")
}

fn format(name: &str) -> FormatId {
    FormatId::new(name).expect("valid format")
}

/// 跑一遍完整的「声明 → 广播 → 收下 → 查询 → 规划」链路。
fn run_pipeline_scenario() -> (
    AgentCapabilityGraph,
    PlanOutcome,
    PipelineRequest,
    Vec<au4a_core::Envelope>,
) {
    let agents = demo_agents().expect("demo agents");
    let mut kernel = Kernel::new(KernelConfig::default());
    for agent in &agents {
        kernel
            .register(&agent.keys, agent.display, &[], Credits(20))
            .expect("register");
    }
    let alice = &agents[0];
    let mut graph = AgentCapabilityGraph::new(alice.keys.did(), CapGraphConfig::default());
    graph
        .declare(&alice.keys, alice.capabilities.clone(), 0)
        .expect("own declaration");

    let mut envelopes = Vec::new();
    for agent in &agents[1..] {
        let declaration =
            Declaration::new(agent.keys.did(), 1, 0, agent.capabilities.clone()).expect("coherent");
        let env = announce(&agent.keys, &declaration, 1).expect("announce");
        kernel.send(&env).expect("kernel accepts");
        envelopes.push(env);
    }
    let report = pump(&mut kernel, &mut graph, 1);
    assert_eq!(report.applied, 4, "四个邻居全部入图");

    let request = PipelineRequest::new(
        format("text/plain"),
        vec![
            PipelineStep::new(skill("translate.en-zh")),
            PipelineStep::new(skill("sentiment.analyze")),
        ],
    )
    .with_final_output(format("application/json"));
    let outcome = graph.plan(&request, &PlanCost::default(), 1);
    (graph, outcome, request, envelopes)
}

#[test]
fn the_full_chain_produces_a_verifiable_pipeline() {
    let (graph, outcome, request, _) = run_pipeline_scenario();
    let pipeline = outcome.pipeline().expect("path exists");
    assert_eq!(pipeline.nodes.len(), 2);
    assert_eq!(pipeline.nodes[0].output_format.as_str(), "application/json");
    assert_eq!(pipeline.nodes[1].input_format.as_str(), "application/json");
    assert_eq!(pipeline.total_price, Credits(5));
    // 关键：用同一个图**独立校验**这条流水线。
    verify_pipeline(&graph, &request, &PlanCost::default(), pipeline).expect("流水线自洽");
    assert!(graph.index_consistent());
}

#[test]
fn a_receiver_can_verify_the_plan_without_trusting_the_proposer() {
    let (_, outcome, request, _) = run_pipeline_scenario();
    let pipeline = outcome.pipeline().expect("path").clone();

    // 另一个 Agent 用自己的图（同样的广播数据）独立校验 → 通过。
    let agents = demo_agents().expect("demo agents");
    let observer = AgentKeys::from_seed(&[99; 32]);
    let mut other = AgentCapabilityGraph::new(observer.did(), CapGraphConfig::default());
    for agent in &agents[1..] {
        let signed = Declaration::new(agent.keys.did(), 1, 0, agent.capabilities.clone())
            .expect("coherent")
            .sign(&agent.keys)
            .expect("signed");
        assert!(other.apply(&signed, 0).is_applied());
    }
    verify_pipeline(&other, &request, &PlanCost::default(), &pipeline).expect("独立图也验得通");

    // 被改过的方案必须验不过（价格被改小）。
    let mut tampered = pipeline.clone();
    tampered.nodes[0].price_per_unit = Credits(1);
    assert!(verify_pipeline(&other, &request, &PlanCost::default(), &tampered).is_err());

    // 被改过格式链的方案必须验不过。
    let mut broken = pipeline.clone();
    broken.nodes[1].input_format = format("text/html");
    assert!(verify_pipeline(&other, &request, &PlanCost::default(), &broken).is_err());

    // 步数与请求不符也必须验不过。
    let mut short = pipeline.clone();
    short.nodes.pop();
    assert!(verify_pipeline(&other, &request, &PlanCost::default(), &short).is_err());

    // 图里不存在的能力（伪造 DID）也必须验不过。
    let mut forged = pipeline;
    forged.nodes[0].did = observer.did();
    assert!(verify_pipeline(&other, &request, &PlanCost::default(), &forged).is_err());
}

#[test]
fn the_whole_chain_is_replayable_byte_for_byte() {
    let (first_graph, first_outcome, _, _) = run_pipeline_scenario();
    let (second_graph, second_outcome, _, _) = run_pipeline_scenario();
    assert_eq!(first_outcome.to_value(), second_outcome.to_value(), "规划结论可重放");
    assert_eq!(first_graph.to_value(), second_graph.to_value(), "图状态可重放");
    assert_eq!(
        first_graph.version_summary().to_string(),
        second_graph.version_summary().to_string()
    );
    assert_eq!(
        first_graph.perf_summary().to_string(),
        second_graph.perf_summary().to_string(),
        "操作计数也必须可重放"
    );
}

#[test]
fn the_index_backed_queries_agree_with_a_brute_force_scan() {
    let (mut graph, _, _, _) = run_pipeline_scenario();
    for name in ["translate.en-zh", "sentiment.analyze", "summarize.zh", "nothing.here"] {
        let query = CapabilityQuery::new(skill(name)).with_limit(0);
        let indexed = graph.query(&query, 1);
        let mut brute: Vec<String> = Vec::new();
        for record in graph.neighbors() {
            for capability in &record.capabilities {
                if query.matches(capability) {
                    brute.push(record.did.as_str().to_string());
                }
            }
        }
        if graph.capabilities_of(graph.owner()).is_some() {
            for capability in graph.own_capabilities() {
                if query.matches(capability) {
                    brute.push(graph.owner().as_str().to_string());
                }
            }
        }
        brute.sort();
        let mut from_index: Vec<String> = indexed
            .matches
            .iter()
            .map(|m| m.did.as_str().to_string())
            .collect();
        from_index.sort();
        assert_eq!(from_index, brute, "技能 {name} 的索引结果与全扫不一致");
    }
}

#[test]
fn a_stale_broadcast_never_reaches_the_planner() {
    let agents = demo_agents().expect("demo agents");
    let alice = &agents[0];
    let bob = &agents[1];
    let mut graph = AgentCapabilityGraph::new(alice.keys.did(), CapGraphConfig::default());

    // 先收下 bob 的 v2（产出 application/json），再重放他的 v1（产出 text/html 那种更便宜的旧版）。
    let fresh = Declaration::new(bob.keys.did(), 2, 2, bob.capabilities.clone())
        .expect("coherent")
        .sign(&bob.keys)
        .expect("signed");
    assert!(graph.apply(&fresh, 1).is_applied());

    let mut stale_capability = bob.capabilities[0].clone();
    stale_capability = stale_capability.with_price(Credits(1));
    let stale = Declaration::new(bob.keys.did(), 1, 1, vec![stale_capability])
        .expect("coherent")
        .sign(&bob.keys)
        .expect("signed");
    assert_eq!(graph.apply(&stale, 2).refusal(), Some(RefusalCode::StaleEpoch));

    // 图里仍然是 v2 的那份能力（价格 3，不是迟到的 1）。
    assert_eq!(
        graph.capabilities_of(&bob.keys.did()).expect("known")[0].price_per_unit,
        Credits(3)
    );
    // 其余邻居照常入图，规划仍然可用。
    for agent in &agents[2..] {
        let signed = Declaration::new(agent.keys.did(), 1, 0, agent.capabilities.clone())
            .expect("coherent")
            .sign(&agent.keys)
            .expect("signed");
        assert!(graph.apply(&signed, 2).is_applied());
    }
    let request = PipelineRequest::new(
        format("text/plain"),
        vec![
            PipelineStep::new(skill("translate.en-zh")),
            PipelineStep::new(skill("sentiment.analyze")),
        ],
    );
    let pipeline = graph
        .plan(&request, &PlanCost::default(), 2)
        .pipeline()
        .cloned()
        .expect("path");
    assert_eq!(pipeline.total_price, Credits(5), "用的是 v2 的价格，不是迟到 v1 的 1");
}

#[test]
fn a_tampered_broadcast_is_dropped_at_the_edge_of_the_graph() {
    let agents = demo_agents().expect("demo agents");
    let alice = &agents[0];
    let mut graph = AgentCapabilityGraph::new(alice.keys.did(), CapGraphConfig::default());
    let mut env = announce(
        &agents[1].keys,
        &Declaration::new(agents[1].keys.did(), 1, 0, agents[1].capabilities.clone())
            .expect("coherent"),
        1,
    )
    .expect("announce");
    env.body["declaration"]["declaration"]["capabilities"][0]["latency_p99_ms"] =
        serde_json::json!(1);
    let outcome = ingest(&mut graph, &env, 1);
    assert_eq!(outcome.refusal(), Some(RefusalCode::Unauthorized));
    assert!(outcome.refusal().expect("code").is_misconduct());
    assert_eq!(graph.capability_count(), 0, "被篡改的通告绝不入图");
    assert!(graph
        .plan(
            &PipelineRequest::new(
                format("text/plain"),
                vec![PipelineStep::new(skill("translate.en-zh"))]
            ),
            &PlanCost::default(),
            1
        )
        .refusal_code()
        .is_some());
}

#[test]
fn scenario_output_covers_every_version_of_the_track() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let value = au4a_capgraph::scenario(&mut kernel).expect("scenario runs");
    // 逐版检查 scenario 里必须出现的证据字段（少一个就说明某一版的增量掉了）。
    assert!(value["graph"]["owner"].is_string(), "v1.1.1/v1.1.2 图投影");
    assert_eq!(value["announcements_sent"].as_u64(), Some(4), "v1.1.3 广播");
    assert_eq!(value["bounded_cache"]["evictions"].as_u64(), Some(2), "v1.1.4 缓存");
    assert_eq!(value["index_consistent"].as_bool(), Some(true), "v1.1.5 索引");
    assert_eq!(value["plan"]["outcome"].as_str(), Some("path"), "v1.1.6 规划");
    assert_eq!(value["versions"]["own"].as_u64(), Some(2), "v1.1.7 版本化");
    assert_eq!(value["perf"]["queries"]["index_rebuilds"].as_u64(), Some(0), "v1.1.8 性能");
    assert_eq!(value["query_cache_hit"].as_bool(), Some(true), "v1.1.8 缓存命中");
    // v1.1.9：整份输出必须能被序列化（可被节点聚合）。
    let text = serde_json::to_string(&value).expect("serialisable");
    assert!(text.len() > 500);
}

#[test]
fn a_declaration_survives_a_json_round_trip_inside_the_chain() {
    let agents = demo_agents().expect("demo agents");
    let bob = &agents[1];
    let signed = Declaration::new(bob.keys.did(), 3, 7, bob.capabilities.clone())
        .expect("coherent")
        .sign(&bob.keys)
        .expect("signed");
    let value = signed.to_value().expect("serialisable");
    let restored: SignedDeclaration = SignedDeclaration::from_value(&value).expect("parses");
    assert_eq!(restored, signed);
    restored.verify().expect("签名在 JSON 往返后仍然有效");

    // 篡改后的往返版本必须被拒。
    let mut broken = value;
    broken["declaration"]["capabilities"][0]["throughput_per_min"] = serde_json::json!(1);
    let broken = SignedDeclaration::from_value(&broken).expect("parses");
    assert!(broken.verify().is_err());
}

#[test]
fn capabilities_are_stable_values_not_aliases_of_the_graph() {
    let (graph, _, _, _) = run_pipeline_scenario();
    let snapshot: Vec<Capability> = graph
        .neighbors()
        .flat_map(|record| record.capabilities.clone())
        .collect();
    let again: Vec<Capability> = graph
        .neighbors()
        .flat_map(|record| record.capabilities.clone())
        .collect();
    assert_eq!(snapshot, again, "同样的读操作得到同样的值（没有内部可变性泄漏）");
}
