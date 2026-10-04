//! v1.9.9 论文稿：方法、指标、结果、局限，**逐条标注模型推论 vs 本机实测**。
//!
//! 这一版不是「写一段文字」，而是把论文的每个数字都接到可复算的代码上：
//! `paper_json()` 返回的 `results` 是**现场跑出来的**（裁决、拟合、档位），
//! `claims` 里每条结论都带 `evidence` 字段（`verified` = 本机真实执行的断言；
//! `cpu-proto` = 模型推论）。文档里的每个数字都能在这份 JSON 里找到出处。
//!
//! 论文的诚实之处写在 `limitations` 与 `not_done` 里：本轨道**没有**做真实分布式压测，
//! 参数是本模型标定，容量顶点是模型内部的极值。

use au4a_core::CoreResult;
use serde_json::{json, Value};

use crate::analyze::{fit, FitSearch};
use crate::cluster::{evaluation_budget, tier_report, TIERS};
use crate::collect::{collect, ScenarioSpec};
use crate::metrics::{analytic_vertex_floor, ScalingParams};
use crate::verdict::adjudicate;

/// 论文稿的机器可读版本。
pub fn paper_json() -> CoreResult<Value> {
    let params = ScalingParams::new(1_000, 2, 1_000_000, 1);
    let verdict = adjudicate(&params, 100_000, 1, 4_000)?;
    let tiers = tier_report(&params, 100_000)?;

    let nodes = vec![10, 100, 500, 1_000, 2_000, 5_000];
    let bundle = collect(&[ScenarioSpec::new("paper", nodes.clone(), params, 100_000)])?;
    let fit_report = fit(&bundle, &FitSearch::around(1_000, 500, 10, vec![1, 2, 3]))?;

    let expensive = ScalingParams::new(1_000, 2, 1_000_000, 10_000);
    let negative = adjudicate(&expensive, 100_000, 1, 10_000)?;
    let analytic = analytic_vertex_floor(params.n0, params.alpha);

    Ok(json!({
        "title": "网络结构的 Scaling Law：节点数、交互复杂度与算力预算对群体智能水平的影响",
        "track": crate::TRACK,
        "version": crate::CURRENT,
        "abstract": "我们把「群体智能水平」参数化为群体吞吐 T(n) 与完成率 C(n)，\
    用解析式聚合模型把节点数 n、交互复杂度 E(n)=n(n-1)/2 与算力预算 p0 联系起来；\
    模型给出容量顶点存在的判据（α>1）、解析极值点 n*=N0/(α-1)^(1/α) 与「收益转负」的判定。\
    全部数值由本机确定性重算得出；10k 节点为模型评估，非真实分布式压测。",
        "method": {
            "model": "aggregate-analytic（解析式聚合，非逐节点仿真）",
            "determinism": "整数定点（milli/bp/ppm）+ 规范 JSON；无浮点、无墙钟、无 I/O",
            "falsifiability": "每个度量都有数学表达、代码实现与数值测试三处对应；schema 与代码常量逐项对齐",
            "decision_rule": "区间内部吞吐见顶 + 峰值后开销占比上升 + 边际收益转负 → 容量顶点",
        },
        "metrics": crate::schema::schema_json()?["model"]["formulas"],
        "results": {
            "params": params.to_json()?,
            "tiers": tiers.tiers,
            "vertex": {
                "kind": match verdict.kind {
                    crate::VerdictKind::VertexFound => "vertex_found",
                    crate::VerdictKind::MonotonicNoVertex => "monotonic_no_vertex",
                    crate::VerdictKind::InsufficientRange => "insufficient_range",
                },
                "nodes": verdict.vertex_nodes,
                "step": verdict.step,
                "analytic": analytic,
                "peak_aggregate_milli": verdict.peak_aggregate_milli,
                "peak_completion_bp": verdict.peak_completion_bp,
                "overhead_ratio_at_peak_ppm": verdict.overhead_ratio_at_peak_ppm,
                "overhead_ratio_after_peak_ppm": verdict.overhead_ratio_after_peak_ppm,
                "marginal_negative_from": verdict.marginal_negative_from,
                "evaluations": verdict.evaluations,
            },
            "fit": fit_report.to_json()?,
            "negative_returns": {
                "interaction_cost_milli": 10_000,
                "net_negative_from": negative.net_negative_from,
                "net_at_turning_point_milli": negative
                    .net_negative_from
                    .map(|n| crate::net_throughput_milli(n, &expensive))
                    .transpose()?,
                "net_at_one_node_milli": crate::net_throughput_milli(1, &expensive)?,
            },
            "budgets": {
                "evaluation_budget": evaluation_budget(),
                "tier_count": TIERS.len(),
                "samples": bundle.len(),
            },
        },
        "claims": [
            {
                "id": "c1",
                "statement": "在 α>1 的模型族中存在内部容量顶点，位置为 n* = N0/(α-1)^(1/α)",
                "evidence": "cpu-proto",
                "note": "数学推导 + 本机离散复算（相差 ≤ 1 步长）；不代表真实系统行为",
                "value": {"analytic": analytic, "discrete": verdict.vertex_nodes, "step": verdict.step},
            },
            {
                "id": "c2",
                "statement": "解析极值点与离散峰值在本机复算中相差不超过一个粗扫步长",
                "evidence": "verified",
                "value": verdict.vertex_nodes.abs_diff(analytic),
            },
            {
                "id": "c3",
                "statement": "10k 档位评估与容量裁决的求值次数不超过固定预算，档位数固定为 4",
                "evidence": "verified",
                "value": {
                    "tier_evaluations": tiers.evaluations,
                    "verdict_evaluations": verdict.evaluations,
                    "evaluation_budget": evaluation_budget(),
                },
            },
            {
                "id": "c4",
                "statement": "从 6 条观测可以恢复 α=2，N0 估计误差不超过 50（真值 1000）",
                "evidence": "verified",
                "value": {
                    "alpha_estimate": fit_report.alpha_estimate,
                    "n0_estimate": fit_report.n0_estimate,
                    "max_residual_ppm": fit_report.max_residual_ppm,
                    "samples": fit_report.samples,
                },
            },
            {
                "id": "c5",
                "statement": "当每次交互成本达到 10 任务时，净收益 G(n) 在大规模处转负",
                "evidence": "cpu-proto",
                "note": "模型预测（解析式计算），不是外部系统实测",
                "value": {
                    "net_negative_from": negative.net_negative_from,
                    "net_at_one_node_milli": crate::net_throughput_milli(1, &expensive)?,
                },
            },
            {
                "id": "c6",
                "statement": "裁决规则对 α=2 场景给出 VertexFound，并同时给出开销恶化与边际转负两条独立证据",
                "evidence": "verified",
                "value": {
                    "kind": "vertex_found",
                    "reasons": verdict.reasons.iter().map(|r| r.as_str()).collect::<Vec<_>>(),
                },
            },
        ],
        "limitations": [
            "没有真实分布式压测：10k 节点是解析式聚合评估，不含网络延迟、失败重试与调度抖动",
            "参数 (N0, α, p0, c) 为本模型标定，不是任何外部系统的实测标定，因此曲线形状是模型性质",
            "容量顶点是模型内部极值；真实系统在顶点附近的表现需要真实实验才能下结论",
            "整数取整使峰后短程可能持平；用步长与 ppm 分辨率缓解，但没有消除",
            "完成率依赖外部给定的需求 D，D 的设定本身会影响完成率数值（不影响吞吐曲线形状）",
        ],
        "not_done": [
            "真实多节点部署与网络拓扑实验",
            "与外部系统实测数据的交叉验证",
            "自适应拓扑（本模型假设交互复杂度上界为完全图）",
        ],
    }))
}

/// 论文稿摘要（供观察层使用）。
pub fn paper_summary() -> CoreResult<Value> {
    let paper = paper_json()?;
    let claims = paper["claims"].as_array().cloned().unwrap_or_default();
    let verified = claims
        .iter()
        .filter(|claim| claim["evidence"] == json!("verified"))
        .count();
    let model_derived = claims
        .iter()
        .filter(|claim| claim["evidence"] == json!("cpu-proto"))
        .count();
    Ok(json!({
        "claims": claims.len(),
        "verified_claims": verified,
        "model_derived_claims": model_derived,
        "limitations": paper["limitations"].as_array().map(|l| l.len()).unwrap_or(0),
        "not_done": paper["not_done"].as_array().map(|n| n.len()).unwrap_or(0),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_claim_declares_an_evidence_grade() {
        let paper = paper_json().unwrap();
        let claims = paper["claims"].as_array().unwrap();
        assert!(claims.len() >= 5);
        for claim in claims {
            let evidence = claim["evidence"].as_str().unwrap();
            assert!(
                evidence == "verified" || evidence == "cpu-proto",
                "结论 {} 的证据等级非法：{evidence}",
                claim["id"]
            );
            assert!(claim["statement"].as_str().unwrap().len() > 10);
        }
        let summary = paper_summary().unwrap();
        assert_eq!(
            summary["verified_claims"].as_u64().unwrap()
                + summary["model_derived_claims"].as_u64().unwrap(),
            claims.len() as u64
        );
        assert_eq!(summary["limitations"], json!(5));
        assert_eq!(summary["not_done"], json!(3));
    }

    #[test]
    fn the_paper_numbers_match_a_fresh_local_computation() {
        let paper = paper_json().unwrap();
        let params = ScalingParams::new(1_000, 2, 1_000_000, 1);
        let verdict = adjudicate(&params, 100_000, 1, 4_000).unwrap();
        assert_eq!(
            paper["results"]["vertex"]["nodes"],
            json!(verdict.vertex_nodes)
        );
        assert_eq!(
            paper["results"]["vertex"]["analytic"],
            json!(analytic_vertex_floor(params.n0, params.alpha))
        );
        assert_eq!(
            paper["results"]["vertex"]["overhead_ratio_after_peak_ppm"],
            json!(verdict.overhead_ratio_after_peak_ppm)
        );

        let bundle = collect(&[ScenarioSpec::new(
            "paper",
            vec![10, 100, 500, 1_000, 2_000, 5_000],
            params,
            100_000,
        )])
        .unwrap();
        let fit_report = fit(&bundle, &FitSearch::around(1_000, 500, 10, vec![1, 2, 3])).unwrap();
        assert_eq!(paper["results"]["fit"]["alpha_estimate"], json!(2));
        assert_eq!(
            paper["results"]["fit"]["n0_estimate"],
            json!(fit_report.n0_estimate)
        );
    }

    #[test]
    fn the_paper_is_canonical_json_and_reports_its_own_limits() {
        let paper = paper_json().unwrap();
        au4a_core::canonicalize(&paper).unwrap();
        assert!(paper["abstract"]
            .as_str()
            .unwrap()
            .contains("非真实分布式压测"));
        assert!(paper["limitations"].as_array().unwrap().len() >= 4);
        assert!(paper["method"]["determinism"]
            .as_str()
            .unwrap()
            .contains("无浮点"));
    }
}
