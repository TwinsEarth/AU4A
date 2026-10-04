//! v1.1.6 集成测试：路径规划。
//!
//! 这一版的核心可证伪主张有两条：
//!
//! 1. 规划找到的是**整条链**代价最小的方案，而不是每步各自最便宜 ——
//!    演示数据里存在一个「贪心必死」的反例，测试直接断言第一步没选最便宜的那个。
//! 2. 「无路径」是**搜出来的结论**，带具体到某一步的类型化原因，而不是一句失败。

use au4a_capgraph::{
    demo_agents, AgentCapabilityGraph, CapGraphConfig, Capability, CapabilityQuery, Declaration,
    FormatId, PipelineRequest, PipelineStep, PlanCost, PlanOutcome, NoPathReason, SkillId,
};
use au4a_core::{AgentKeys, Credits, Did, RefusalCode};
use au4a_kernel::{Kernel, KernelConfig};

fn skill(name: &str) -> SkillId {
    SkillId::new(name).expect("valid skill")
}

fn format(name: &str) -> FormatId {
    FormatId::new(name).expect("valid format")
}

fn capability(name: &str, price: i64, inputs: &[&str], outputs: &[&str], latency: (u32, u32)) -> Capability {
    Capability::new(skill(name), Credits(price))
        .with_formats(inputs, outputs)
        .expect("valid formats")
        .with_latency(latency.0, latency.1)
}

/// alice 视角的图：4 个邻居，含两个「便宜但格式不兼容」的诱饵。
fn demo_graph() -> (AgentCapabilityGraph, Vec<Did>) {
    let agents = demo_agents().expect("demo agents");
    let mut graph = AgentCapabilityGraph::new(agents[0].keys.did(), CapGraphConfig::default());
    for agent in &agents {
        let signed = Declaration::new(agent.keys.did(), 1, 0, agent.capabilities.clone())
            .expect("coherent")
            .sign(&agent.keys)
            .expect("signed");
        graph.apply(&signed, 0);
    }
    let dids = agents[1..].iter().map(|a| a.keys.did()).collect();
    (graph, dids)
}

fn translation_then_sentiment() -> PipelineRequest {
    PipelineRequest::new(
        format("text/plain"),
        vec![
            PipelineStep::new(skill("translate.en-zh")),
            PipelineStep::new(skill("sentiment.analyze")),
        ],
    )
}

#[test]
fn a_two_step_pipeline_is_built_out_of_two_different_agents() {
    let (mut graph, dids) = demo_graph();
    let outcome = graph.plan(&translation_then_sentiment(), &PlanCost::default(), 0);
    let pipeline = outcome.pipeline().expect("path exists");
    assert_eq!(pipeline.nodes.len(), 2);
    assert_eq!(pipeline.nodes[0].did, dids[0], "bob 提供 application/json 的翻译");
    assert_eq!(pipeline.nodes[1].did, dids[1], "carol 接受 application/json 的情感分析");
    assert_eq!(pipeline.handoffs(), 1, "两个不同 Agent → 一次换手");
    assert_eq!(pipeline.total_price, Credits(5));
    assert_eq!(pipeline.total_latency_ms, 200, "90 + 110（空载时等于 p50）");
    assert!(pipeline.search.settled >= 2, "搜索确实展开过状态");
    assert_eq!(pipeline.search.candidates, 4);
}

#[test]
fn the_search_is_not_greedy_the_cheapest_first_hop_is_a_dead_end() {
    let (mut graph, dids) = demo_graph();
    let dave = dids[2].clone();
    // 贪心第一步：最便宜的翻译提供者。
    let cheapest = graph
        .query(&CapabilityQuery::new(skill("translate.en-zh")).with_limit(0), 0)
        .matches
        .into_iter()
        .min_by_key(|m| m.capability.price_per_unit.get())
        .expect("provider");
    assert_eq!(cheapest.did, dave);
    assert_eq!(cheapest.capability.price_per_unit, Credits(1));

    // 从它出发没有任何可接续的下一步。
    let dave_capability = cheapest.capability.clone();
    let successors = graph
        .query(&CapabilityQuery::new(skill("sentiment.analyze")).with_limit(0), 0)
        .matches
        .iter()
        .filter(|m| dave_capability.handoff_format(&m.capability).is_some())
        .count();
    assert_eq!(successors, 0);

    // 规划照样给出可行解，且第一步不是它。
    let pipeline = graph
        .plan(&translation_then_sentiment(), &PlanCost::default(), 0)
        .pipeline()
        .expect("search beats greedy")
        .clone();
    assert_ne!(pipeline.nodes[0].did, dave);
    assert_eq!(pipeline.nodes[0].price_per_unit, Credits(3), "为此多付 2 微积分是值得的");
}

#[test]
fn format_compatibility_is_checked_on_every_handoff() {
    let (mut graph, _) = demo_graph();
    // 强制要求中间格式是 application/json：只有 bob 能产出、只有 carol 能接受。
    let request = PipelineRequest::new(
        format("text/plain"),
        vec![
            PipelineStep::new(skill("translate.en-zh")).expecting(format("application/json")),
            PipelineStep::new(skill("sentiment.analyze")),
        ],
    );
    let pipeline = graph.plan(&request, &PlanCost::default(), 0).pipeline().cloned();
    assert!(pipeline.is_some());
    let pipeline = pipeline.expect("path");
    assert_eq!(pipeline.nodes[0].output_format.as_str(), "application/json");
    assert_eq!(pipeline.nodes[1].input_format.as_str(), "application/json");

    // 强制要求 text/html 作为中间格式：dave 能产出，但没有任何情感分析能接受它。
    let request = PipelineRequest::new(
        format("text/plain"),
        vec![
            PipelineStep::new(skill("translate.en-zh")).expecting(format("text/html")),
            PipelineStep::new(skill("sentiment.analyze")),
        ],
    );
    match graph.plan(&request, &PlanCost::default(), 0) {
        PlanOutcome::NoPath(no_path) => {
            assert!(matches!(
                no_path.reason,
                NoPathReason::NoCompatibleFormat { step: 1, .. }
            ));
            assert_eq!(no_path.refusal_code(), RefusalCode::Unsupported);
            assert!(no_path.search.settled > 0, "失败也是搜出来的");
        }
        PlanOutcome::Path(pipeline) => panic!("text/html 不该接得上：{:?}", pipeline.to_value()),
    }
}

#[test]
fn budget_and_deadline_are_independent_constraints() {
    let (mut graph, _) = demo_graph();
    let base = translation_then_sentiment();
    assert!(graph.plan(&base, &PlanCost::default(), 0).is_path());
    match graph.plan(&base.clone().with_budget(Credits(4)), &PlanCost::default(), 0) {
        PlanOutcome::NoPath(no_path) => {
            assert_eq!(no_path.reason, NoPathReason::BudgetExceeded { allowed: 4 });
            assert_eq!(no_path.refusal_code(), RefusalCode::PolicyDenied);
        }
        PlanOutcome::Path(p) => panic!("{:?}", p.to_value()),
    }
    match graph.plan(&base.clone().with_deadline_ms(199), &PlanCost::default(), 0) {
        PlanOutcome::NoPath(no_path) => {
            assert_eq!(no_path.reason, NoPathReason::DeadlineExceeded { allowed_ms: 199 });
            assert_eq!(no_path.refusal_code(), RefusalCode::Timeout);
        }
        PlanOutcome::Path(p) => panic!("{:?}", p.to_value()),
    }
    // 恰好够：5 微积分 / 200ms。
    let exact = base.clone().with_budget(Credits(5)).with_deadline_ms(200);
    let pipeline = graph.plan(&exact, &PlanCost::default(), 0).pipeline().cloned();
    assert!(pipeline.is_some());
    assert_eq!(pipeline.expect("pipeline").total_price, Credits(5));
}

#[test]
fn hard_constraints_remove_candidates_and_are_named() {
    let (mut graph, _) = demo_graph();
    let base = translation_then_sentiment();
    // 可靠度下限 9999：没有任何演示能力满足。
    match graph.plan(&base.clone().with_min_reliability_bp(9_999), &PlanCost::default(), 0) {
        PlanOutcome::NoPath(no_path) => {
            assert!(matches!(
                no_path.reason,
                NoPathReason::ConstraintRejected { step: 0, .. }
            ));
            assert_eq!(no_path.refusal_code(), RefusalCode::PolicyDenied);
        }
        PlanOutcome::Path(p) => panic!("{:?}", p.to_value()),
    }
    // 载荷超过 1 MiB 的硬上限。
    assert!(!graph
        .plan(&base.clone().with_payload_bytes(u64::MAX), &PlanCost::default(), 0)
        .is_path());
    assert!(graph
        .plan(&base.clone().with_payload_bytes(1_024), &PlanCost::default(), 0)
        .is_path());
    // 区域：白名单为空的能让任何区域通过；显式限定不匹配的区域则被剔除。
    assert!(graph
        .plan(&base.clone().with_region("eu-west"), &PlanCost::default(), 0)
        .is_path());
}

#[test]
fn an_unknown_skill_is_reported_as_a_missing_provider() {
    let (mut graph, _) = demo_graph();
    let request = PipelineRequest::new(
        format("text/plain"),
        vec![PipelineStep::new(skill("speech.transcribe"))],
    );
    match graph.plan(&request, &PlanCost::default(), 0) {
        PlanOutcome::NoPath(no_path) => {
            assert_eq!(
                no_path.reason,
                NoPathReason::NoProvider {
                    step: 0,
                    skill: "speech.transcribe".to_string()
                }
            );
            assert!(no_path.reason.detail().contains("no provider"));
        }
        PlanOutcome::Path(p) => panic!("{:?}", p.to_value()),
    }
}

#[test]
fn a_three_step_pipeline_walks_through_three_agents() {
    let owner = AgentKeys::from_seed(&[80; 32]);
    let mut graph = AgentCapabilityGraph::new(owner.did(), CapGraphConfig::default());
    let chain = [
        (81u8, capability("speech.transcribe", 9, &["audio/wav"], &["text/plain"], (400, 900))),
        (82, capability("translate.en-zh", 3, &["text/plain"], &["application/json"], (90, 240))),
        (83, capability("sentiment.analyze", 2, &["application/json"], &["application/json"], (110, 300))),
        // 诱饵：更便宜的语音转写，但产出 mp3，接不上翻译。
        (84, capability("speech.transcribe", 1, &["audio/wav"], &["audio/mpeg"], (50, 120))),
    ];
    for (seed, cap) in chain {
        let keys = AgentKeys::from_seed(&[seed; 32]);
        let signed = Declaration::new(keys.did(), 1, 0, vec![cap])
            .expect("coherent")
            .sign(&keys)
            .expect("signed");
        assert!(graph.apply(&signed, 0).is_applied());
    }
    let request = PipelineRequest::new(
        format("audio/wav"),
        vec![
            PipelineStep::new(skill("speech.transcribe")),
            PipelineStep::new(skill("translate.en-zh")),
            PipelineStep::new(skill("sentiment.analyze")),
        ],
    )
    .with_final_output(format("application/json"));
    let pipeline = graph.plan(&request, &PlanCost::default(), 0).pipeline().cloned();
    let pipeline = pipeline.expect("three hops are possible");
    assert_eq!(pipeline.nodes.len(), 3);
    assert_eq!(pipeline.nodes[0].price_per_unit, Credits(9), "绕开 1 微积分的死路");
    assert_eq!(pipeline.total_price, Credits(14));
    assert_eq!(pipeline.handoffs(), 2);
    let formats: Vec<String> = pipeline
        .nodes
        .iter()
        .map(|n| n.output_format.as_str().to_string())
        .collect();
    assert_eq!(formats, vec!["text/plain", "application/json", "application/json"]);
}

#[test]
fn planning_is_replayable_across_two_identical_graphs() {
    let (mut first, _) = demo_graph();
    let (mut second, _) = demo_graph();
    let a = first.plan(&translation_then_sentiment(), &PlanCost::default(), 0);
    let b = second.plan(&translation_then_sentiment(), &PlanCost::default(), 0);
    assert_eq!(a.to_value(), b.to_value());
    let request = translation_then_sentiment().with_budget(Credits(1));
    let c = first.plan(&request, &PlanCost::default(), 0);
    let d = second.plan(&request, &PlanCost::default(), 0);
    assert_eq!(c.to_value(), d.to_value(), "无路径的结论也要可重放");
}

#[test]
fn scenario_returns_a_pipeline_and_a_no_path_demo() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let value = au4a_capgraph::scenario(&mut kernel).expect("scenario runs");
    assert_eq!(value["plan"]["outcome"].as_str(), Some("path"));
    let nodes = value["plan"]["pipeline"]["nodes"].as_array().expect("nodes");
    assert_eq!(nodes.len(), 2);
    assert_eq!(value["plan"]["pipeline"]["total_price"].as_u64(), Some(5));
    let agents = demo_agents().expect("demo agents");
    assert_eq!(nodes[0]["agent"].as_str(), Some(agents[1].keys.did().as_str()));
    assert_eq!(nodes[1]["agent"].as_str(), Some(agents[2].keys.did().as_str()));
    assert_eq!(value["no_path_demo"]["outcome"].as_str(), Some("no_path"));
    assert_eq!(
        value["no_path_demo"]["no_path"]["refusal_code"].as_str(),
        Some("unsupported")
    );
    // 索引一致性在规划前后都必须成立。
    assert_eq!(value["index_consistent"].as_bool(), Some(true));
}
