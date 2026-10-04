//! 一致性检查（v1.3.5）：三区摘要 + 块级差异报告。
//!
//! 「恢复成功」不能只写一句「返回 Ok」。真实系统里状态损坏的形态是具体的：
//! 少了几个键、多了几个键、某个块被改过、某代被换过、迁移半途而废。这一版把这些
//! 形态变成**可定位**的结论：区 + 键 + 类型（missing / extra / modified），
//! 而不是一个笼统的「校验失败」。
//!
//! 三种检查各管一段：
//!
//! * [`compare`]：两份快照之间（源 vs 恢复后）。
//! * [`compare_store`]：存储**原始字节** vs 期望快照——不经过 `read_snapshot`，
//!   因为「读不出来」本身就是要报告的一种损坏。
//! * [`audit_node`]：节点层面的 2PC 卫生（孤儿代、未完成迁移的意图记录）。

use std::collections::BTreeMap;

use au4a_core::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::recovery::NodeStore;
use crate::snapshot::{StateBlock, StateSnapshot, StateZone};
use crate::store::{split_store_key, StateStore};

/// 发现的问题类型。**可机读**，观察层可以按类型聚合。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingCode {
    /// 源有、恢复后没有。
    Missing,
    /// 恢复后有、源没有。
    Extra,
    /// 两边都有但内容摘要不同。
    Modified,
    /// 存储里的键不合规范（连区名都解析不出来）。
    BadKey,
    /// 键存在但值读不出来。
    Unreadable,
    /// 代存储里有不该存在的代。
    OrphanGeneration,
    /// 有未完成的迁移（意图记录还在）。
    UnfinishedMigration,
    /// 意图记录声明的目标与当前 live 不一致。
    IntentMismatch,
}

impl FindingCode {
    pub fn as_str(self) -> &'static str {
        match self {
            FindingCode::Missing => "missing",
            FindingCode::Extra => "extra",
            FindingCode::Modified => "modified",
            FindingCode::BadKey => "bad_key",
            FindingCode::Unreadable => "unreadable",
            FindingCode::OrphanGeneration => "orphan_generation",
            FindingCode::UnfinishedMigration => "unfinished_migration",
            FindingCode::IntentMismatch => "intent_mismatch",
        }
    }
}

/// 一条可定位的发现。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub code: FindingCode,
    pub zone: Option<StateZone>,
    pub key: Option<String>,
    pub detail: String,
}

impl Finding {
    fn new(code: FindingCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            zone: None,
            key: None,
            detail: detail.into(),
        }
    }

    fn at(code: FindingCode, zone: StateZone, key: &str, detail: impl Into<String>) -> Self {
        Self {
            code,
            zone: Some(zone),
            key: Some(key.to_string()),
            detail: detail.into(),
        }
    }
}

/// 单区报告。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZoneReport {
    pub zone: StateZone,
    pub left_root: String,
    pub right_root: String,
    pub equal: bool,
    pub missing: Vec<String>,
    pub extra: Vec<String>,
    pub modified: Vec<String>,
}

impl ZoneReport {
    pub fn is_clean(&self) -> bool {
        self.missing.is_empty() && self.extra.is_empty() && self.modified.is_empty()
    }

    pub fn changed(&self) -> usize {
        self.missing.len() + self.extra.len() + self.modified.len()
    }
}

/// 两份快照的一致性报告。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsistencyReport {
    pub left_content_root: String,
    pub right_content_root: String,
    pub identical: bool,
    pub left_blocks: usize,
    pub right_blocks: usize,
    pub zones: Vec<ZoneReport>,
    pub findings: Vec<Finding>,
}

impl ConsistencyReport {
    /// 无任何发现 = 干净。空报告不算干净（宁可显式失败）。
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty() && self.identical
    }

    pub fn changed_blocks(&self) -> usize {
        self.zones.iter().map(|z| z.changed()).sum()
    }

    pub fn found(&self, code: FindingCode) -> Vec<&Finding> {
        self.findings.iter().filter(|f| f.code == code).collect()
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// 存储原始内容 vs 期望快照的报告。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreReport {
    pub prefix: String,
    pub expected_content_root: String,
    /// 存储损坏到无法构成快照时为 `None`——这本身就是结论。
    pub observed_content_root: Option<String>,
    pub identical: bool,
    pub observed_blocks: usize,
    pub zones: Vec<ZoneReport>,
    pub findings: Vec<Finding>,
}

impl StoreReport {
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty() && self.identical
    }

    pub fn found(&self, code: FindingCode) -> Vec<&Finding> {
        self.findings.iter().filter(|f| f.code == code).collect()
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// 节点层面的审计。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeAudit {
    pub node: String,
    pub head: u64,
    pub live_content_root: String,
    pub live_blocks: usize,
    pub orphan_generations: Vec<u64>,
    pub intent: Option<Value>,
    pub findings: Vec<Finding>,
}

impl NodeAudit {
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }

    pub fn found(&self, code: FindingCode) -> Vec<&Finding> {
        self.findings.iter().filter(|f| f.code == code).collect()
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// 比较两份快照：`left` 是源，`right` 是恢复后的结果。
pub fn compare(left: &StateSnapshot, right: &StateSnapshot) -> CoreResult<ConsistencyReport> {
    let mut zones = Vec::new();
    let mut findings = Vec::new();
    for zone in StateZone::ALL {
        let left_map: BTreeMap<&str, &StateBlock> =
            left.zone_blocks(zone).map(|b| (b.key(), b)).collect();
        let right_map: BTreeMap<&str, &StateBlock> =
            right.zone_blocks(zone).map(|b| (b.key(), b)).collect();

        let mut missing = Vec::new();
        let mut extra = Vec::new();
        let mut modified = Vec::new();
        for (key, block) in &left_map {
            match right_map.get(key) {
                None => {
                    missing.push((*key).to_string());
                    findings.push(Finding::at(
                        FindingCode::Missing,
                        zone,
                        key,
                        "源有、恢复后缺失",
                    ));
                }
                Some(other) if other.digest() != block.digest() => {
                    modified.push((*key).to_string());
                    findings.push(Finding::at(
                        FindingCode::Modified,
                        zone,
                        key,
                        "块内容与源不一致",
                    ));
                }
                Some(_) => {}
            }
        }
        for key in right_map.keys() {
            if !left_map.contains_key(key) {
                extra.push((*key).to_string());
                findings.push(Finding::at(
                    FindingCode::Extra,
                    zone,
                    key,
                    "恢复后多出源上没有的块",
                ));
            }
        }
        let left_root = left.zone_root(zone)?;
        let right_root = right.zone_root(zone)?;
        zones.push(ZoneReport {
            zone,
            equal: left_root == right_root,
            left_root,
            right_root,
            missing,
            extra,
            modified,
        });
    }
    Ok(ConsistencyReport {
        left_content_root: left.content_root()?,
        right_content_root: right.content_root()?,
        identical: left.same_content(right),
        left_blocks: left.blocks().len(),
        right_blocks: right.blocks().len(),
        zones,
        findings,
    })
}

/// 直接读存储原始内容并与期望快照比对。
///
/// 刻意**不走** [`crate::store::read_snapshot`]：那条路径遇到坏键会直接报错，
/// 而这里的任务是**指出坏在哪里**。
pub fn compare_store<S: StateStore + ?Sized>(
    store: &S,
    prefix: &str,
    expected: &StateSnapshot,
) -> CoreResult<StoreReport> {
    let mut findings = Vec::new();
    let mut observed: BTreeMap<(StateZone, String), StateBlock> = BTreeMap::new();
    let keys = store.list(prefix)?;
    for key in &keys {
        let (zone, name) = match split_store_key(prefix, key) {
            Some(pair) => pair,
            None => {
                findings.push(Finding::new(
                    FindingCode::BadKey,
                    format!("键不符合 <zone>:<key> 规范: {key}"),
                ));
                continue;
            }
        };
        match store.get(key)? {
            None => findings.push(Finding::new(
                FindingCode::Unreadable,
                format!("键存在但读不出值: {key}"),
            )),
            Some(value) => match StateBlock::new(zone, name.to_string(), value) {
                Ok(block) => {
                    observed.insert((zone, name.to_string()), block);
                }
                Err(e) => findings.push(Finding::at(
                    FindingCode::Unreadable,
                    zone,
                    name,
                    format!("块不合法（{e}）: {key}"),
                )),
            },
        }
    }

    let mut expected_map: BTreeMap<(StateZone, String), &StateBlock> = BTreeMap::new();
    for block in expected.blocks() {
        expected_map.insert((block.zone(), block.key().to_string()), block);
    }

    let mut zones = Vec::new();
    for zone in StateZone::ALL {
        let mut missing = Vec::new();
        let mut extra = Vec::new();
        let mut modified = Vec::new();
        for ((z, key), block) in &expected_map {
            if *z != zone {
                continue;
            }
            match observed.get(&(*z, key.clone())) {
                None => {
                    missing.push(key.clone());
                    findings.push(Finding::at(
                        FindingCode::Missing,
                        zone,
                        key,
                        "存储中缺失该块",
                    ));
                }
                Some(other) if other.digest() != block.digest() => {
                    modified.push(key.clone());
                    findings.push(Finding::at(
                        FindingCode::Modified,
                        zone,
                        key,
                        "存储中的块内容被改过",
                    ));
                }
                Some(_) => {}
            }
        }
        for ((z, key), _) in &observed {
            if *z == zone && !expected_map.contains_key(&(*z, key.clone())) {
                extra.push(key.clone());
                findings.push(Finding::at(
                    FindingCode::Extra,
                    zone,
                    key,
                    "存储中存在源上没有的块",
                ));
            }
        }
        // 区摘要可以直接从观测块重建（观测块已经过 StateBlock 校验）。
        let observed_zone_blocks: Vec<StateBlock> = observed
            .iter()
            .filter(|((z, _), _)| *z == zone)
            .map(|(_, b)| b.clone())
            .collect();
        let observed_zone_root = zone_root_of(zone, &observed_zone_blocks)?;
        zones.push(ZoneReport {
            zone,
            left_root: expected.zone_root(zone)?,
            right_root: observed_zone_root,
            equal: missing.is_empty() && extra.is_empty() && modified.is_empty(),
            missing,
            extra,
            modified,
        });
    }

    let observed_blocks: Vec<StateBlock> = observed.values().cloned().collect();
    let observed_content_root = content_root_of(expected, &observed_blocks);

    let identical = observed_content_root
        .as_deref()
        .map(|r| r == expected.content_root().unwrap_or_default())
        .unwrap_or(false);

    Ok(StoreReport {
        prefix: prefix.to_string(),
        expected_content_root: expected.content_root()?,
        observed_content_root,
        identical,
        observed_blocks: observed.len(),
        zones,
        findings,
    })
}

/// 节点 2PC 卫生审计：孤儿代、未完成迁移、意图与 live 是否一致。
pub fn audit_node<S: StateStore>(node: &NodeStore<S>) -> CoreResult<NodeAudit> {
    let head = node.head()?;
    let live = node.live_snapshot()?;
    let live_content_root = live.content_root()?;
    let orphans = node.orphan_generations(head)?;
    let intent = node.intent()?;
    let mut findings = Vec::new();
    for gen in &orphans {
        findings.push(Finding::new(
            FindingCode::OrphanGeneration,
            format!("第 {gen} 代不是当前生效代（head={head}）"),
        ));
    }
    if let Some(value) = &intent {
        findings.push(Finding::new(
            FindingCode::UnfinishedMigration,
            format!(
                "存在未完成的迁移意图: {}",
                value.get("tx").and_then(|t| t.as_str()).unwrap_or("?")
            ),
        ));
        if let Some(target) = value.get("target").and_then(|t| t.as_str()) {
            if target != live_content_root {
                findings.push(Finding::new(
                    FindingCode::IntentMismatch,
                    format!("意图声明的目标 {target} 与当前 live {live_content_root} 不一致"),
                ));
            }
        }
    }
    Ok(NodeAudit {
        node: node.node().as_str().to_string(),
        head,
        live_content_root,
        live_blocks: live.blocks().len(),
        orphan_generations: orphans,
        intent,
        findings,
    })
}

/// 某个代命名空间下可被观测的块数（用于报告「代里到底有多少东西」）。
pub fn generation_block_count<S: StateStore + ?Sized>(store: &S, gen: u64) -> CoreResult<usize> {
    Ok(store.list(&crate::recovery::generation_prefix(gen))?.len())
}

fn zone_root_of(zone: StateZone, blocks: &[StateBlock]) -> CoreResult<String> {
    // 与 StateSnapshot::zone_root 的取值保持一致：空区也有确定摘要。
    let mut sorted = blocks.to_vec();
    sorted.sort_by(|a, b| a.key().as_bytes().cmp(b.key().as_bytes()));
    let locators: Vec<Value> = sorted.iter().map(|b| b.locator()).collect();
    au4a_core::canonical_hash(&serde_json::json!({"b": locators, "z": zone.as_str()}))
}

fn content_root_of(expected: &StateSnapshot, blocks: &[StateBlock]) -> Option<String> {
    let snapshot = StateSnapshot::capture(
        expected.agent(),
        expected.source_node(),
        expected.epoch(),
        blocks.to_vec(),
    )
    .ok()?;
    snapshot.content_root().ok()
}

/// 把报告压成一行人类可读的结论（观察层日志用）。
pub fn summarize(report: &ConsistencyReport) -> String {
    if report.is_clean() {
        return format!(
            "一致：{} 块，content_root={}",
            report.left_blocks,
            crate::short(&report.left_content_root)
        );
    }
    let mut parts = Vec::new();
    for zone in &report.zones {
        if !zone.is_clean() {
            parts.push(format!(
                "{}: -{} +{} ~{}",
                zone.zone.as_str(),
                zone.missing.len(),
                zone.extra.len(),
                zone.modified.len()
            ));
        }
    }
    format!(
        "不一致（{} 处）: {}",
        report.changed_blocks(),
        parts.join("; ")
    )
}

/// 校验一个错误码是否是「一致性失败」——供调用方分类。
pub fn is_integrity_error(err: &CoreError) -> bool {
    matches!(
        err,
        CoreError::InvalidSignature | CoreError::Encoding | CoreError::InvalidVersion
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recovery::INTENT_KEY;
    use crate::recovery::NodeStore;
    use crate::store::MemoryStore;
    use crate::transfer::NodeId;
    use serde_json::json;

    fn keys(seed: u8) -> au4a_core::AgentKeys {
        au4a_core::AgentKeys::from_seed(&[seed; 32])
    }

    fn snap(seed: u8, pairs: &[(StateZone, &str, Value)]) -> StateSnapshot {
        let blocks: Vec<StateBlock> = pairs
            .iter()
            .map(|(z, k, v)| StateBlock::new(*z, *k, v.clone()).unwrap())
            .collect();
        StateSnapshot::capture(&keys(seed).did(), "node-a", 1, blocks).unwrap()
    }

    fn sample(seed: u8) -> StateSnapshot {
        snap(
            seed,
            &[
                (StateZone::Fs, "/a", json!({"n": 1})),
                (StateZone::Memory, "m", json!(2)),
                (StateZone::Context, "goal", json!("g")),
            ],
        )
    }

    #[test]
    fn identical_snapshots_report_clean() {
        let report = compare(&sample(1), &sample(1)).unwrap();
        assert!(report.is_clean());
        assert_eq!(report.changed_blocks(), 0);
        assert!(report.findings.is_empty());
        assert!(report.zones.iter().all(|z| z.equal));
        assert!(summarize(&report).starts_with("一致"));
    }

    #[test]
    fn damage_is_located_by_zone_and_key() {
        let left = sample(2);
        let right = snap(
            2,
            &[
                // /a 被改
                (StateZone::Fs, "/a", json!({"n": 2})),
                // m 缺失
                // goal 保留
                (StateZone::Context, "goal", json!("g")),
                // 多出一块
                (StateZone::Memory, "extra", json!(true)),
            ],
        );
        let report = compare(&left, &right).unwrap();
        assert!(!report.is_clean());
        assert_eq!(report.found(FindingCode::Modified).len(), 1);
        assert_eq!(report.found(FindingCode::Missing).len(), 1);
        assert_eq!(report.found(FindingCode::Extra).len(), 1);
        let fs = report
            .zones
            .iter()
            .find(|z| z.zone == StateZone::Fs)
            .unwrap();
        assert_eq!(fs.modified, vec!["/a".to_string()]);
        assert!(!fs.equal);
        let memory = report
            .zones
            .iter()
            .find(|z| z.zone == StateZone::Memory)
            .unwrap();
        assert_eq!(memory.missing, vec!["m".to_string()]);
        assert_eq!(memory.extra, vec!["extra".to_string()]);
        assert!(summarize(&report).contains("fs"));
    }

    #[test]
    fn store_damage_is_found_from_raw_bytes() {
        let expected = sample(3);
        let mut store = MemoryStore::new();
        crate::store::write_snapshot(&mut store, "live:", &expected).unwrap();
        // 完好时干净。
        assert!(compare_store(&store, "live:", &expected).unwrap().is_clean());

        // 改一块、删一块、加一块、塞一个坏键。
        store.put("live:memory:m", json!({"tampered": true})).unwrap();
        store.remove("live:context:goal").unwrap();
        store.put("live:fs:/injected", json!(1)).unwrap();
        store.put("live:nonsense", json!(1)).unwrap();
        let report = compare_store(&store, "live:", &expected).unwrap();
        assert!(!report.is_clean());
        assert_eq!(report.found(FindingCode::Modified).len(), 1);
        assert_eq!(report.found(FindingCode::Missing).len(), 1);
        assert_eq!(report.found(FindingCode::Extra).len(), 1);
        assert_eq!(report.found(FindingCode::BadKey).len(), 1);
        assert!(report.observed_content_root.is_some());
        assert!(!report.identical);
    }

    #[test]
    fn a_clean_node_audit_has_no_findings() {
        let a = keys(4);
        let mut node = NodeStore::open(NodeId::new("node-b").unwrap(), a.did(), MemoryStore::new())
            .unwrap();
        node.install(&sample(4), true).unwrap();
        let audit = audit_node(&node).unwrap();
        assert!(audit.is_clean(), "{:?}", audit.findings);
        assert!(audit.orphan_generations.is_empty());
        assert!(audit.intent.is_none());
    }

    #[test]
    fn an_unfinished_migration_is_reported() {
        let a = keys(5);
        let mut node = NodeStore::open(NodeId::new("node-b").unwrap(), a.did(), MemoryStore::new())
            .unwrap();
        node.install(&sample(5), true).unwrap();
        node.store_mut()
            .put(
                INTENT_KEY,
                json!({"tx": "deadbeef", "staged": 9, "previous": 2, "target": "00"}),
            )
            .unwrap();
        let audit = audit_node(&node).unwrap();
        assert!(!audit.is_clean());
        assert_eq!(audit.found(FindingCode::UnfinishedMigration).len(), 1);
        assert_eq!(audit.found(FindingCode::IntentMismatch).len(), 1);
    }

    #[test]
    fn integrity_error_classification_is_explicit() {
        assert!(is_integrity_error(&CoreError::InvalidSignature));
        assert!(!is_integrity_error(&CoreError::Overflow));
    }
}
