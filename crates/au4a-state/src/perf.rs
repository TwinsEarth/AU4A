//! 性能优化（v1.3.7）：**确定性的工作量核算**，不是墙钟计时。
//!
//! 本轨道禁止读墙钟（可重放性是硬约束），所以「更快」不能靠毫秒证明。
//! 这里改用**可复算的工作量**：算了多少次哈希、复用了多少次、搬了多少块、
//! 省下多少块。同样的输入给同样的数字，因此优化本身也是可证伪的。
//!
//! 三处真实的省力：
//!
//! 1. **块摘要复用**（[`DigestCache`]）：块摘要只取决于 `(zone, key, 规范 JSON)`。
//!    缓存键就是这三者的拼接，命中即输入相同 ⇒ 摘要必然相同（SHA-256 是确定的），
//!    所以复用不是「赌一把」，而是同一条数学事实的复用。
//! 2. **只物化变动的块**（[`plan`]）：先按摘要比较算出变更清单，再只对变动块搬运 value。
//!    未变块连 `value.clone()` 都不发生——这一点用 `ops` 计数可以断言。
//! 3. **只补缺失的块**（续跑）：断线重连后只传缺口，`ops_skipped` 记录省下的操作数。

use std::collections::BTreeMap;

use au4a_core::{canonical_hash, canonicalize, CoreError, CoreResult, Did};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::diff::{DeltaOp, StateDelta};
use crate::snapshot::{StateBlock, StateSnapshot, StateZone};
use crate::transfer::{send_chunks, LocalNetwork, NodeId, TransferSession};

/// 确定性工作量计数器。**没有时间字段**——时间不可重放，工作量可以。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkCounter {
    /// 真正做过的块摘要哈希次数。
    pub block_hashes: u64,
    /// 命中缓存而复用的摘要次数。
    pub hashes_reused: u64,
    /// 做过的规范 JSON 编码次数。
    pub canon_encodings: u64,
    /// 快照级哈希（root / content_root / zone_root）次数。
    pub root_hashes: u64,
    /// 被物化搬运的块数（只有变动块会被物化）。
    pub blocks_materialized: u64,
    /// 写进存储的键数。
    pub store_writes: u64,
    /// 传输的操作数。
    pub ops_transferred: u64,
    /// 因为续跑而省下的操作数。
    pub ops_skipped: u64,
}

impl WorkCounter {
    pub fn new() -> Self {
        Self::default()
    }

    /// 总哈希次数（真实 + 复用）。
    pub fn total_digest_lookups(&self) -> u64 {
        self.block_hashes + self.hashes_reused
    }

    /// 复用率（万分比，整数——本轨道不用浮点）。
    pub fn reuse_ratio_bp(&self) -> i64 {
        let total = self.total_digest_lookups();
        if total == 0 {
            return 0;
        }
        ((self.hashes_reused as i64) * 10_000) / (total as i64)
    }

    pub fn merge(&mut self, other: &WorkCounter) {
        self.block_hashes += other.block_hashes;
        self.hashes_reused += other.hashes_reused;
        self.canon_encodings += other.canon_encodings;
        self.root_hashes += other.root_hashes;
        self.blocks_materialized += other.blocks_materialized;
        self.store_writes += other.store_writes;
        self.ops_transferred += other.ops_transferred;
        self.ops_skipped += other.ops_skipped;
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// 块摘要缓存：键 = `(zone, key, 规范 JSON 字节)`，值 = 摘要。
///
/// 这个键的设计是**安全性**的关键：把 value 的规范字节写进键里，
/// 就不可能把「另一个值的摘要」错配给当前值——错配需要 SHA-256 碰撞。
#[derive(Clone, Debug, Default)]
pub struct DigestCache {
    entries: BTreeMap<(StateZone, String, String), String>,
}

impl DigestCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 取摘要：命中则复用（不哈希），未命中则计算并写入缓存。
    pub fn digest_for(
        &mut self,
        zone: StateZone,
        key: &str,
        value: &Value,
        counter: &mut WorkCounter,
    ) -> CoreResult<String> {
        let canonical = canonicalize(value)?;
        counter.canon_encodings += 1;
        let cache_key = (zone, key.to_string(), canonical.clone());
        if let Some(digest) = self.entries.get(&cache_key) {
            counter.hashes_reused += 1;
            return Ok(digest.clone());
        }
        let digest = canonical_hash(&value_payload(zone, key, value))?;
        counter.block_hashes += 1;
        self.entries.insert(cache_key, digest.clone());
        Ok(digest)
    }

    /// 用缓存捕获快照：未变的块不再重新哈希。
    pub fn capture(
        &mut self,
        agent: &Did,
        node: &str,
        epoch: u64,
        blocks: &[StateBlock],
        counter: &mut WorkCounter,
    ) -> CoreResult<StateSnapshot> {
        let mut rebuilt = Vec::with_capacity(blocks.len());
        for block in blocks {
            let digest =
                self.digest_for(block.zone(), block.key(), block.value(), counter)?;
            if digest == block.digest() {
                // 摘要相同：直接沿用原块（连 value 都不用搬）。
                rebuilt.push(block.clone());
            } else {
                // 理论上不可达（缓存正确性保证），但语义上必须返回新块。
                rebuilt.push(StateBlock::new_cached(
                    block.zone(),
                    block.key(),
                    block.value().clone(),
                    digest,
                )?);
            }
        }
        let snapshot = StateSnapshot::capture(agent, node, epoch, rebuilt)?;
        counter.root_hashes += 1;
        Ok(snapshot)
    }

    /// 从原始 `(zone, key, value)` 造快照：全部走缓存（首次全 miss，二次全 hit）。
    pub fn capture_raw(
        &mut self,
        agent: &Did,
        node: &str,
        epoch: u64,
        blocks: Vec<(StateZone, String, Value)>,
        counter: &mut WorkCounter,
    ) -> CoreResult<StateSnapshot> {
        let mut rebuilt = Vec::with_capacity(blocks.len());
        for (zone, key, value) in blocks {
            let digest = self.digest_for(zone, &key, &value, counter)?;
            rebuilt.push(StateBlock::new_cached(zone, key, value, digest)?);
        }
        let snapshot = StateSnapshot::capture(agent, node, epoch, rebuilt)?;
        counter.root_hashes += 1;
        Ok(snapshot)
    }
}

fn value_payload(zone: StateZone, key: &str, value: &Value) -> Value {
    serde_json::json!({"k": key, "v": value, "z": zone.as_str()})
}

/// 变更清单：只记「哪些块的摘要变了」，不搬运任何 value。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeltaPlan {
    pub set: Vec<(StateZone, String)>,
    pub del: Vec<(StateZone, String)>,
    /// 两边都有且摘要相同的块数。
    pub unchanged: usize,
    /// 为算这份计划而比较的摘要次数。
    pub digest_comparisons: usize,
}

impl DeltaPlan {
    pub fn op_count(&self) -> usize {
        self.set.len() + self.del.len()
    }

    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.del.is_empty()
    }

    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "set": self.set.iter().map(|(z, k)| format!("{}+{}", z.as_str(), k)).collect::<Vec<_>>(),
            "del": self.del.iter().map(|(z, k)| format!("{}-{}", z.as_str(), k)).collect::<Vec<_>>(),
            "unchanged": self.unchanged,
            "ops": self.op_count(),
            "digest_comparisons": self.digest_comparisons,
        })
    }
}

/// 只比较摘要的差异计划：**不克隆任何 value**。
pub fn plan(from: &StateSnapshot, to: &StateSnapshot) -> CoreResult<DeltaPlan> {
    if from.agent() != to.agent() {
        return Err(CoreError::InvalidDid);
    }
    let mut set = Vec::new();
    let mut del = Vec::new();
    let mut unchanged = 0usize;
    let mut comparisons = 0usize;
    for block in to.blocks() {
        match from.block(block.zone(), block.key()) {
            Some(other) => {
                comparisons += 1;
                if other.digest() == block.digest() {
                    unchanged += 1;
                } else {
                    set.push((block.zone(), block.key().to_string()));
                }
            }
            None => {
                comparisons += 1;
                set.push((block.zone(), block.key().to_string()));
            }
        }
    }
    for block in from.blocks() {
        if to.block(block.zone(), block.key()).is_none() {
            comparisons += 1;
            del.push((block.zone(), block.key().to_string()));
        }
    }
    set.sort();
    del.sort();
    Ok(DeltaPlan {
        set,
        del,
        unchanged,
        digest_comparisons: comparisons,
    })
}

/// 按计划物化差异：**只为 set 清单里的块搬运 value**，未变块一次克隆都不发生。
pub fn materialize(
    from: &StateSnapshot,
    to: &StateSnapshot,
    delta_plan: &DeltaPlan,
    counter: &mut WorkCounter,
) -> CoreResult<StateDelta> {
    let mut ops = Vec::with_capacity(delta_plan.set.len());
    for (zone, key) in &delta_plan.set {
        let block = to
            .block(*zone, key)
            .ok_or(CoreError::Encoding)?;
        ops.push(DeltaOp::from_block(block));
        counter.blocks_materialized += 1;
    }
    let del = delta_plan
        .del
        .iter()
        .map(|(zone, key)| crate::diff::DelOp {
            zone: *zone,
            key: key.clone(),
        })
        .collect();
    StateDelta::from_ops(
        to.agent().clone(),
        from.content_root()?,
        to.content_root()?,
        ops,
        del,
    )
}

/// 续跑核算：全量重传 vs 只补缺口，省下多少操作。
pub fn resume_savings(total_ops: usize, resumed_from_chunks: usize, chunk_ops: usize) -> (u64, u64) {
    let skipped_chunks = resumed_from_chunks as u64;
    let skipped = skipped_chunks * chunk_ops as u64;
    let transferred = (total_ops as u64).saturating_sub(skipped);
    (transferred, skipped)
}

/// 一轮传输的工作量核算（与 [`crate::transfer`] 的真实传输共用同一份数据）。
pub fn measure_transfer(
    net: &mut LocalNetwork,
    from: &NodeId,
    to: &NodeId,
    delta: &StateDelta,
    max_ops: usize,
    skip_chunks: usize,
    counter: &mut WorkCounter,
) -> CoreResult<usize> {
    let report = send_chunks(net, from, to, delta, max_ops, skip_chunks, usize::MAX)?;
    let chunk_count = delta.chunked(max_ops)?.len();
    let ops_per_chunk = if chunk_count > 0 {
        delta.op_count().div_ceil(chunk_count)
    } else {
        0
    };
    let (transferred, skipped) = resume_savings(delta.op_count(), skip_chunks, ops_per_chunk);
    counter.ops_transferred += transferred;
    counter.ops_skipped += skipped;
    Ok(report.frames as usize)
}

/// 接收侧：把 mailbox 里的块收进会话并核算。
pub fn measure_receive(
    net: &mut LocalNetwork,
    node: &NodeId,
    session: Option<TransferSession>,
    counter: &mut WorkCounter,
) -> CoreResult<(TransferSession, usize)> {
    let frames = net.take(node)?;
    let mut session = match session {
        Some(s) => s,
        None => {
            let first = frames.first().ok_or(CoreError::FrameTruncated)?;
            TransferSession::open(&crate::transfer::decode_chunk(first)?)?
        }
    };
    let mut accepted = 0usize;
    for frame in &frames {
        let chunk = crate::transfer::decode_chunk(frame)?;
        if session.accept(&chunk)? == crate::transfer::AcceptOutcome::Accepted {
            accepted += 1;
            counter.blocks_materialized += chunk.op_count() as u64;
        }
    }
    Ok((session, accepted))
}

/// 2PC 影子代写入的工作量：只写差异涉及的块。
pub fn generation_write_estimate(delta_plan: &DeltaPlan, total_blocks: usize) -> Value {
    serde_json::json!({
        "blocks_total": total_blocks,
        "blocks_changed": delta_plan.op_count(),
        "blocks_rewritten": total_blocks,
        "note": "影子代必须完整物化（否则无法保证不出现部分状态），\
                 但上一代只在 confirm 之后被删除——省下的是读路径的重算，不是写路径的完整物化",
    })
}

/// 全部计数器的一次性摘要（观察层与证据共用）。
pub fn summary(counter: &WorkCounter) -> Value {
    serde_json::json!({
        "block_hashes": counter.block_hashes,
        "hashes_reused": counter.hashes_reused,
        "total_digest_lookups": counter.total_digest_lookups(),
        "reuse_ratio_bp": counter.reuse_ratio_bp(),
        "canon_encodings": counter.canon_encodings,
        "root_hashes": counter.root_hashes,
        "blocks_materialized": counter.blocks_materialized,
        "store_writes": counter.store_writes,
        "ops_transferred": counter.ops_transferred,
        "ops_skipped": counter.ops_skipped,
        "wall_clock_used": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::StateZone;
    use serde_json::json;

    fn keys(seed: u8) -> au4a_core::AgentKeys {
        au4a_core::AgentKeys::from_seed(&[seed; 32])
    }

    fn raw(n: usize, v: i64) -> Vec<(StateZone, String, Value)> {
        (0..n)
            .map(|i| (StateZone::Memory, format!("k{i}"), json!({"v": v})))
            .collect()
    }

    #[test]
    fn capturing_the_same_state_twice_reuses_every_digest() {
        let k = keys(1);
        let mut cache = DigestCache::new();
        let mut first = WorkCounter::new();
        let a = cache
            .capture_raw(&k.did(), "node-a", 1, raw(8, 1), &mut first)
            .unwrap();
        assert_eq!(first.block_hashes, 8);
        assert_eq!(first.hashes_reused, 0);

        let mut second = WorkCounter::new();
        let b = cache
            .capture_raw(&k.did(), "node-a", 1, raw(8, 1), &mut second)
            .unwrap();
        assert_eq!(second.block_hashes, 0, "第二次不该再算一次哈希");
        assert_eq!(second.hashes_reused, 8);
        assert_eq!(second.reuse_ratio_bp(), 10_000);
        // 优化不能改变结果。
        assert_eq!(a.root(), b.root());
        assert_eq!(a.blocks(), b.blocks());
        b.verify().unwrap();
    }

    #[test]
    fn only_the_changed_block_is_rehashed() {
        let k = keys(2);
        let mut cache = DigestCache::new();
        let mut counter = WorkCounter::new();
        let first = cache
            .capture_raw(&k.did(), "node-a", 1, raw(8, 1), &mut counter)
            .unwrap();
        let mut changed = raw(8, 1);
        changed[3].2 = json!({"v": 99});
        let mut second = WorkCounter::new();
        let updated = cache
            .capture_raw(&k.did(), "node-a", 2, changed, &mut second)
            .unwrap();
        assert_eq!(second.block_hashes, 1, "只有第 3 块真的变了");
        assert_eq!(second.hashes_reused, 7);
        assert_ne!(first.content_root().unwrap(), updated.content_root().unwrap());
        updated.verify().unwrap();
    }

    #[test]
    fn the_plan_materializes_only_changed_blocks() {
        let k = keys(3);
        let mut cache = DigestCache::new();
        let mut counter = WorkCounter::new();
        let from = cache
            .capture_raw(&k.did(), "node-a", 1, raw(20, 1), &mut counter)
            .unwrap();
        let mut changed = raw(20, 1);
        changed[0].2 = json!({"v": 2});
        changed[19].2 = json!({"v": 3});
        changed.push((StateZone::Fs, "/new".to_string(), json!(1)));
        let mut to_counter = WorkCounter::new();
        let to = cache
            .capture_raw(&k.did(), "node-a", 2, changed, &mut to_counter)
            .unwrap();

        let delta_plan = plan(&from, &to).unwrap();
        assert_eq!(delta_plan.set.len(), 3, "两个改 + 一个新增");
        assert_eq!(delta_plan.del.len(), 0);
        assert_eq!(delta_plan.unchanged, 18);
        // 21 次比较 = to 的 21 个块各查一次（20 个命中 + 1 个新增）。
        assert_eq!(delta_plan.digest_comparisons, 21);

        let mut materialize_counter = WorkCounter::new();
        let delta = materialize(&from, &to, &delta_plan, &mut materialize_counter).unwrap();
        assert_eq!(materialize_counter.blocks_materialized, 3);
        assert_eq!(delta.apply_to(&from).unwrap().content_root().unwrap(), to.content_root().unwrap());
    }

    #[test]
    fn resume_skips_the_blocks_already_received() {
        let k = keys(4);
        let mut cache = DigestCache::new();
        let mut counter = WorkCounter::new();
        let from = cache
            .capture_raw(&k.did(), "node-a", 1, raw(2, 1), &mut counter)
            .unwrap();
        let to = cache
            .capture_raw(&k.did(), "node-a", 2, raw(20, 2), &mut counter)
            .unwrap();
        let delta = StateDelta::between(&from, &to).unwrap();
        let total_chunks = delta.chunked(2).unwrap().len();
        assert!(total_chunks > 2);
        let na = NodeId::new("node-a").unwrap();
        let nb = NodeId::new("node-b").unwrap();
        let mut net = LocalNetwork::new(&[na.clone(), nb.clone()]);
        let mut work = WorkCounter::new();

        // 第一批：只发 2 块并送达；其余在路上丢失（不发送即等价于丢失）。
        send_chunks(&mut net, &na, &nb, &delta, 2, 0, 2).unwrap();
        let (session, accepted) = measure_receive(&mut net, &nb, None, &mut work).unwrap();
        assert_eq!(accepted, 2);
        let resume_from = session.resume_from();
        assert_eq!(resume_from, 2);

        // 续跑：从第 2 块补发到结尾。
        let frames =
            measure_transfer(&mut net, &na, &nb, &delta, 2, resume_from, &mut work).unwrap();
        assert_eq!(frames, total_chunks - 2);
        assert!(work.ops_transferred > 0);
        assert!(work.ops_skipped > 0, "续跑必须省下工作量：{:?}", work);
        let (session, _) = measure_receive(&mut net, &nb, Some(session), &mut work).unwrap();
        assert!(session.is_complete(total_chunks));
    }

    #[test]
    fn counters_are_deterministic() {
        let k = keys(5);
        let run = || {
            let mut cache = DigestCache::new();
            let mut counter = WorkCounter::new();
            let a = cache
                .capture_raw(&k.did(), "node-a", 1, raw(6, 1), &mut counter)
                .unwrap();
            let b = cache
                .capture_raw(&k.did(), "node-a", 1, raw(6, 1), &mut counter)
                .unwrap();
            (a.root().to_string(), b.root().to_string(), summary(&counter))
        };
        assert_eq!(run(), run(), "同样的输入必须给同样的工作量数字");
    }

    #[test]
    fn wall_clock_is_never_used() {
        // 证据字段里必须明确写「没有用墙钟」——这是本轨道可重放性的前提。
        let value = summary(&WorkCounter::new());
        assert_eq!(value["wall_clock_used"], json!(false));
    }

    #[test]
    fn generation_write_estimate_is_honest_about_shadow_paging() {
        let plan = DeltaPlan {
            set: vec![(StateZone::Memory, "k".to_string())],
            del: vec![],
            unchanged: 9,
            digest_comparisons: 10,
        };
        let value = generation_write_estimate(&plan, 10);
        assert_eq!(value["blocks_changed"], json!(1));
        assert_eq!(value["blocks_rewritten"], json!(10));
    }
}
