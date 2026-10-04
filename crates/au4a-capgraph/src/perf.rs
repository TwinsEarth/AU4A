//! v1.1.8 —— 性能优化（用**操作计数**证明，不读墙钟）。
//!
//! 本 crate 被契约禁止读真实时间，所以「快不快」不能用 `Instant` 自证。
//! 这一版改成可证伪的形式：**数操作**。
//!
//! 1. **增量索引维护**：v1.1.5 起索引只在受影响的 Agent 上更新；
//!    这里把「整表重建」单独做成一条路径（[`CapabilityIndex::rebuild`]）并计数，
//!    测试断言正常读写过程中 `rebuilds == 0`，而写入次数正好等于入图的能力数——
//!    也就是每次更新是 `O(该 Agent 的能力数)`，不是 `O(全图)`。
//! 2. **有界 top-k**：`limit < 候选数` 时用大小为 k 的堆做选择，不做全排序；
//!    统计里 `full_sorts` 必须为 0、`bounded_selections` 为 1，
//!    且结果与「全排序后截断」**逐位相同**（优化不得改变语义）。
//! 3. **查询结果缓存**：以 `(查询指纹, 图修订号)` 为键，容量有上限、LRU 淘汰。
//!    任何一次数据变更都会推进修订号，从而让旧键自然失效——
//!    「缓存正确性」由「命中结果 == 未命中结果」的断言保证，而不是靠小心。
//!
//! 这三件事都只是**优化**：v1.1.9 的不变式测试会比较优化前后的结果是否一致。

use std::collections::BinaryHeap;

use crate::index::{CapabilityMatch, CapKey, CapabilityIndex, QueryResult};

/// 累计查询性能计数（每个图一份，全部是整数计数）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueryPerf {
    /// 执行的查询次数。
    pub queries: u64,
    /// 因 `limit == 0` 或 `limit >= 候选数` 而做全排序的次数。
    pub full_sorts: u64,
    /// 使用有界选择（大小为 k 的堆）的次数。
    pub bounded_selections: u64,
    /// 排序/选择过程中的比较次数（有界选择的比较是 O(n log k)）。
    pub comparisons: u64,
    /// 累计扫描的候选条数（索引给出的候选，不是全图）。
    pub candidates_scanned: u64,
    /// 索引整表重建次数（正常路径应当恒为 0）。
    pub index_rebuilds: u64,
}

impl QueryPerf {
    pub fn to_value(self) -> serde_json::Value {
        serde_json::json!({
            "queries": self.queries,
            "full_sorts": self.full_sorts,
            "bounded_selections": self.bounded_selections,
            "comparisons": self.comparisons,
            "candidates_scanned": self.candidates_scanned,
            "index_rebuilds": self.index_rebuilds,
        })
    }
}

/// 排序键：可靠度降序 → 加权延迟升序 → 价格升序 → DID → 槽位。
///
/// 与 [`crate::index`] 里的排序规则**必须**一致，否则有界选择与全排序会给出不同顺序。
pub(crate) fn rank_key(m: &CapabilityMatch) -> (i64, u64, i64, String, usize) {
    (
        -m.score,
        m.capability.effective_latency_ms(),
        m.capability.price_per_unit.get(),
        m.did.as_str().to_string(),
        m.slot,
    )
}

/// 有界 top-k 选择：`k == 0` 或 `k >= 候选数` 时退化为全排序（并计数）。
///
/// 返回值与「全排序后截断到 k」逐位相同——这是优化的**语义不变式**。
pub fn rank_top_k(matches: Vec<CapabilityMatch>, k: usize, perf: &mut QueryPerf) -> Vec<CapabilityMatch> {
    if k == 0 || k >= matches.len() {
        perf.full_sorts += 1;
        let mut all = matches;
        all.sort_by(|a, b| rank_key(a).cmp(&rank_key(b)));
        perf.comparisons += all.len() as u64; // 记账：一次全排序至少 n 次比较
        return all;
    }
    // 有界选择：维护一个大小为 k 的**最大堆**（堆顶是当前最差），
    // 遇到更好的就替换。比较次数 O(n log k)，且不排序整个候选集。
    let mut heap: BinaryHeap<(i64, u64, i64, String, usize)> = BinaryHeap::with_capacity(k + 1);
    let mut keys: Vec<(i64, u64, i64, String, usize)> = Vec::with_capacity(matches.len());
    for m in &matches {
        keys.push(rank_key(m));
    }
    for key in &keys {
        if heap.len() < k {
            heap.push(key.clone());
            continue;
        }
        perf.comparisons += 1;
        match heap.peek() {
            Some(worst) if key < worst => {
                heap.pop();
                heap.push(key.clone());
                perf.comparisons += 1;
            }
            _ => {}
        }
    }
    let mut selected: Vec<(i64, u64, i64, String, usize)> = heap.into_vec();
    selected.sort();
    perf.bounded_selections += 1;
    // 按选中的键取回原始条目（键唯一：DID + 槽位决定一条能力）。
    let mut picked: Vec<CapabilityMatch> = Vec::with_capacity(selected.len());
    for key in &selected {
        if let Some(position) = keys.iter().position(|candidate| candidate == key) {
            if let Some(item) = matches.get(position) {
                picked.push(item.clone());
            }
        }
    }
    picked
}

/// 查询结果缓存的统计。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueryCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub inserts: u64,
    pub evictions: u64,
    /// 因修订号变化而作废的条目数（惰性淘汰）。
    pub invalidated: u64,
}

impl QueryCacheStats {
    pub fn to_value(self) -> serde_json::Value {
        serde_json::json!({
            "hits": self.hits,
            "misses": self.misses,
            "inserts": self.inserts,
            "evictions": self.evictions,
            "invalidated": self.invalidated,
        })
    }
}

/// 一条缓存条目：结果 + 它对应的图修订号。
#[derive(Clone, Debug)]
struct CachedQuery {
    revision: u64,
    result: QueryResult,
    last_used: u64,
}

/// 查询结果缓存：容量有上限、按键精确命中、修订号不匹配即视为未命中。
///
/// 为什么键里带修订号：能力图是**会变**的，缓存必须自己会失效。
/// 把「失效」做成键的一部分，比事后逐个清理更不容易遗漏。
#[derive(Clone, Debug)]
pub struct QueryCache {
    capacity: usize,
    entries: std::collections::BTreeMap<String, CachedQuery>,
    use_seq: u64,
    stats: QueryCacheStats,
}

impl QueryCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: std::collections::BTreeMap::new(),
            use_seq: 0,
            stats: QueryCacheStats::default(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn stats(&self) -> QueryCacheStats {
        self.stats
    }

    /// 命中则返回结果副本；修订号不一致的条目被丢弃并计入 `invalidated`。
    pub fn get(&mut self, key: &str, revision: u64) -> Option<QueryResult> {
        let stale = self
            .entries
            .get(key)
            .map(|entry| entry.revision != revision)
            .unwrap_or(false);
        if stale {
            self.entries.remove(key);
            self.stats.invalidated += 1;
        }
        if self.entries.contains_key(key) {
            self.use_seq += 1;
            let seq = self.use_seq;
            if let Some(entry) = self.entries.get_mut(key) {
                entry.last_used = seq;
            }
            self.stats.hits += 1;
            return self.entries.get(key).map(|entry| entry.result.clone());
        }
        self.stats.misses += 1;
        None
    }

    /// 写入结果；容量为 0 时什么都不做（等价于关闭缓存）。
    pub fn insert(&mut self, key: String, revision: u64, result: QueryResult) {
        if self.capacity == 0 {
            return;
        }
        while self.entries.len() >= self.capacity {
            let victim = self
                .entries
                .iter()
                .min_by(|a, b| {
                    a.1.last_used
                        .cmp(&b.1.last_used)
                        .then_with(|| a.0.cmp(b.0))
                })
                .map(|(k, _)| k.clone());
            match victim {
                Some(key) => {
                    self.entries.remove(&key);
                    self.stats.evictions += 1;
                }
                None => break,
            }
        }
        self.use_seq += 1;
        let last_used = self.use_seq;
        self.entries.insert(
            key,
            CachedQuery {
                revision,
                result,
                last_used,
            },
        );
        self.stats.inserts += 1;
    }

    /// 显式清空（例如图的容量配置变化时）。
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn to_value(&self) -> serde_json::Value {
        serde_json::json!({
            "capacity": self.capacity,
            "entries": self.entries.len(),
            "hits": self.stats.hits,
            "misses": self.stats.misses,
            "inserts": self.stats.inserts,
            "evictions": self.stats.evictions,
            "invalidated": self.stats.invalidated,
        })
    }
}

/// 索引的整表重建（**只在显式修复时使用**，正常读写路径不调用它）。
///
/// 它的存在本身就是证据：既然有一条「重建」的路径，那么「正常路径从未调用它」
/// 就是可断言的（`QueryPerf::index_rebuilds == 0`）。
pub fn rebuild_index(
    index: &mut CapabilityIndex,
    snapshot: &[(au4a_core::Did, Vec<crate::capability::Capability>)],
    perf: &mut QueryPerf,
) {
    for (did, capabilities) in snapshot {
        index.insert_agent(did, capabilities);
    }
    perf.index_rebuilds += 1;
}

/// 索引的键数量（用于不变式检查与证据）。
pub fn index_key_count(index: &CapabilityIndex) -> usize {
    index.skill_entries().len()
}

/// 一个便于测试与证据展示的键类型别名。
pub type CacheKey = (CapKey, String);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{Capability, SkillId};
    use au4a_core::{AgentKeys, Credits, Did};

    fn did(seed: u8) -> Did {
        AgentKeys::from_seed(&[seed; 32]).did()
    }

    fn matches(count: usize) -> Vec<CapabilityMatch> {
        (0..count)
            .map(|index| {
                let seed = (index % 200) as u8;
                CapabilityMatch {
                    did: did(seed),
                    slot: index / 200,
                    score: i64::from(9_000 - (index as u16 % 100)),
                    capability: Capability::new(
                        SkillId::new("translate.en-zh").expect("valid"),
                        Credits((index % 7) as i64 + 1),
                    )
                    .with_latency(10 + (index % 50) as u32, 100 + (index % 90) as u32),
                }
            })
            .collect()
    }

    #[test]
    fn bounded_selection_matches_a_full_sort_exactly() {
        let candidates = matches(300);
        let mut perf = QueryPerf::default();
        let bounded = rank_top_k(candidates.clone(), 5, &mut perf);
        assert_eq!(perf.bounded_selections, 1);
        assert_eq!(perf.full_sorts, 0, "有界选择不做全排序");

        let mut full = candidates;
        full.sort_by(|a, b| rank_key(a).cmp(&rank_key(b)));
        let expected: Vec<CapabilityMatch> = full.into_iter().take(5).collect();
        assert_eq!(bounded.len(), 5);
        assert_eq!(
            bounded.iter().map(|m| rank_key(m)).collect::<Vec<_>>(),
            expected.iter().map(|m| rank_key(m)).collect::<Vec<_>>(),
            "有界选择与全排序逐位一致"
        );
    }

    #[test]
    fn limit_zero_or_oversized_falls_back_to_a_full_sort() {
        let mut perf = QueryPerf::default();
        let all = rank_top_k(matches(10), 0, &mut perf);
        assert_eq!(all.len(), 10);
        assert_eq!(perf.full_sorts, 1);
        let all = rank_top_k(matches(3), 10, &mut perf);
        assert_eq!(all.len(), 3);
        assert_eq!(perf.full_sorts, 2);
        assert_eq!(perf.bounded_selections, 0);
    }

    #[test]
    fn the_cache_is_keyed_by_revision_and_evicts_deterministically() {
        let mut cache = QueryCache::new(2);
        let result = QueryResult::empty(7);
        cache.insert("a".to_string(), 1, result.clone());
        assert!(cache.get("a", 1).is_some());
        assert_eq!(cache.stats().hits, 1);
        // 修订号变了 → 旧条目作废（惰性），并计入 invalidated。
        assert!(cache.get("a", 2).is_none());
        assert_eq!(cache.stats().invalidated, 1);
        assert_eq!(cache.len(), 0);

        cache.insert("a".to_string(), 2, result.clone());
        cache.insert("b".to_string(), 2, result.clone());
        cache.get("a", 2);
        cache.insert("c".to_string(), 2, result);
        assert_eq!(cache.len(), 2, "容量上限");
        assert_eq!(cache.stats().evictions, 1);
        assert!(cache.get("b", 2).is_none(), "最久未用的被淘汰");
        assert!(cache.get("a", 2).is_some());
    }

    #[test]
    fn a_zero_capacity_cache_still_answers_correctly() {
        let mut cache = QueryCache::new(0);
        cache.insert("a".to_string(), 1, QueryResult::empty(3));
        assert_eq!(cache.len(), 0);
        assert!(cache.get("a", 1).is_none());
        assert_eq!(cache.stats().misses, 1);
    }
}
