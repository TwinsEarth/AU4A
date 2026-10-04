//! v1.1.5 —— 查询接口与倒排索引。
//!
//! 「谁能做 X」这个问题不能靠遍历整张图回答：一个 1000 个 Agent 的视图里，
//! 每次查询都全扫一遍，代价随网络规模线性增长，而且**全扫的结果与索引的结果必须一致**——
//! 所以索引不是「加速手段」，它是这张图的主要读路径，正确性由测试对齐。
//!
//! 索引结构（全部是 `BTreeMap`/`BTreeSet`，确定性迭代顺序）：
//!
//! * `by_skill: SkillId -> { (did, slot) }` —— 主查询路径。
//! * `by_input_format: FormatId -> { (did, slot) }` —— 带输入格式过滤时与技能集求交。
//!
//! `(did, slot)` 是**键**而不是能力的副本：数据只有一份（图/缓存持有），
//! 索引只记「哪里有」，因此不存在「索引与图不一致但两边都自洽」的幽灵状态；
//! 一致性靠 `graph` 在声明、淘汰、过期、失效四个入口同步更新来维持，
//! 并由 v1.1.9 的不变式测试逐个核对。
//!
//! 查询统计（`QueryStats`）是**证据**：它记录了本次查询真正扫描了多少条候选、
//! 整张图里有多少条能力。测试断言前者远小于后者——这才是「走索引」的可证伪形式。

use std::collections::{BTreeMap, BTreeSet};

use au4a_core::{canonical_hash, CoreResult, Credits, Did, RefusalCode};
use serde_json::{json, Value};

use crate::capability::{Capability, FormatId, SkillId};

/// 索引键：哪个 Agent 的第几条能力。
pub type CapKey = (Did, usize);

/// 倒排索引。只存键，不存能力副本。
#[derive(Clone, Debug, Default)]
pub struct CapabilityIndex {
    by_skill: BTreeMap<SkillId, BTreeSet<CapKey>>,
    by_input_format: BTreeMap<FormatId, BTreeSet<CapKey>>,
    by_did: BTreeMap<Did, BTreeSet<usize>>,
    writes: u64,
}

impl CapabilityIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// 索引一个 Agent 的**全部**能力（先清掉该 Agent 的旧键，保证幂等）。
    pub fn insert_agent(&mut self, did: &Did, capabilities: &[Capability]) -> usize {
        self.remove_agent(did);
        for (slot, cap) in capabilities.iter().enumerate() {
            let key = (did.clone(), slot);
            self.by_skill.entry(cap.skill.clone()).or_default().insert(key.clone());
            for format in &cap.supported_formats {
                self.by_input_format
                    .entry(format.clone())
                    .or_default()
                    .insert(key.clone());
            }
            self.by_did.entry(did.clone()).or_default().insert(slot);
            self.writes += 1;
        }
        capabilities.len()
    }

    /// 移除一个 Agent 的全部键（邻居被淘汰/过期/失效时调用）。
    pub fn remove_agent(&mut self, did: &Did) -> usize {
        let Some(slots) = self.by_did.remove(did) else {
            return 0;
        };
        for set in self.by_skill.values_mut() {
            set.retain(|(d, _)| d != did);
        }
        for set in self.by_input_format.values_mut() {
            set.retain(|(d, _)| d != did);
        }
        self.by_skill.retain(|_, set| !set.is_empty());
        self.by_input_format.retain(|_, set| !set.is_empty());
        slots.len()
    }

    /// 某个技能的全部候选键。
    pub fn candidates_for_skill(&self, skill: &SkillId) -> Vec<CapKey> {
        self.by_skill
            .get(skill)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// 某个技能 + 某个输入格式的候选键（两个索引求交，仍然不是全图扫描）。
    pub fn candidates_for(&self, skill: &SkillId, input_format: Option<&FormatId>) -> Vec<CapKey> {
        let Some(skill_set) = self.by_skill.get(skill) else {
            return Vec::new();
        };
        match input_format {
            None => skill_set.iter().cloned().collect(),
            Some(format) => match self.by_input_format.get(format) {
                None => Vec::new(),
                Some(format_set) => skill_set.intersection(format_set).cloned().collect(),
            },
        }
    }

    pub fn indexed_agents(&self) -> usize {
        self.by_did.len()
    }

    pub fn indexed_skills(&self) -> usize {
        self.by_skill.len()
    }

    pub fn indexed_entries(&self) -> usize {
        self.by_did.values().map(BTreeSet::len).sum()
    }

    /// 索引写入次数（v1.1.8 用它证明「增量维护」而不是整表重建）。
    pub fn writes(&self) -> u64 {
        self.writes
    }

    /// `(技能, 键)` 全量列表：一致性不变式检查用（键必须能解析回同技能的能力）。
    pub fn skill_entries(&self) -> Vec<(SkillId, CapKey)> {
        let mut out = Vec::new();
        for (skill, keys) in &self.by_skill {
            for key in keys {
                out.push((skill.clone(), key.clone()));
            }
        }
        out
    }

    pub fn to_value(&self) -> Value {
        json!({
            "agents": self.indexed_agents(),
            "skills": self.indexed_skills(),
            "entries": self.indexed_entries(),
            "writes": self.writes,
        })
    }
}

/// 查询条件。全部是可选的硬过滤：不满足的候选直接不进结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityQuery {
    pub skill: SkillId,
    pub input_format: Option<FormatId>,
    pub output_format: Option<FormatId>,
    pub max_price_per_unit: Option<Credits>,
    pub min_reliability_bp: Option<u16>,
    pub max_latency_p99_ms: Option<u32>,
    pub min_throughput_per_min: Option<u32>,
    pub min_available_bp: Option<u16>,
    pub region: Option<String>,
    /// 返回条数上限；`0` 表示不限。
    pub limit: usize,
}

impl CapabilityQuery {
    pub fn new(skill: SkillId) -> Self {
        Self {
            skill,
            input_format: None,
            output_format: None,
            max_price_per_unit: None,
            min_reliability_bp: None,
            max_latency_p99_ms: None,
            min_throughput_per_min: None,
            min_available_bp: None,
            region: None,
            limit: 8,
        }
    }

    pub fn with_input_format(mut self, format: FormatId) -> Self {
        self.input_format = Some(format);
        self
    }

    pub fn with_output_format(mut self, format: FormatId) -> Self {
        self.output_format = Some(format);
        self
    }

    pub fn with_max_price(mut self, price: Credits) -> Self {
        self.max_price_per_unit = Some(price);
        self
    }

    pub fn with_min_reliability_bp(mut self, bp: u16) -> Self {
        self.min_reliability_bp = Some(bp);
        self
    }

    pub fn with_max_latency_p99_ms(mut self, ms: u32) -> Self {
        self.max_latency_p99_ms = Some(ms);
        self
    }

    pub fn with_min_throughput(mut self, per_min: u32) -> Self {
        self.min_throughput_per_min = Some(per_min);
        self
    }

    pub fn with_min_available_bp(mut self, bp: u16) -> Self {
        self.min_available_bp = Some(bp);
        self
    }

    pub fn with_region(mut self, region: &str) -> Self {
        self.region = Some(region.to_string());
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// 一条能力是否满足全部条件。
    pub fn matches(&self, cap: &Capability) -> bool {
        if cap.skill != self.skill {
            return false;
        }
        if let Some(format) = &self.input_format {
            if !cap.accepts_format(format) {
                return false;
            }
        }
        if let Some(format) = &self.output_format {
            if !cap.produces_format(format) {
                return false;
            }
        }
        if let Some(max) = self.max_price_per_unit {
            if cap.price_per_unit > max {
                return false;
            }
        }
        if let Some(min_bp) = self.min_reliability_bp {
            if cap.reliability_bp < min_bp {
                return false;
            }
        }
        if let Some(max_ms) = self.max_latency_p99_ms {
            if cap.latency_p99_ms > max_ms {
                return false;
            }
        }
        if let Some(min_tp) = self.min_throughput_per_min {
            if cap.throughput_per_min < min_tp {
                return false;
            }
        }
        if let Some(min_bp) = self.min_available_bp {
            if cap.available_bp() < min_bp {
                return false;
            }
        }
        if let Some(region) = &self.region {
            if !cap.constraints.accepts_region(Some(region)) {
                return false;
            }
        }
        true
    }

    /// 查询指纹：v1.1.8 的结果缓存以它为键。
    pub fn fingerprint(&self) -> CoreResult<String> {
        canonical_hash(&self.to_value()?)
    }

    pub fn to_value(&self) -> CoreResult<Value> {
        Ok(json!({
            "skill": self.skill.as_str(),
            "input_format": self.input_format.as_ref().map(FormatId::as_str),
            "output_format": self.output_format.as_ref().map(FormatId::as_str),
            "max_price_per_unit": self.max_price_per_unit.map(|c| c.get()),
            "min_reliability_bp": self.min_reliability_bp,
            "max_latency_p99_ms": self.max_latency_p99_ms,
            "min_throughput_per_min": self.min_throughput_per_min,
            "min_available_bp": self.min_available_bp,
            "region": self.region,
            "limit": self.limit,
        }))
    }
}

/// 一条命中结果。`score` 是排序用的第一关键字（负载加权可靠度，越大越好）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityMatch {
    pub did: Did,
    pub slot: usize,
    pub capability: Capability,
    pub score: i64,
}

impl CapabilityMatch {
    pub fn to_value(&self) -> Value {
        json!({
            "did": self.did.as_str(),
            "slot": self.slot,
            "score": self.score,
            "price_per_unit": self.capability.price_per_unit.get(),
            "latency_p99_ms": self.capability.latency_p99_ms,
            "effective_latency_ms": self.capability.effective_latency_ms(),
            "effective_reliability_bp": self.capability.effective_reliability_bp(),
            "skill": self.capability.skill.as_str(),
        })
    }
}

/// 本次查询实际做了多少工作。**这是「不走全图扫描」的证据**。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueryStats {
    /// 索引给出的候选数（= 真正扫描的条数）。
    pub candidates: usize,
    /// 真正被检查的能力条数（应等于 `candidates`）。
    pub scanned: usize,
    /// 通过过滤的条数。
    pub matched: usize,
    /// 整张图里的能力总条数（对照量）。
    pub nodes_total: usize,
    /// 本查询是否走了「有界 top-k」而不是全排序（v1.1.8 起）。
    pub bounded_selection: bool,
}

impl QueryStats {
    pub fn to_value(self) -> Value {
        json!({
            "candidates": self.candidates,
            "scanned": self.scanned,
            "matched": self.matched,
            "nodes_total": self.nodes_total,
            "bounded_selection": self.bounded_selection,
        })
    }
}

/// 查询结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryResult {
    pub matches: Vec<CapabilityMatch>,
    pub stats: QueryStats,
}

impl QueryResult {
    /// 无候选时的结果（也用于「技能不存在」这种正常结局，而不是错误）。
    pub fn empty(nodes_total: usize) -> Self {
        Self {
            matches: Vec::new(),
            stats: QueryStats {
                nodes_total,
                ..QueryStats::default()
            },
        }
    }

    pub fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }

    pub fn best(&self) -> Option<&CapabilityMatch> {
        self.matches.first()
    }

    /// 「查不到」不是异常，是一种有类型的结局：技能无人提供 → `unsupported`。
    pub fn refusal_code(&self) -> Option<RefusalCode> {
        if self.matches.is_empty() {
            Some(RefusalCode::Unsupported)
        } else {
            None
        }
    }

    pub fn to_value(&self) -> Value {
        json!({
            "matches": self.matches.iter().map(CapabilityMatch::to_value).collect::<Vec<_>>(),
            "count": self.matches.len(),
            "stats": self.stats.to_value(),
        })
    }
}

/// 排序与截断的**参考实现**（v1.1.5 起）：全排序后截断。
///
/// v1.1.8 的 [`crate::perf::rank_top_k`] 在 `limit < 候选数` 时用有界选择替代它，
/// 并且必须与它**逐位一致**——这条不变式由 `perf` 模块的测试断言。
pub fn rank_and_truncate(mut matches: Vec<CapabilityMatch>, limit: usize) -> Vec<CapabilityMatch> {
    matches.sort_by(|a, b| crate::perf::rank_key(a).cmp(&crate::perf::rank_key(b)));
    if limit > 0 && matches.len() > limit {
        matches.truncate(limit);
    }
    matches
}

#[cfg(test)]
mod tests {
    use super::*;
    use au4a_core::AgentKeys;

    fn did(seed: u8) -> Did {
        AgentKeys::from_seed(&[seed; 32]).did()
    }

    fn skill(name: &str) -> SkillId {
        SkillId::new(name).expect("valid")
    }

    fn cap(name: &str, price: i64) -> Capability {
        Capability::new(skill(name), Credits(price))
    }

    #[test]
    fn the_index_maps_skill_to_keys_and_removes_cleanly() {
        let mut index = CapabilityIndex::new();
        let a = did(1);
        index.insert_agent(&a, &[cap("x", 1), cap("y", 2)]);
        assert_eq!(index.candidates_for_skill(&skill("x")), vec![(a.clone(), 0)]);
        assert_eq!(index.candidates_for_skill(&skill("y")), vec![(a.clone(), 1)]);
        assert_eq!(index.candidates_for_skill(&skill("z")), Vec::new());
        assert_eq!(index.indexed_entries(), 2);
        assert_eq!(index.indexed_agents(), 1);

        // 重新索引同一个 Agent 是幂等的（不会累积旧槽位）。
        index.insert_agent(&a, &[cap("x", 1)]);
        assert_eq!(index.indexed_entries(), 1);
        assert_eq!(index.candidates_for_skill(&skill("y")), Vec::new());

        assert_eq!(index.remove_agent(&a), 1);
        assert_eq!(index.indexed_entries(), 0);
        assert_eq!(index.remove_agent(&a), 0);
    }

    #[test]
    fn the_input_format_index_intersects_with_the_skill_index() {
        let mut index = CapabilityIndex::new();
        let a = did(1);
        let b = did(2);
        let json = FormatId::new("application/json").expect("valid");
        index.insert_agent(&a, &[cap("t", 1).with_formats(&["text/plain"], &["application/json"]).expect("v")]);
        index.insert_agent(&b, &[cap("t", 1).with_formats(&["application/json"], &["application/json"]).expect("v")]);
        assert_eq!(index.candidates_for(&skill("t"), None).len(), 2);
        assert_eq!(
            index.candidates_for(&skill("t"), Some(&json)),
            vec![(b.clone(), 0)]
        );
        let absent = FormatId::new("audio/wav").expect("valid");
        assert!(index.candidates_for(&skill("t"), Some(&absent)).is_empty());
    }

    #[test]
    fn query_filters_are_all_hard_conditions() {
        let base_cap = cap("translate.en-zh", 5)
            .with_latency(100, 300)
            .with_throughput(60)
            .with_load_bp(2_000)
            .with_reliability_bp(9_000);
        let base = CapabilityQuery::new(skill("translate.en-zh"));
        assert!(base.matches(&base_cap));
        assert!(!base.clone().with_max_price(Credits(4)).matches(&base_cap));
        assert!(base.clone().with_max_price(Credits(5)).matches(&base_cap));
        assert!(!base.clone().with_min_reliability_bp(9_001).matches(&base_cap));
        assert!(!base.clone().with_max_latency_p99_ms(299).matches(&base_cap));
        assert!(!base.clone().with_min_throughput(61).matches(&base_cap));
        assert!(!base.clone().with_min_available_bp(8_001).matches(&base_cap));
        assert!(base.clone().with_min_available_bp(8_000).matches(&base_cap));
        assert!(base.clone().with_region("eu-west").matches(&base_cap), "空白名单=不限区域，故任何区域都受理");
        assert!(!base.clone().with_output_format(FormatId::new("application/json").expect("v")).matches(&base_cap));
        assert!(base
            .clone()
            .with_input_format(FormatId::new("text/plain").expect("v"))
            .matches(&base_cap));
        assert!(!base.clone().matches(&cap("other.skill", 1)));
    }

    #[test]
    fn ranking_prefers_reliability_then_latency_then_price() {
        let a = CapabilityMatch {
            did: did(1),
            slot: 0,
            score: 9_000,
            capability: cap("x", 5).with_latency(100, 300),
        };
        let b = CapabilityMatch {
            did: did(2),
            slot: 0,
            score: 9_500,
            capability: cap("x", 9).with_latency(500, 900),
        };
        let c = CapabilityMatch {
            did: did(3),
            slot: 0,
            score: 9_000,
            capability: cap("x", 5).with_latency(50, 100),
        };
        let ranked = rank_and_truncate(vec![a.clone(), b.clone(), c.clone()], 0);
        assert_eq!(ranked[0].did, b.did, "可靠度优先");
        assert_eq!(ranked[1].did, c.did, "同可靠度比延迟");
        assert_eq!(ranked[2].did, a.did);
        assert_eq!(rank_and_truncate(vec![a.clone(), b.clone()], 1).len(), 1);
    }

    #[test]
    fn query_fingerprints_are_stable_and_differ_on_any_field() {
        let q = CapabilityQuery::new(skill("x")).with_max_price(Credits(5));
        let same = CapabilityQuery::new(skill("x")).with_max_price(Credits(5));
        assert_eq!(q.fingerprint().expect("hash"), same.fingerprint().expect("hash"));
        assert_ne!(
            q.fingerprint().expect("hash"),
            q.clone().with_limit(1).fingerprint().expect("hash")
        );
    }
}
