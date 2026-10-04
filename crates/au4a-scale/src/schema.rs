//! v1.9.8 机器可读契约：把模型、单位、预算、裁决规则、证据分级导出成一份 JSON。
//!
//! 为什么值得单独一版：参考项目最贵的失败是**文档声称的能力在代码里不可达**。
//! 这里把「度量定义 + 单位 + 求值预算 + 裁决码 + 证据分级 + 局限」写成可被程序检查的
//! 一份 JSON，并用测试断言它与代码里的常量逐个对齐；任何一方漂移都会红。

use au4a_core::CoreResult;
use serde_json::{json, Value};

use crate::cluster::{evaluation_budget, MAX_SCAN_SAMPLES, TIERS};
use crate::metrics::ScalingParams;
use crate::verdict::{VerdictKind, VerdictReason};

/// 度量与裁决的机器可读契约。
pub fn schema_json() -> CoreResult<Value> {
    Ok(json!({
        "track": crate::TRACK,
        "title": crate::TITLE,
        "range": crate::RANGE,
        "current": crate::CURRENT,
        "claim_scope": "确定性聚合模型（解析式实现）；10k 节点为模型评估，非真实分布式压测",
        "units": {
            "milli": "千分之一任务 / epoch",
            "bp": "万分之一（10000 bp = 1.0）",
            "ppm": "百万分之一（1000000 ppm = 1.0）",
        },
        "model": {
            "params": ["n0", "alpha", "p0_milli", "interaction_cost_milli"],
            "alpha_range": [ScalingParams::MIN_ALPHA, ScalingParams::MAX_ALPHA],
            "formulas": {
                "interaction_complexity": "E(n) = n*(n-1)/2",
                "effective_per_node_milli": "p(n) = p0 * N0^alpha / (N0^alpha + n^alpha)",
                "aggregate_throughput_milli": "T(n) = n * p(n)",
                "orchestration_overhead_milli": "O(n) = c * E(n) / 1000",
                "net_throughput_milli": "G(n) = T(n) - O(n)",
                "completion_bp": "C(n) = min(10000, 10000*T(n)/D)",
                "overhead_ratio_bp": "R(n) = 10000*O(n)/T(n)",
                "overhead_ratio_ppm": "R_ppm(n) = 1000000*O(n)/T(n)",
                "marginal_gain_milli": "dT(n) = T(n+1) - T(n)",
                "analytic_vertex": "n* = N0 / (alpha-1)^(1/alpha)，仅 alpha > 1 时存在内部极值",
            },
            "vertex_exists_iff": "alpha > 1",
            "rounding": "p(n) 向下取整；单点误差上界 = 每节点 1 milli；比较占比时用 ppm 保分辨率",
        },
        "tiers": TIERS,
        "budgets": {
            "max_scan_samples": MAX_SCAN_SAMPLES,
            "evaluation_budget": evaluation_budget(),
            "memory": "返回定长结构；无长度随 N 增长的容器",
        },
        "verdict": {
            "kinds": VerdictKind::ALL.iter().map(|k| k.as_str()).collect::<Vec<_>>(),
            "reasons": VerdictReason::ALL.iter().map(|r| r.as_str()).collect::<Vec<_>>(),
            "rule": "区间内部吞吐见顶 + 峰值后开销占比上升 + 边际收益转负 → 容量顶点",
        },
        "evidence_grades": {
            "verified": "本机真实执行的断言（数值一致性、确定性与 digest、求值预算、墙钟上界由测试给出）",
            "cpu_proto": "模型推论：曲线外推、极值点位置、收益转负的节点数（解析式预测，非外部系统实测）",
            "not_done": "真实分布式压测、真实网络拓扑与延迟、跨节点一致性",
        },
        "limitations": [
            "模型是解析式聚合，不含网络延迟分布、失败重试与真实调度抖动",
            "参数 (N0, alpha, p0, c) 是本模型标定，不是任何外部系统的实测标定",
            "容量顶点是模型内部的极值，不代表真实系统在该节点数会退化",
            "整数取整会让峰后短程持平；判定用步长与 ppm 分辨率缓解，但不消除",
        ],
    }))
}

/// 契约摘要（供 `results_json` 与观察层使用）。
pub fn schema_summary() -> CoreResult<Value> {
    let schema = schema_json()?;
    Ok(json!({
        "formulas": schema["model"]["formulas"].as_object().map(|m| m.len()).unwrap_or(0),
        "tiers": schema["tiers"].as_array().map(|t| t.len()).unwrap_or(0),
        "verdict_kinds": schema["verdict"]["kinds"].as_array().map(|v| v.len()).unwrap_or(0),
        "verdict_reasons": schema["verdict"]["reasons"].as_array().map(|v| v.len()).unwrap_or(0),
        "limitations": schema["limitations"].as_array().map(|l| l.len()).unwrap_or(0),
        "evaluation_budget": schema["budgets"]["evaluation_budget"],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schema_covers_every_constant_of_the_code() {
        let schema = schema_json().unwrap();
        assert_eq!(schema["track"], json!("1.9"));
        assert_eq!(schema["tiers"], json!(TIERS));
        assert_eq!(
            schema["budgets"]["max_scan_samples"],
            json!(MAX_SCAN_SAMPLES)
        );
        assert_eq!(
            schema["budgets"]["evaluation_budget"],
            json!(evaluation_budget())
        );
        assert_eq!(
            schema["verdict"]["kinds"].as_array().unwrap().len(),
            VerdictKind::ALL.len()
        );
        assert_eq!(
            schema["verdict"]["reasons"].as_array().unwrap().len(),
            VerdictReason::ALL.len()
        );
        for reason in VerdictReason::ALL {
            let listed = schema["verdict"]["reasons"].as_array().unwrap();
            let hits = listed
                .iter()
                .filter(|value| value.as_str() == Some(reason.as_str()))
                .count();
            assert_eq!(hits, 1, "{} 应恰好出现一次", reason.as_str());
        }
    }

    #[test]
    fn the_schema_is_canonical_json_without_floats() {
        let schema = schema_json().unwrap();
        au4a_core::canonicalize(&schema).unwrap();
        let summary = schema_summary().unwrap();
        assert_eq!(summary["formulas"], json!(10));
        assert_eq!(summary["tiers"], json!(TIERS.len()));
        assert_eq!(summary["limitations"], json!(4));
    }

    #[test]
    fn every_claim_scope_is_stated() {
        let schema = schema_json().unwrap();
        assert!(schema["claim_scope"]
            .as_str()
            .unwrap()
            .contains("非真实分布式压测"));
        assert!(schema["evidence_grades"]["not_done"].is_string());
        assert_eq!(schema["limitations"].as_array().unwrap().len(), 4);
    }
}
