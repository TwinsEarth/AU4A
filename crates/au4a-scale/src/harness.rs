//! v1.9.2 实验框架：把「档位 × 参数」变成一份可复现、可内容寻址的实验报告。
//!
//! 框架本身**不引入第二套算法**：每一行的数值都直接来自 [`crate::metrics`]，
//! 因此「数学表达 / 代码实现 / 数值测试」三者不会各自漂移。
//!
//! 确定性来自三件事：档位固定、参数固定、无浮点（整数定点 + 规范 JSON）。
//! 报告带 `digest = canonical_hash(rows)`，两个 Agent 只要交换 digest 就能确认
//! 彼此跑的是同一份实验——不需要互相上传数据。

use au4a_core::{canonical_hash, CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::metrics::{
    aggregate_throughput_milli, completion_bp, effective_per_node_milli, marginal_gain_milli,
    net_throughput_milli, orchestration_overhead_milli, overhead_ratio_bp, ScalingParams,
};

/// 实验配置：跑哪些节点档位、用什么模型参数、需求是多少。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentConfig {
    /// 节点档位（必须严格递增、非空）。
    pub nodes: Vec<u64>,
    pub params: ScalingParams,
    /// 需求（毫任务/epoch），用于完成率。
    pub demand_milli: i64,
}

impl ExperimentConfig {
    pub fn new(nodes: Vec<u64>, params: ScalingParams, demand_milli: i64) -> Self {
        Self {
            nodes,
            params,
            demand_milli,
        }
    }

    /// 本轨道的标准档位：10 / 100 / 1k / 10k。
    pub fn default_tiers() -> Self {
        Self::new(
            vec![10, 100, 1_000, 10_000],
            ScalingParams::default(),
            20_000,
        )
    }

    pub fn validate(&self) -> CoreResult<()> {
        self.params.validate()?;
        if self.nodes.is_empty() {
            return Err(CoreError::InvalidKind);
        }
        if self.demand_milli <= 0 {
            return Err(CoreError::ZeroAmount);
        }
        let mut previous = 0u64;
        for node in &self.nodes {
            // 严格递增：重复档位会让 digest 与「档位」语义都失去意义。
            if *node == 0 || *node <= previous {
                return Err(CoreError::InvalidKind);
            }
            previous = *node;
        }
        Ok(())
    }

    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn from_json(value: &Value) -> CoreResult<Self> {
        serde_json::from_value(value.clone()).map_err(|_| CoreError::Encoding)
    }
}

/// 一行观测：某个节点档位下的全部度量。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    pub nodes: u64,
    pub per_node_milli: i64,
    pub aggregate_milli: i64,
    pub overhead_milli: i64,
    pub net_milli: i64,
    pub completion_bp: i64,
    pub overhead_ratio_bp: i64,
    /// 模型定义的相邻步差 `T(n+1) − T(n)`。
    pub marginal_gain_milli: i64,
    /// 与**下一个档位**之间的收益差（最后一个档位为 0）。
    pub step_gain_milli: i64,
}

/// 实验报告。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentReport {
    pub config: ExperimentConfig,
    pub rows: Vec<Row>,
    /// `canonical_hash(rows)`：跨 Agent 比对「是否同一份实验」只用它。
    pub digest: String,
}

impl ExperimentReport {
    pub fn row(&self, nodes: u64) -> Option<&Row> {
        self.rows.iter().find(|row| row.nodes == nodes)
    }

    /// 群体吞吐最高的档位（并列时取较小节点数）。
    pub fn peak(&self) -> Option<&Row> {
        self.rows
            .iter()
            .fold(None, |best: Option<&Row>, row| match best {
                Some(current) if current.aggregate_milli >= row.aggregate_milli => Some(current),
                _ => Some(row),
            })
    }

    /// 净收益为负的第一个档位（「收益转负」的档位级证据）。
    pub fn first_negative_net(&self) -> Option<&Row> {
        self.rows.iter().find(|row| row.net_milli < 0)
    }

    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }
}

/// 跑一次实验：确定性、无 I/O、无墙钟。
pub fn run(config: &ExperimentConfig) -> CoreResult<ExperimentReport> {
    config.validate()?;
    let params = &config.params;

    let mut rows = Vec::with_capacity(config.nodes.len());
    for nodes in &config.nodes {
        rows.push(Row {
            nodes: *nodes,
            per_node_milli: effective_per_node_milli(*nodes, params)?,
            aggregate_milli: aggregate_throughput_milli(*nodes, params)?,
            overhead_milli: orchestration_overhead_milli(*nodes, params)?,
            net_milli: net_throughput_milli(*nodes, params)?,
            completion_bp: completion_bp(*nodes, params, config.demand_milli)?,
            overhead_ratio_bp: overhead_ratio_bp(*nodes, params)?,
            marginal_gain_milli: marginal_gain_milli(*nodes, params)?,
            step_gain_milli: 0,
        });
    }
    // 档位间收益差：只在相邻档位之间定义。
    for index in 0..rows.len().saturating_sub(1) {
        let next = rows[index + 1].aggregate_milli;
        let current = rows[index].aggregate_milli;
        rows[index].step_gain_milli = next.checked_sub(current).ok_or(CoreError::Overflow)?;
    }

    let rows_json = serde_json::to_value(&rows).map_err(|_| CoreError::Encoding)?;
    let digest = canonical_hash(&rows_json)?;
    Ok(ExperimentReport {
        config: config.clone(),
        rows,
        digest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config() -> ExperimentConfig {
        ExperimentConfig::default_tiers()
    }

    #[test]
    fn every_row_field_comes_from_the_metrics_module() {
        let report = run(&config()).unwrap();
        let params = &report.config.params;
        for row in &report.rows {
            assert_eq!(
                row.per_node_milli,
                effective_per_node_milli(row.nodes, params).unwrap()
            );
            assert_eq!(
                row.aggregate_milli,
                aggregate_throughput_milli(row.nodes, params).unwrap()
            );
            assert_eq!(
                row.overhead_milli,
                orchestration_overhead_milli(row.nodes, params).unwrap()
            );
            assert_eq!(
                row.net_milli,
                net_throughput_milli(row.nodes, params).unwrap()
            );
            assert_eq!(
                row.completion_bp,
                completion_bp(row.nodes, params, report.config.demand_milli).unwrap()
            );
            assert_eq!(
                row.overhead_ratio_bp,
                overhead_ratio_bp(row.nodes, params).unwrap()
            );
            assert_eq!(
                row.marginal_gain_milli,
                marginal_gain_milli(row.nodes, params).unwrap()
            );
        }
    }

    #[test]
    fn runs_are_deterministic_and_content_addressed() {
        let first = run(&config()).unwrap();
        let second = run(&config()).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.digest, second.digest);
        assert_eq!(first.digest.len(), 64);
        let rows_json = serde_json::to_value(&first.rows).unwrap();
        assert_eq!(first.digest, canonical_hash(&rows_json).unwrap());
    }

    #[test]
    fn step_gain_shows_the_curve_turning() {
        // α=3 且 N0=100：顶点附近之后档位间的收益应当转负。
        let config = ExperimentConfig::new(
            vec![10, 100, 1_000, 10_000],
            ScalingParams::new(100, 3, 1_000_000, 1),
            20_000,
        );
        let report = run(&config).unwrap();
        let first_step = report.row(10).unwrap().step_gain_milli;
        assert!(first_step > 0);
        assert!(
            report.row(1_000).unwrap().step_gain_milli < 0,
            "峰后档位收益必须转负"
        );
        assert_eq!(
            report.row(10_000).unwrap().step_gain_milli,
            0,
            "最后一档没有下一步"
        );
    }

    #[test]
    fn the_peak_helper_picks_the_best_tier_ties_to_the_smaller_one() {
        let report = run(&config()).unwrap();
        let peak = report.peak().unwrap();
        let best = report
            .rows
            .iter()
            .map(|row| row.aggregate_milli)
            .max()
            .unwrap();
        assert_eq!(peak.aggregate_milli, best);
        for row in &report.rows {
            if row.aggregate_milli == best {
                assert!(row.nodes <= peak.nodes);
            }
        }
    }

    #[test]
    fn invalid_configs_are_refused() {
        let params = ScalingParams::default();
        assert_eq!(
            run(&ExperimentConfig::new(vec![], params, 1_000)).err(),
            Some(CoreError::InvalidKind)
        );
        assert_eq!(
            run(&ExperimentConfig::new(vec![100, 100], params, 1_000)).err(),
            Some(CoreError::InvalidKind)
        );
        assert_eq!(
            run(&ExperimentConfig::new(vec![1_000, 100], params, 1_000)).err(),
            Some(CoreError::InvalidKind)
        );
        assert_eq!(
            run(&ExperimentConfig::new(vec![0], params, 1_000)).err(),
            Some(CoreError::InvalidKind)
        );
        assert_eq!(
            run(&ExperimentConfig::new(vec![10], params, 0)).err(),
            Some(CoreError::ZeroAmount)
        );
        assert_eq!(
            run(&ExperimentConfig::new(
                vec![10],
                ScalingParams::new(0, 2, 1, 1),
                1_000
            ))
            .err(),
            Some(CoreError::InvalidKind)
        );
    }

    #[test]
    fn reports_serialize_without_floats_and_configs_roundtrip() {
        let report = run(&config()).unwrap();
        let json = report.to_json().unwrap();
        au4a_core::canonicalize(&json).unwrap();
        assert_eq!(json["digest"], json!(report.digest));
        let config_json = report.config.to_json().unwrap();
        assert_eq!(
            ExperimentConfig::from_json(&config_json).unwrap(),
            report.config
        );
    }
}
