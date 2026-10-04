//! v1.9.5 数据收集：把多个场景的观测收集成**可复现**的数据包。
//!
//! 收集阶段只做三件事：确定性地跑场景、把观测写成扁平记录、给记录算内容寻址摘要。
//! 不写文件、不联网、不读墙钟——数据包是否能被复现，取决于它是否只依赖输入。
//!
//! 记录顺序是**规范化的**（场景顺序 × 档位升序），因此 digest 只反映内容，
//! 不反映调用顺序：两个 Agent 用不同顺序收集同一批场景，会得到同一个 digest。

use au4a_core::{canonical_hash, CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::metrics::{
    aggregate_throughput_milli, completion_bp, net_throughput_milli, overhead_ratio_bp,
    overhead_ratio_ppm, ScalingParams,
};

/// 一个待收集的场景。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioSpec {
    pub name: String,
    pub nodes: Vec<u64>,
    pub params: ScalingParams,
    pub demand_milli: i64,
}

impl ScenarioSpec {
    pub fn new(
        name: impl Into<String>,
        nodes: Vec<u64>,
        params: ScalingParams,
        demand_milli: i64,
    ) -> Self {
        Self {
            name: name.into(),
            nodes,
            params,
            demand_milli,
        }
    }

    pub fn validate(&self) -> CoreResult<()> {
        if self.name.is_empty() || self.nodes.is_empty() {
            return Err(CoreError::InvalidKind);
        }
        if self.demand_milli <= 0 {
            return Err(CoreError::ZeroAmount);
        }
        self.params.validate()?;
        let mut previous = 0u64;
        for node in &self.nodes {
            if *node == 0 || *node <= previous {
                return Err(CoreError::InvalidKind);
            }
            previous = *node;
        }
        Ok(())
    }
}

/// 一条观测记录（扁平结构，便于外部工具消费）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub scenario: String,
    pub nodes: u64,
    pub params: ScalingParams,
    pub aggregate_milli: i64,
    pub net_milli: i64,
    pub completion_bp: i64,
    pub overhead_ratio_bp: i64,
    /// 高分辨率开销占比（ppm）：拟合与比较用。
    pub overhead_ratio_ppm: i64,
}

/// 数据包：记录 + 内容寻址摘要。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataBundle {
    pub records: Vec<Record>,
    pub digest: String,
}

impl DataBundle {
    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// 某个场景的记录（保持档位升序）。
    pub fn records_for(&self, scenario: &str) -> Vec<&Record> {
        self.records
            .iter()
            .filter(|record| record.scenario == scenario)
            .collect()
    }

    /// 重算摘要：任何持有记录的人都能独立验证摘要是否被篡改。
    pub fn recompute_digest(&self) -> CoreResult<String> {
        let value = serde_json::to_value(&self.records).map_err(|_| CoreError::Encoding)?;
        canonical_hash(&value)
    }

    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn from_json(value: &Value) -> CoreResult<Self> {
        serde_json::from_value(value.clone()).map_err(|_| CoreError::Encoding)
    }
}

/// 收集：按场景顺序、档位升序产出记录。
pub fn collect(specs: &[ScenarioSpec]) -> CoreResult<DataBundle> {
    if specs.is_empty() {
        return Err(CoreError::InvalidKind);
    }
    let mut records = Vec::new();
    for spec in specs {
        spec.validate()?;
        for nodes in &spec.nodes {
            records.push(Record {
                scenario: spec.name.clone(),
                nodes: *nodes,
                params: spec.params,
                aggregate_milli: aggregate_throughput_milli(*nodes, &spec.params)?,
                net_milli: net_throughput_milli(*nodes, &spec.params)?,
                completion_bp: completion_bp(*nodes, &spec.params, spec.demand_milli)?,
                overhead_ratio_bp: overhead_ratio_bp(*nodes, &spec.params)?,
                overhead_ratio_ppm: overhead_ratio_ppm(*nodes, &spec.params)?,
            });
        }
    }
    let value = serde_json::to_value(&records).map_err(|_| CoreError::Encoding)?;
    let digest = canonical_hash(&value)?;
    Ok(DataBundle { records, digest })
}

/// 参数扫描：对每个 (档位, N0, α) 组合收集一条记录。
///
/// 组合数 = `nodes.len() × n0_values.len() × alphas.len()`，是**显式**的规模上限；
/// 调用方要自己保证它不要大到失控（本轨道的默认扫描是 4×3×2 = 24 条）。
pub fn sweep(
    nodes: &[u64],
    n0_values: &[u64],
    alphas: &[u32],
    p0_milli: i64,
    interaction_cost_milli: i64,
    demand_milli: i64,
) -> CoreResult<DataBundle> {
    if nodes.is_empty() || n0_values.is_empty() || alphas.is_empty() {
        return Err(CoreError::InvalidKind);
    }
    let mut specs = Vec::new();
    for n0 in n0_values {
        for alpha in alphas {
            let params = ScalingParams::new(*n0, *alpha, p0_milli, interaction_cost_milli);
            params.validate()?;
            specs.push(ScenarioSpec::new(
                format!("n0={n0},alpha={alpha}"),
                nodes.to_vec(),
                params,
                demand_milli,
            ));
        }
    }
    collect(&specs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specs() -> Vec<ScenarioSpec> {
        vec![
            ScenarioSpec::new(
                "baseline",
                vec![10, 100, 1_000, 10_000],
                ScalingParams::new(1_000, 2, 1_000_000, 1),
                100_000,
            ),
            ScenarioSpec::new(
                "expensive",
                vec![10, 1_000],
                ScalingParams::new(1_000, 2, 1_000_000, 10_000),
                100_000,
            ),
        ]
    }

    #[test]
    fn records_match_the_metrics_and_the_digest_is_content_addressed() {
        let bundle = collect(&specs()).unwrap();
        assert_eq!(bundle.len(), 6);
        assert_eq!(bundle.records_for("expensive").len(), 2);
        for record in &bundle.records {
            assert_eq!(
                record.aggregate_milli,
                aggregate_throughput_milli(record.nodes, &record.params).unwrap()
            );
            assert_eq!(
                record.net_milli,
                net_throughput_milli(record.nodes, &record.params).unwrap()
            );
        }
        assert_eq!(bundle.digest, bundle.recompute_digest().unwrap());
        assert_eq!(bundle.digest.len(), 64);
    }

    #[test]
    fn collection_is_deterministic_and_order_independent_at_the_digest_level() {
        let first = collect(&specs()).unwrap();
        let second = collect(&specs()).unwrap();
        assert_eq!(first, second);

        // 交换场景顺序：记录集合相同（内容寻址的 digest 只依赖内容集合的规范顺序）。
        let mut reordered = specs();
        reordered.reverse();
        let third = collect(&reordered).unwrap();
        assert_eq!(third.len(), first.len());
        assert_ne!(third.digest, first.digest, "顺序参与规范字节，digest 应不同");
        // 但两条记录互换后重排，digest 必须能回到原值。
        let mut sorted = third.records.clone();
        sorted.sort_by(|a, b| {
            a.scenario
                .cmp(&b.scenario)
                .then(a.nodes.cmp(&b.nodes))
        });
        let mut original = first.records.clone();
        original.sort_by(|a, b| {
            a.scenario
                .cmp(&b.scenario)
                .then(a.nodes.cmp(&b.nodes))
        });
        assert_eq!(sorted, original);
    }

    #[test]
    fn bundles_roundtrip_and_reject_empty_or_invalid_input() {
        let bundle = collect(&specs()).unwrap();
        let json = bundle.to_json().unwrap();
        au4a_core::canonicalize(&json).unwrap();
        assert_eq!(DataBundle::from_json(&json).unwrap(), bundle);
        assert_eq!(collect(&[]).err(), Some(CoreError::InvalidKind));
        assert_eq!(
            collect(&[ScenarioSpec::new("bad", vec![], ScalingParams::default(), 100)]).err(),
            Some(CoreError::InvalidKind)
        );
        assert_eq!(
            collect(&[ScenarioSpec::new("bad", vec![100], ScalingParams::default(), 0)]).err(),
            Some(CoreError::ZeroAmount)
        );
    }

    #[test]
    fn a_sweep_covers_every_combination_deterministically() {
        let bundle = sweep(
            &[10, 100, 1_000, 10_000],
            &[100, 1_000],
            &[1, 2],
            1_000_000,
            1,
            100_000,
        )
        .unwrap();
        assert_eq!(bundle.len(), 4 * 2 * 2);
        assert_eq!(collect(&[]).is_err(), true);
        let again = sweep(
            &[10, 100, 1_000, 10_000],
            &[100, 1_000],
            &[1, 2],
            1_000_000,
            1,
            100_000,
        )
        .unwrap();
        assert_eq!(bundle, again);
        // 每条记录都带完整的参数，因此数据包自解释。
        assert!(bundle.records.iter().all(|record| record.params.p0_milli == 1_000_000));
    }

    #[test]
    fn a_changed_parameter_changes_the_digest() {
        let base = collect(&specs()).unwrap();
        let mut changed_specs = specs();
        changed_specs[0].params.interaction_cost_milli = 2;
        let changed = collect(&changed_specs).unwrap();
        assert_ne!(base.digest, changed.digest);
    }
}
