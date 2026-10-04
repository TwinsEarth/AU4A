//! 状态存储抽象（v1.3.1）：[`StateStore`] trait + 本地内存实现。
//!
//! 分布式存储在主网里是「谁背书、谁复制、谁付费」的问题；在这一版里我们只冻结**语义**：
//! `put / get / list / remove` 四个动作 + 确定性顺序。trait 是给未来换实现用的
//! （UDOS 的分布式存储、内容寻址对象层、真实的冗余副本），而不是给人类操作面板用的。
//!
//! 快照落库用复合键 `"<zone>:<key>"`，可以加前缀（v1.3.4 的两阶段提交就用前缀把
//! 「暂存区」和「生效区」隔开，从而保证目标节点不会留下部分状态）。

use std::cell::Cell;
use std::collections::BTreeMap;

use au4a_core::{canonicalize, CoreError, CoreResult, Did};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::snapshot::{StateBlock, StateSnapshot, StateZone};

/// 复合键的分隔符：`fs:/work/a.txt`。
pub const ZONE_SEP: char = ':';

/// 存储计数器（v1.3.7 的性能断言读它，而不是读墙钟）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreCounters {
    pub puts: u64,
    pub gets: u64,
    /// `get` 命中次数。
    pub hits: u64,
    pub removes: u64,
    pub lists: u64,
}

/// 分布式状态存储的最小契约。
///
/// 所有实现必须满足：
/// 1. `put` 之后 `get` 返回同一个值（规范 JSON 意义下相等）。
/// 2. `list(prefix)` 返回按字节序升序、且只含以 `prefix` 开头的键。
/// 3. `remove` 返回「此前是否存在」，对不存在的键不报错（幂等）。
pub trait StateStore {
    fn put(&mut self, key: &str, value: Value) -> CoreResult<()>;
    fn get(&self, key: &str) -> CoreResult<Option<Value>>;
    fn list(&self, prefix: &str) -> CoreResult<Vec<String>>;
    fn remove(&mut self, key: &str) -> CoreResult<bool>;
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// 本地内存实现：确定性（BTreeMap 字节序）、无 I/O、无网络、无锁。
///
/// 计数器放在 `Cell` 里，这样读路径（`get`/`list`）保持 `&self`——
/// 存储的读接口不该要求独占借用，否则「多个只读观察者」就成了空话。
#[derive(Clone, Debug, Default)]
pub struct MemoryStore {
    entries: BTreeMap<String, Value>,
    counters: Cell<StoreCounters>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn counters(&self) -> StoreCounters {
        self.counters.get()
    }

    /// 全部键（升序）。给一致性检查用，不走 `list` 的计数器。
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(|k| k.as_str())
    }
}

impl StateStore for MemoryStore {
    fn put(&mut self, key: &str, value: Value) -> CoreResult<()> {
        validate_store_key(key)?;
        canonicalize(&value)?; // 浮点在这里被拒，避免不可规范化的值进存储
        self.entries.insert(key.to_string(), value);
        self.bump(|c| c.puts += 1);
        Ok(())
    }

    fn get(&self, key: &str) -> CoreResult<Option<Value>> {
        validate_store_key(key)?;
        match self.entries.get(key) {
            Some(v) => {
                self.bump(|c| {
                    c.gets += 1;
                    c.hits += 1;
                });
                Ok(Some(v.clone()))
            }
            None => {
                self.bump(|c| c.gets += 1);
                Ok(None)
            }
        }
    }

    fn list(&self, prefix: &str) -> CoreResult<Vec<String>> {
        self.bump(|c| c.lists += 1);
        Ok(self
            .entries
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect())
    }

    fn remove(&mut self, key: &str) -> CoreResult<bool> {
        validate_store_key(key)?;
        self.bump(|c| c.removes += 1);
        Ok(self.entries.remove(key).is_some())
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

impl MemoryStore {
    fn bump(&self, f: impl FnOnce(&mut StoreCounters)) {
        let mut counters = self.counters.get();
        f(&mut counters);
        self.counters.set(counters);
    }

    pub fn reset_counters(&mut self) {
        self.counters.set(StoreCounters::default());
    }
}

fn validate_store_key(key: &str) -> CoreResult<()> {
    if key.is_empty() || key.len() > crate::snapshot::MAX_KEY_LEN * 2 {
        return Err(CoreError::Encoding);
    }
    if key.chars().any(|c| (c as u32) < 0x20) {
        return Err(CoreError::Encoding);
    }
    Ok(())
}

/// 复合键：`<prefix><zone>:<key>`。
pub fn store_key(prefix: &str, zone: StateZone, key: &str) -> String {
    format!("{prefix}{}{ZONE_SEP}{key}", zone.as_str())
}

/// 反解复合键；前缀不匹配或区名非法时返回 `None`。
pub fn split_store_key<'a>(prefix: &str, key: &'a str) -> Option<(StateZone, &'a str)> {
    let rest = key.strip_prefix(prefix)?;
    let (zone, name) = rest.split_once(ZONE_SEP)?;
    let zone = StateZone::parse(zone)?;
    if name.is_empty() {
        return None;
    }
    Some((zone, name))
}

/// 把整份快照写入存储（写 `prefix` 命名空间）；返回写入块数。
pub fn write_snapshot<S: StateStore + ?Sized>(
    store: &mut S,
    prefix: &str,
    snap: &StateSnapshot,
) -> CoreResult<usize> {
    for block in snap.blocks() {
        store.put(
            &store_key(prefix, block.zone(), block.key()),
            block.value().clone(),
        )?;
    }
    Ok(snap.blocks().len())
}

/// 从存储重建快照。任何前缀内不合法的键都视为**损坏**并被拒（不静默丢状态）。
pub fn read_snapshot<S: StateStore + ?Sized>(
    store: &S,
    prefix: &str,
    agent: &Did,
    source_node: &str,
    epoch: u64,
) -> CoreResult<StateSnapshot> {
    let keys = store.list(prefix)?;
    let mut blocks = Vec::with_capacity(keys.len());
    for key in keys {
        let (zone, name) = split_store_key(prefix, &key).ok_or(CoreError::Encoding)?;
        let value = store.get(&key)?.ok_or(CoreError::Encoding)?;
        blocks.push(StateBlock::new(zone, name, value)?);
    }
    StateSnapshot::capture(agent, source_node, epoch, blocks)
}

/// 删除一个命名空间下的全部键；返回删除条数。
pub fn clear_namespace<S: StateStore + ?Sized>(store: &mut S, prefix: &str) -> CoreResult<usize> {
    let keys = store.list(prefix)?;
    let mut removed = 0;
    for key in keys {
        if store.remove(&key)? {
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> StateSnapshot {
        let did = au4a_core::AgentKeys::from_seed(&[9u8; 32]).did();
        let blocks = vec![
            StateBlock::new(StateZone::Fs, "/w/a", json!({"n": 1})).unwrap(),
            StateBlock::new(StateZone::Memory, "m1", json!([1, 2, 3])).unwrap(),
            StateBlock::new(StateZone::Context, "goal", json!("move")).unwrap(),
        ];
        StateSnapshot::capture(&did, "node-a", 4, blocks).unwrap()
    }

    #[test]
    fn put_get_remove_are_consistent() {
        let mut store = MemoryStore::new();
        store.put("k", json!({"a": 1})).unwrap();
        assert_eq!(store.get("k").unwrap(), Some(json!({"a": 1})));
        assert!(store.remove("k").unwrap());
        assert!(!store.remove("k").unwrap());
        assert_eq!(store.get("k").unwrap(), None);
        assert!(store.is_empty());
    }

    #[test]
    fn list_is_prefix_filtered_and_sorted() {
        let mut store = MemoryStore::new();
        store.put("fs:/b", json!(1)).unwrap();
        store.put("fs:/a", json!(2)).unwrap();
        store.put("memory:x", json!(3)).unwrap();
        assert_eq!(store.list("fs:").unwrap(), vec!["fs:/a", "fs:/b"]);
        assert_eq!(store.list("").unwrap().len(), 3);
        assert_eq!(store.list("nope").unwrap().len(), 0);
    }

    #[test]
    fn floats_never_enter_the_store() {
        let mut store = MemoryStore::new();
        assert_eq!(store.put("k", json!(1.25)), Err(CoreError::FloatForbidden));
        assert!(store.is_empty());
    }

    #[test]
    fn snapshot_roundtrip_through_a_namespace() {
        let snap = sample();
        let mut store = MemoryStore::new();
        assert_eq!(write_snapshot(&mut store, "live:", &snap).unwrap(), 3);
        // 同一出处（node + epoch）读回：连文档 root 都一致。
        let same = read_snapshot(
            &store,
            "live:",
            snap.agent(),
            snap.source_node(),
            snap.epoch(),
        )
        .unwrap();
        assert_eq!(same.root(), snap.root());
        // 换到别的节点读回：状态内容必须一致，root 因出处不同而不同（这正是迁移的语义）。
        let moved = read_snapshot(&store, "live:", snap.agent(), "node-b", 99).unwrap();
        assert_eq!(moved.content_root().unwrap(), snap.content_root().unwrap());
        assert_ne!(moved.root(), snap.root());
        assert_eq!(clear_namespace(&mut store, "live:").unwrap(), 3);
        assert!(store.is_empty());
    }

    #[test]
    fn corrupted_namespace_key_is_refused_not_skipped() {
        let snap = sample();
        let mut store = MemoryStore::new();
        write_snapshot(&mut store, "live:", &snap).unwrap();
        store.put("live:bogus:/x", json!(1)).unwrap();
        assert_eq!(
            read_snapshot(&store, "live:", snap.agent(), "node-b", 1),
            Err(CoreError::Encoding)
        );
    }

    #[test]
    fn counters_count_real_operations() {
        let mut store = MemoryStore::new();
        store.put("a", json!(1)).unwrap();
        let _ = store.get("a").unwrap();
        let _ = store.get("b").unwrap();
        let c = store.counters();
        assert_eq!(c.puts, 1);
        assert_eq!(c.gets, 2);
        assert_eq!(c.hits, 1);
    }
}
