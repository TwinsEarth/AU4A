//! v1.1.4 —— 邻居能力缓存。
//!
//! Agent 不可能无限记住每个邻居的能力：邻居会离开、会沉默、会改名换姓地重启。
//! 一个没有上限的「网络全局能力表」正是参考项目 `MarketAgentCard` 那类设计的隐含假设，
//! 而它在真实网络里等于内存泄漏 + 陈旧路由。
//!
//! 这一层的三条硬性质：
//!
//! 1. **容量上限是硬的**：`len() <= capacity` 在任何操作后都成立；满了就按
//!    **最近最少使用**（LRU）淘汰，而不是拒绝新信息——拒绝新信息会让图永远停在旧世界。
//! 2. **失效是显式的**：TTL 用**逻辑刻**（不读墙钟），过期条目在 `get` / `insert` /
//!    `expire` 时被清掉；另有 `invalidate` 用于「我确定它错了」的场景。
//! 3. **淘汰顺序是确定的**：LRU 以 `(last_used, did)` 为序，`did` 是稳定 tie-break。
//!    同样的操作序列在任何节点上淘汰同一个条目——否则「缓存」会变成不可复现的状态源。
//!
//! v1.1.8 会把这里的 O(n) 淘汰换成有序索引上的 O(log n)，但**语义不变**：
//! 那是优化，不是行为变更，测试会比较两者结果相同。

use std::collections::BTreeMap;

use au4a_core::Did;

use crate::capability::SkillId;
use crate::graph::NeighborRecord;

/// 缓存命中/淘汰统计。性能证据用**操作计数**而不是墙钟（本 crate 禁止读钟）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub inserts: u64,
    pub updates: u64,
    pub evictions: u64,
    pub expirations: u64,
    pub invalidations: u64,
}

impl CacheStats {
    pub fn to_value(self) -> serde_json::Value {
        serde_json::json!({
            "hits": self.hits,
            "misses": self.misses,
            "inserts": self.inserts,
            "updates": self.updates,
            "evictions": self.evictions,
            "expirations": self.expirations,
            "invalidations": self.invalidations,
        })
    }
}

/// 一次写入的结果。淘汰与过期都被显式报告，而不是静默发生。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CacheInsert {
    /// 容量为 0 时写入被拒（这是唯一的「拒绝新信息」情形）。
    pub accepted: bool,
    /// 覆盖了同一个 Agent 的旧记录。
    pub replaced: bool,
    /// 本次写入前被 TTL 清掉的条目。
    pub expired: Vec<Did>,
    /// 本次写入为腾出容量而淘汰的条目（按淘汰顺序）。
    pub evicted: Vec<Did>,
}

/// 缓存槽位：记录 + LRU 使用序号。
#[derive(Clone, Debug, PartialEq, Eq)]
struct Slot {
    record: NeighborRecord,
    last_used: u64,
}

/// 邻居能力缓存。容量有上限，条目会过期，淘汰是确定性的。
#[derive(Clone, Debug)]
pub struct CapabilityCache {
    capacity: usize,
    ttl_ticks: u64,
    slots: BTreeMap<Did, Slot>,
    use_seq: u64,
    stats: CacheStats,
}

impl CapabilityCache {
    /// `capacity == 0` 表示「不缓存」：所有写入被拒（`CacheInsert::accepted == false`）。
    /// `ttl_ticks == 0` 表示不过期。
    pub fn new(capacity: usize, ttl_ticks: u64) -> Self {
        Self {
            capacity,
            ttl_ticks,
            slots: BTreeMap::new(),
            use_seq: 0,
            stats: CacheStats::default(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn ttl_ticks(&self) -> u64 {
        self.ttl_ticks
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.slots.len() >= self.capacity
    }

    pub fn stats(&self) -> CacheStats {
        self.stats
    }

    /// 条目是否已过期（`at + ttl < now`）。`ttl == 0` 表示永不过期。
    fn is_expired(&self, slot: &Slot, now: u64) -> bool {
        self.ttl_ticks != 0 && now > slot.record.at.saturating_add(self.ttl_ticks)
    }

    /// 条目是否存在且未过期（不做清理、不触碰 LRU）。
    pub fn is_live(&self, did: &Did, now: u64) -> bool {
        self.slots
            .get(did)
            .map(|slot| !self.is_expired(slot, now))
            .unwrap_or(false)
    }

    /// 命中并「触碰」LRU（`get` 的语义：用过就算最近使用）。
    pub fn get(&mut self, did: &Did, now: u64) -> Option<&NeighborRecord> {
        let expired = self
            .slots
            .get(did)
            .map(|slot| self.is_expired(slot, now))
            .unwrap_or(false);
        if expired {
            self.slots.remove(did);
            self.stats.expirations += 1;
            self.stats.misses += 1;
            return None;
        }
        if self.slots.contains_key(did) {
            self.use_seq += 1;
            let seq = self.use_seq;
            if let Some(slot) = self.slots.get_mut(did) {
                slot.last_used = seq;
            }
            self.stats.hits += 1;
            return self.slots.get(did).map(|slot| &slot.record);
        }
        self.stats.misses += 1;
        None
    }

    /// 只看不碰：不计命中、不更新 LRU、不清理过期条目。
    pub fn peek(&self, did: &Did) -> Option<&NeighborRecord> {
        self.slots.get(did).map(|slot| &slot.record)
    }

    /// 全部槽位（含已过期但尚未清理的）。只读迭代，不改变任何计数。
    pub fn iter(&self) -> impl Iterator<Item = &NeighborRecord> {
        self.slots.values().map(|slot| &slot.record)
    }

    /// 当前存活（未过期）的条目数。
    pub fn live_len(&self, now: u64) -> usize {
        self.slots
            .values()
            .filter(|slot| !self.is_expired(slot, now))
            .count()
    }

    /// LRU 顺序（最久未用在前），用于证据与调试。确定性：`(last_used, did)` 排序。
    pub fn lru_order(&self) -> Vec<Did> {
        let mut pairs: Vec<(u64, &Did)> = self
            .slots
            .values()
            .map(|s| (s.last_used, &s.record.did))
            .collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
        pairs.into_iter().map(|(_, did)| did.clone()).collect()
    }

    /// 写入一条记录。满了就淘汰 LRU；容量为 0 直接拒绝。
    pub fn insert(&mut self, record: NeighborRecord, now: u64) -> CacheInsert {
        let mut outcome = CacheInsert::default();
        if self.capacity == 0 {
            return outcome;
        }
        outcome.expired = self.expire(now);
        let did = record.did.clone();
        let replaced = self.slots.contains_key(&did);
        if replaced {
            self.stats.updates += 1;
        } else {
            self.stats.inserts += 1;
            while self.slots.len() >= self.capacity {
                match self.evict_lru() {
                    Some(victim) => outcome.evicted.push(victim),
                    None => break,
                }
            }
        }
        self.use_seq += 1;
        let last_used = self.use_seq;
        self.slots.insert(did, Slot { record, last_used });
        outcome.accepted = true;
        outcome.replaced = replaced;
        outcome
    }

    /// 淘汰最近最少使用的条目（`(last_used, did)` 最小者）。
    ///
    /// O(n) 扫描是刻意的起步实现：先让语义与测试成立，v1.1.8 再换成有序索引，
    /// 并由测试断言两者给出**同一个**淘汰结果。
    pub fn evict_lru(&mut self) -> Option<Did> {
        let victim = self
            .slots
            .values()
            .min_by(|a, b| {
                a.last_used
                    .cmp(&b.last_used)
                    .then_with(|| a.record.did.cmp(&b.record.did))
            })
            .map(|slot| slot.record.did.clone())?;
        self.slots.remove(&victim);
        self.stats.evictions += 1;
        Some(victim)
    }

    /// 清理过期条目，返回被清理的 DID（按字典序，确定性）。
    pub fn expire(&mut self, now: u64) -> Vec<Did> {
        let victims: Vec<Did> = self
            .slots
            .values()
            .filter(|slot| self.is_expired(slot, now))
            .map(|slot| slot.record.did.clone())
            .collect();
        for did in &victims {
            self.slots.remove(did);
        }
        self.stats.expirations += victims.len() as u64;
        victims
    }

    /// 显式失效：调用方知道这条记录错了（例如对端退出、版本冲突）。
    pub fn invalidate(&mut self, did: &Did) -> bool {
        let removed = self.slots.remove(did).is_some();
        if removed {
            self.stats.invalidations += 1;
        }
        removed
    }

    pub fn clear(&mut self) {
        self.slots.clear();
    }

    /// 视图里的技能集合（只读，升序去重）。
    pub fn skills(&self) -> Vec<&SkillId> {
        let mut skills: Vec<&SkillId> = Vec::new();
        for record in self.iter() {
            for cap in &record.capabilities {
                if !skills.contains(&&cap.skill) {
                    skills.push(&cap.skill);
                }
            }
        }
        skills.sort();
        skills
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{Capability, SkillId};
    use au4a_core::{AgentKeys, Credits};

    fn did(seed: u8) -> Did {
        AgentKeys::from_seed(&[seed; 32]).did()
    }

    fn record(seed: u8, epoch: u64, at: u64, skill: &str) -> NeighborRecord {
        NeighborRecord {
            did: did(seed),
            epoch,
            at,
            fingerprint: format!("fp-{seed}-{epoch}"),
            capabilities: vec![Capability::new(
                SkillId::new(skill).expect("valid"),
                Credits(2),
            )],
        }
    }

    #[test]
    fn capacity_is_a_hard_bound_with_lru_eviction() {
        let mut cache = CapabilityCache::new(2, 0);
        assert!(cache.insert(record(1, 1, 0, "a"), 0).accepted);
        assert!(cache.insert(record(2, 1, 0, "b"), 0).accepted);
        assert_eq!(cache.len(), 2);
        assert!(cache.is_full());
        // 用一下 1 号，使 2 号成为最久未用。
        assert!(cache.get(&did(1), 0).is_some());
        let outcome = cache.insert(record(3, 1, 0, "c"), 0);
        assert_eq!(outcome.evicted, vec![did(2)], "淘汰最久未用者");
        assert_eq!(cache.len(), 2, "容量上限不变式");
        assert!(cache.peek(&did(2)).is_none());
        assert!(cache.peek(&did(1)).is_some());
        assert!(cache.peek(&did(3)).is_some());
        assert_eq!(cache.stats().evictions, 1);
    }

    #[test]
    fn ttl_expiry_uses_logical_ticks_only() {
        let mut cache = CapabilityCache::new(4, 10);
        cache.insert(record(1, 1, 100, "a"), 100);
        assert!(cache.get(&did(1), 110).is_some(), "at+ttl 仍未过期");
        assert!(cache.get(&did(1), 111).is_none(), "超过 at+ttl 立刻过期");
        assert_eq!(cache.stats().expirations, 1);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn expiry_sweep_is_deterministic_and_reported() {
        let mut cache = CapabilityCache::new(8, 5);
        for seed in [3u8, 1, 2] {
            cache.insert(record(seed, 1, 0, "a"), 0);
        }
        // 报告顺序是 DID 的字典序（BTreeMap 迭代序），与插入顺序无关。
        let mut expected = vec![did(1), did(2), did(3)];
        expected.sort();
        assert_eq!(cache.expire(6), expected);
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.stats().expirations, 3);
    }

    #[test]
    fn zero_capacity_refuses_writes_and_full_cache_does_not() {
        let mut zero = CapabilityCache::new(0, 0);
        let outcome = zero.insert(record(1, 1, 0, "a"), 0);
        assert!(!outcome.accepted);
        assert_eq!(zero.len(), 0);
        assert!(zero.is_full(), "容量 0 永远处于满状态");

        let mut one = CapabilityCache::new(1, 0);
        for seed in 1..5u8 {
            assert!(
                one.insert(record(seed, 1, 0, "a"), 0).accepted,
                "no capacity => evict, not refuse"
            );
        }
        assert_eq!(one.len(), 1);
        // 第 1 条不淘汰，之后 3 条各淘汰 1 条。
        assert_eq!(one.stats().evictions, 3);
    }

    #[test]
    fn lru_order_and_invalidation_are_observable() {
        let mut cache = CapabilityCache::new(4, 0);
        cache.insert(record(1, 1, 0, "a"), 0);
        cache.insert(record(2, 1, 0, "b"), 0);
        cache.insert(record(3, 1, 0, "c"), 0);
        cache.get(&did(2), 0);
        assert_eq!(cache.lru_order(), vec![did(1), did(3), did(2)]);
        assert!(cache.invalidate(&did(1)));
        assert!(!cache.invalidate(&did(1)), "重复失效是无操作");
        assert_eq!(cache.stats().invalidations, 1);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn peek_does_not_count_as_a_hit_but_get_does() {
        let mut cache = CapabilityCache::new(2, 0);
        cache.insert(record(1, 1, 0, "a"), 0);
        assert!(cache.peek(&did(1)).is_some());
        assert_eq!(cache.stats().hits, 0);
        assert!(cache.get(&did(1), 0).is_some());
        assert_eq!(cache.stats().hits, 1);
        assert!(cache.get(&did(9), 0).is_none());
        assert_eq!(cache.stats().misses, 1);
    }

    #[test]
    fn replacing_a_record_does_not_consume_capacity() {
        let mut cache = CapabilityCache::new(1, 0);
        cache.insert(record(1, 1, 0, "a"), 0);
        let outcome = cache.insert(record(1, 2, 5, "b"), 5);
        assert!(outcome.accepted && outcome.replaced);
        assert!(outcome.evicted.is_empty());
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.peek(&did(1)).expect("present").epoch, 2);
        assert_eq!(cache.stats().updates, 1);
        assert_eq!(
            cache
                .skills()
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>(),
            vec!["b"]
        );
    }
}
