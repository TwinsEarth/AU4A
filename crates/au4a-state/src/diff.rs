//! 块级增量 diff（v1.3.2）：三区 `set / del`，可切块、可续跑。
//!
//! 迁移一个长期运行的 Agent 不可能每次搬运全部状态：上下文区每天都在长，
//! 文件系统区可能以 GiB 计。所以传输的单位是**差异**，不是快照。
//!
//! 差异的定义刻意做得很小：
//!
//! * `set`：目标上不存在、或摘要不同的块（整块搬运，不做字节级 patch——见下）。
//! * `del`：目标上有、源上已经没有的块。
//!
//! 为什么是**块级**而不是字节级：字节级 patch 需要双方对同一份 base 字节，
//! 而 Agent 的状态是 JSON 语义的；块级 diff 让「应用顺序」「重复应用」都变成幂等的，
//! 从而天然支持**续跑**（收到一半断了，重发那几块即可）。
//!
//! 差异自证：delta 携带 `from_root -> to_root`，应用方在应用前必须能验证
//! 「我的 base 确实是 from」，应用后必须得到 `to_content_root`。否则拒绝。

use au4a_core::{canonical_hash, CoreError, CoreResult, Did};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

use crate::snapshot::{StateBlock, StateSnapshot, StateZone};

/// 单个块级操作的 `set` 记录。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeltaOp {
    pub zone: StateZone,
    pub key: String,
    /// 目标块的内容。
    pub value: Value,
    /// 目标块的摘要（= `StateBlock::digest`，冗余携带以便接收方在应用前就校验）。
    pub digest: String,
}

impl DeltaOp {
    pub fn from_block(block: &StateBlock) -> Self {
        Self {
            zone: block.zone(),
            key: block.key().to_string(),
            value: block.value().clone(),
            digest: block.digest().to_string(),
        }
    }

    /// 校验这一条记录是否自洽：重算摘要必须等于携带的摘要。
    pub fn verify(&self) -> CoreResult<()> {
        let block = StateBlock::new(self.zone, self.key.clone(), self.value.clone())?;
        if block.digest() == self.digest {
            Ok(())
        } else {
            Err(CoreError::InvalidSignature)
        }
    }
}

/// 一个 `del` 记录：只需要定位。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelOp {
    pub zone: StateZone,
    pub key: String,
}

/// 源快照 → 目标快照的块级差异。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StateDelta {
    agent: Did,
    from_content_root: String,
    to_content_root: String,
    set: Vec<DeltaOp>,
    del: Vec<DelOp>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StateDeltaWire {
    agent: Did,
    from_content_root: String,
    to_content_root: String,
    set: Vec<DeltaOp>,
    del: Vec<DelOp>,
}

impl StateDelta {
    /// 计算 `from -> to` 的差异；两者必须属于同一个 Agent。
    pub fn between(from: &StateSnapshot, to: &StateSnapshot) -> CoreResult<Self> {
        if from.agent() != to.agent() {
            return Err(CoreError::InvalidDid);
        }
        let mut set = Vec::new();
        for block in to.blocks() {
            let same = from
                .block(block.zone(), block.key())
                .map(|b| b.digest() == block.digest())
                .unwrap_or(false);
            if !same {
                set.push(DeltaOp::from_block(block));
            }
        }
        let mut del = Vec::new();
        for block in from.blocks() {
            if to.block(block.zone(), block.key()).is_none() {
                del.push(DelOp {
                    zone: block.zone(),
                    key: block.key().to_string(),
                });
            }
        }
        set.sort_by(|a, b| {
            a.zone
                .index()
                .cmp(&b.zone.index())
                .then_with(|| a.key.as_bytes().cmp(b.key.as_bytes()))
        });
        del.sort();
        Ok(Self {
            agent: to.agent().clone(),
            from_content_root: from.content_root()?,
            to_content_root: to.content_root()?,
            set,
            del,
        })
    }

    /// 由已排序的 set/del 重新组装（续跑时把分块拼回一份完整差异）。
    /// 组装结果必须通过 [`StateDelta::validate`]，否则拒绝——拼错的差异不能进入应用阶段。
    pub fn from_ops(
        agent: Did,
        from_content_root: impl Into<String>,
        to_content_root: impl Into<String>,
        set: Vec<DeltaOp>,
        del: Vec<DelOp>,
    ) -> CoreResult<Self> {
        let delta = Self {
            agent,
            from_content_root: from_content_root.into(),
            to_content_root: to_content_root.into(),
            set,
            del,
        };
        delta.validate()?;
        Ok(delta)
    }

    pub fn agent(&self) -> &Did {
        &self.agent
    }

    pub fn from_content_root(&self) -> &str {
        &self.from_content_root
    }

    pub fn to_content_root(&self) -> &str {
        &self.to_content_root
    }

    pub fn set(&self) -> &[DeltaOp] {
        &self.set
    }

    pub fn del(&self) -> &[DelOp] {
        &self.del
    }

    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.del.is_empty()
    }

    pub fn op_count(&self) -> usize {
        self.set.len() + self.del.len()
    }

    /// 变动的块数，按区统计（三区 set/del 的投影，用于证据）。
    pub fn per_zone(&self) -> Value {
        let mut out = serde_json::Map::new();
        for zone in StateZone::ALL {
            let sets = self.set.iter().filter(|o| o.zone == zone).count();
            let dels = self.del.iter().filter(|o| o.zone == zone).count();
            out.insert(zone.as_str().to_string(), json!({"set": sets, "del": dels}));
        }
        Value::Object(out)
    }

    /// 内容寻址 id：整份 delta 的稳定标识（续跑时用来确认「还是同一笔传输」）。
    pub fn id(&self) -> CoreResult<String> {
        canonical_hash(&self.to_value()?)
    }

    pub fn to_value(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn from_value(value: &Value) -> CoreResult<Self> {
        let wire: StateDeltaWire =
            serde_json::from_value(value.clone()).map_err(|_| CoreError::Encoding)?;
        let delta = Self {
            agent: wire.agent,
            from_content_root: wire.from_content_root,
            to_content_root: wire.to_content_root,
            set: wire.set,
            del: wire.del,
        };
        delta.validate()?;
        Ok(delta)
    }

    /// 结构自洽：排序严格升序、无重复、每条 set 摘要自洽、zone/key 合法。
    pub fn validate(&self) -> CoreResult<()> {
        Did::parse(self.agent.as_str())?;
        let mut prev: Option<(usize, &str)> = None;
        for op in &self.set {
            op.verify()?;
            if op.zone == StateZone::Fs && op.key.is_empty() {
                return Err(CoreError::Encoding);
            }
            if let Some((pz, pk)) = prev {
                if op.zone.index() < pz || (op.zone.index() == pz && op.key.as_str() <= pk) {
                    return Err(CoreError::Encoding);
                }
            }
            prev = Some((op.zone.index(), op.key.as_str()));
        }
        let mut prev_del: Option<(usize, &str)> = None;
        for op in &self.del {
            if let Some((pz, pk)) = prev_del {
                if op.zone.index() < pz || (op.zone.index() == pz && op.key.as_str() <= pk) {
                    return Err(CoreError::Encoding);
                }
            }
            prev_del = Some((op.zone.index(), op.key.as_str()));
        }
        // 同一个 (zone,key) 不能既 set 又 del。
        for op in &self.del {
            if self
                .set
                .iter()
                .any(|s| s.zone == op.zone && s.key == op.key)
            {
                return Err(CoreError::Encoding);
            }
        }
        Ok(())
    }

    /// 把差异应用到 base 上。**先验证 base 是不是我们以为的 base**：
    /// 不匹配即拒绝——这是「续跑/重放不会把状态搞乱」的关键断言。
    pub fn apply_to(&self, base: &StateSnapshot) -> CoreResult<StateSnapshot> {
        self.validate()?;
        if base.agent() != &self.agent {
            return Err(CoreError::InvalidDid);
        }
        if base.content_root()? != self.from_content_root {
            // base 不是这笔 delta 的起点：可能过期（竞争）或被人换过（恶意）。
            return Err(CoreError::InvalidSignature);
        }
        let mut blocks = base.blocks().to_vec();
        for op in &self.del {
            blocks.retain(|b| !(b.zone() == op.zone && b.key() == op.key));
        }
        for op in &self.set {
            blocks.retain(|b| !(b.zone() == op.zone && b.key() == op.key));
            blocks.push(StateBlock::new(op.zone, op.key.clone(), op.value.clone())?);
        }
        let applied =
            StateSnapshot::capture(base.agent(), base.source_node(), base.epoch(), blocks)?;
        if applied.content_root()? != self.to_content_root {
            return Err(CoreError::InvalidSignature);
        }
        Ok(applied)
    }

    /// 应用并搬到目标节点/时刻：迁移的最后一步。
    pub fn apply_moved(
        &self,
        base: &StateSnapshot,
        node: &str,
        epoch: u64,
    ) -> CoreResult<StateSnapshot> {
        self.apply_to(base)?.rebase(node, epoch)
    }

    /// 按 `max_ops` 切块。块内操作保持全局序，块自身内容寻址，从而可续跑。
    pub fn chunked(&self, max_ops: usize) -> CoreResult<Vec<DeltaChunk>> {
        if max_ops == 0 {
            return Err(CoreError::ZeroAmount);
        }
        let mut chunks = Vec::new();
        let mut index = 0usize;
        let mut set_start = 0usize;
        let mut del_start = 0usize;
        while set_start < self.set.len() || del_start < self.del.len() {
            let mut budget = max_ops;
            let take_set = budget.min(self.set.len() - set_start);
            budget -= take_set;
            let take_del = budget.min(self.del.len() - del_start);
            // 保证每轮至少前进一格，避免 max_ops 被极端输入卡死。
            let (take_set, take_del) = if take_set + take_del == 0 {
                (usize::from(set_start < self.set.len()), 0)
            } else {
                (take_set, take_del)
            };
            let chunk = DeltaChunk {
                index,
                delta_id: self.id()?,
                from_content_root: self.from_content_root.clone(),
                to_content_root: self.to_content_root.clone(),
                set: self.set[set_start..set_start + take_set].to_vec(),
                del: self.del[del_start..del_start + take_del].to_vec(),
                total_ops: self.op_count(),
                digest: String::new(),
            };
            set_start += take_set;
            del_start += take_del;
            chunks.push(chunk.with_digest()?);
            index += 1;
        }
        Ok(chunks)
    }
}

impl<'de> Deserialize<'de> for StateDelta {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error as DeError;
        let wire = StateDeltaWire::deserialize(deserializer)?;
        let delta = StateDelta {
            agent: wire.agent,
            from_content_root: wire.from_content_root,
            to_content_root: wire.to_content_root,
            set: wire.set,
            del: wire.del,
        };
        delta.validate().map_err(D::Error::custom)?;
        Ok(delta)
    }
}

/// 一块差异。`digest` 覆盖块自身的全部字段：续跑时先校验摘要，
/// 已经收到的块不会被重新应用（幂等），没到齐的块可以单独重发。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DeltaChunk {
    index: usize,
    delta_id: String,
    from_content_root: String,
    to_content_root: String,
    set: Vec<DeltaOp>,
    del: Vec<DelOp>,
    total_ops: usize,
    digest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeltaChunkWire {
    index: usize,
    delta_id: String,
    from_content_root: String,
    to_content_root: String,
    set: Vec<DeltaOp>,
    del: Vec<DelOp>,
    total_ops: usize,
    digest: String,
}

impl DeltaChunk {
    fn digest_of(
        index: usize,
        delta_id: &str,
        from: &str,
        to: &str,
        set: &[DeltaOp],
        del: &[DelOp],
        total_ops: usize,
    ) -> CoreResult<String> {
        canonical_hash(&json!({
            "del": del,
            "delta": delta_id,
            "from": from,
            "index": index,
            "set": set,
            "to": to,
            "total_ops": total_ops,
        }))
    }

    fn with_digest(mut self) -> CoreResult<Self> {
        self.digest = Self::digest_of(
            self.index,
            &self.delta_id,
            &self.from_content_root,
            &self.to_content_root,
            &self.set,
            &self.del,
            self.total_ops,
        )?;
        Ok(self)
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn delta_id(&self) -> &str {
        &self.delta_id
    }

    pub fn from_content_root(&self) -> &str {
        &self.from_content_root
    }

    pub fn to_content_root(&self) -> &str {
        &self.to_content_root
    }

    pub fn set(&self) -> &[DeltaOp] {
        &self.set
    }

    pub fn del(&self) -> &[DelOp] {
        &self.del
    }

    pub fn total_ops(&self) -> usize {
        self.total_ops
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub fn op_count(&self) -> usize {
        self.set.len() + self.del.len()
    }

    pub fn verify(&self) -> CoreResult<()> {
        for op in &self.set {
            op.verify()?;
        }
        let expected = Self::digest_of(
            self.index,
            &self.delta_id,
            &self.from_content_root,
            &self.to_content_root,
            &self.set,
            &self.del,
            self.total_ops,
        )?;
        if expected == self.digest {
            Ok(())
        } else {
            Err(CoreError::InvalidSignature)
        }
    }

    pub fn to_value(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    /// 反序列化并**重新验证**摘要：被改过的块在进入会话前就被拒。
    pub fn from_value(value: &Value) -> CoreResult<Self> {
        serde_json::from_value(value.clone()).map_err(|_| CoreError::Encoding)
    }
}

/// 仅供本 crate 的测试构造「同 index、不同内容」的块，用来覆盖冲突拒绝分支。
#[cfg(test)]
pub(crate) fn test_chunk(
    index: usize,
    delta_id: &str,
    from: &str,
    to: &str,
    set: Vec<DeltaOp>,
    del: Vec<DelOp>,
    total_ops: usize,
) -> CoreResult<DeltaChunk> {
    DeltaChunk {
        index,
        delta_id: delta_id.to_string(),
        from_content_root: from.to_string(),
        to_content_root: to.to_string(),
        set,
        del,
        total_ops,
        digest: String::new(),
    }
    .with_digest()
}

impl<'de> Deserialize<'de> for DeltaChunk {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error as DeError;
        let wire = DeltaChunkWire::deserialize(deserializer)?;
        let chunk = DeltaChunk {
            index: wire.index,
            delta_id: wire.delta_id,
            from_content_root: wire.from_content_root,
            to_content_root: wire.to_content_root,
            set: wire.set,
            del: wire.del,
            total_ops: wire.total_ops,
            digest: wire.digest,
        };
        chunk.verify().map_err(D::Error::custom)?;
        Ok(chunk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::StateBlock;

    fn did(seed: u8) -> Did {
        au4a_core::AgentKeys::from_seed(&[seed; 32]).did()
    }

    fn snap(blocks: Vec<(StateZone, &str, Value)>) -> StateSnapshot {
        let blocks: Vec<StateBlock> = blocks
            .into_iter()
            .map(|(z, k, v)| StateBlock::new(z, k, v).unwrap())
            .collect();
        StateSnapshot::capture(&did(1), "node-a", 1, blocks).unwrap()
    }

    #[test]
    fn diff_finds_sets_and_dels_in_all_three_zones() {
        let a = snap(vec![
            (StateZone::Fs, "/a", json!(1)),
            (StateZone::Memory, "m", json!(2)),
            (StateZone::Context, "goal", json!("x")),
            (StateZone::Context, "gone", json!(true)),
        ]);
        let b = snap(vec![
            (StateZone::Fs, "/a", json!(1)),
            (StateZone::Memory, "m", json!(3)),
            (StateZone::Context, "goal", json!("x")),
            (StateZone::Context, "new", json!(false)),
        ]);
        let d = StateDelta::between(&a, &b).unwrap();
        assert_eq!(d.set().len(), 2);
        assert_eq!(d.del().len(), 1);
        assert_eq!(d.per_zone()["memory"]["set"], json!(1));
        assert_eq!(d.per_zone()["context"]["del"], json!(1));
        assert_eq!(d.set()[0].zone, StateZone::Memory);
    }

    #[test]
    fn applying_a_delta_reproduces_the_target_content() {
        let a = snap(vec![
            (StateZone::Fs, "/a", json!(1)),
            (StateZone::Memory, "m", json!(2)),
        ]);
        let b = snap(vec![
            (StateZone::Fs, "/a", json!(9)),
            (StateZone::Context, "c", json!(1)),
        ]);
        let d = StateDelta::between(&a, &b).unwrap();
        let applied = d.apply_to(&a).unwrap();
        assert_eq!(applied.content_root().unwrap(), b.content_root().unwrap());
    }

    #[test]
    fn applying_to_the_wrong_base_is_refused() {
        let a = snap(vec![(StateZone::Fs, "/a", json!(1))]);
        let b = snap(vec![(StateZone::Fs, "/a", json!(2))]);
        let c = snap(vec![(StateZone::Fs, "/a", json!(3))]);
        let d = StateDelta::between(&a, &b).unwrap();
        // 拿 c 当 base：必须拒绝，而不是「尽力而为」地算出个东西。
        assert_eq!(d.apply_to(&c), Err(CoreError::InvalidSignature));
    }

    #[test]
    fn reapplying_a_delta_is_idempotent_or_refused_but_never_corrupts() {
        let a = snap(vec![(StateZone::Memory, "m", json!(1))]);
        let b = snap(vec![(StateZone::Memory, "m", json!(2))]);
        let d = StateDelta::between(&a, &b).unwrap();
        let once = d.apply_to(&a).unwrap();
        // 再应用一次：base 已经不是 from，必须被拒（不会被静默重复应用）。
        assert_eq!(d.apply_to(&once), Err(CoreError::InvalidSignature));
    }

    #[test]
    fn chunking_preserves_order_and_enables_resume() {
        let from_blocks: Vec<(StateZone, String, Value)> = (0..10)
            .map(|i| (StateZone::Memory, format!("k{i}"), json!(i)))
            .collect();
        let to_blocks: Vec<(StateZone, String, Value)> = (0..10)
            .map(|i| (StateZone::Memory, format!("k{i}"), json!(i + 100)))
            .collect();
        let a = snap(
            from_blocks
                .iter()
                .map(|(z, k, v)| (*z, k.as_str(), v.clone()))
                .collect(),
        );
        let b = snap(
            to_blocks
                .iter()
                .map(|(z, k, v)| (*z, k.as_str(), v.clone()))
                .collect(),
        );
        let d = StateDelta::between(&a, &b).unwrap();
        let chunks = d.chunked(3).unwrap();
        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks[0].op_count(), 3);
        assert_eq!(chunks[3].op_count(), 1);
        assert!(chunks.iter().all(|c| c.verify().is_ok()));
        let round: DeltaChunk = serde_json::from_value(chunks[1].to_value().unwrap()).unwrap();
        assert_eq!(round, chunks[1]);
    }

    #[test]
    fn a_tampered_chunk_is_refused() {
        let a = snap(vec![(StateZone::Memory, "m", json!(1))]);
        let b = snap(vec![(StateZone::Memory, "m", json!(2))]);
        let d = StateDelta::between(&a, &b).unwrap();
        let chunk = &d.chunked(8).unwrap()[0];
        let mut value = chunk.to_value().unwrap();
        value["set"][0]["value"] = json!(999);
        assert!(serde_json::from_value::<DeltaChunk>(value).is_err());
    }

    #[test]
    fn delta_roundtrips_and_detects_duplicate_ops() {
        let a = snap(vec![(StateZone::Fs, "/a", json!(1))]);
        let b = snap(vec![
            (StateZone::Fs, "/a", json!(2)),
            (StateZone::Fs, "/b", json!(3)),
        ]);
        let d = StateDelta::between(&a, &b).unwrap();
        let value = d.to_value().unwrap();
        assert_eq!(StateDelta::from_value(&value).unwrap(), d);

        let mut bad = value.clone();
        let first = bad["set"][0].clone();
        bad["set"].as_array_mut().unwrap().push(first);
        assert_eq!(StateDelta::from_value(&bad), Err(CoreError::Encoding));
    }

    #[test]
    fn an_empty_delta_is_a_no_op() {
        let a = snap(vec![(StateZone::Fs, "/a", json!(1))]);
        let d = StateDelta::between(&a, &a).unwrap();
        assert!(d.is_empty());
        assert_eq!(
            d.apply_to(&a).unwrap().content_root().unwrap(),
            a.content_root().unwrap()
        );
        assert!(d.chunked(4).unwrap().is_empty());
    }

    #[test]
    fn cross_agent_diff_is_refused() {
        let a = StateSnapshot::capture(
            &did(2),
            "node-a",
            1,
            vec![StateBlock::new(StateZone::Fs, "/a", json!(1)).unwrap()],
        )
        .unwrap();
        let b = StateSnapshot::capture(
            &did(3),
            "node-a",
            1,
            vec![StateBlock::new(StateZone::Fs, "/a", json!(2)).unwrap()],
        )
        .unwrap();
        assert_eq!(StateDelta::between(&a, &b), Err(CoreError::InvalidDid));
    }
}
