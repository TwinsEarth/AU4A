//! 状态快照（v1.3.1）：把 Agent 的三区状态冻结成**内容寻址**的块集合。
//!
//! Agent 的主体性要求它能在节点之间移动而**不丢失自己**。因此状态不是「一个文件」，
//! 而是三区（three zones）：
//!
//! * [`StateZone::Fs`]：文件系统投影（路径 → 内容）。
//! * [`StateZone::Memory`]：工作记忆（键 → 值）。
//! * [`StateZone::Context`]：上下文/任务状态（槽 → 值）。UDOS 的 `TransferBundle`
//!   （goal / todo / context / done / owner / trace）就落在这一区。
//!
//! 三区都被规范化成 `(zone, key, value)` 的**块**，块按 `(zone, key)` 字节序排序，
//! 每块有自己的内容摘要，整份快照有一个 [`StateSnapshot::root`]。
//! 于是「恢复后的状态和源逐字节一致」可以被压缩成一条断言：**root 相等**。
//!
//! 为什么排序与摘要都在这一层做：快照是**跨节点**的字节，任何插入序/宿主语言差异
//! 都会让同一份语义产生不同的字节，从而让验签随机失败。规范 JSON（AU4A-CJ）+
//! 排序 + 内容寻址把这件事变成确定的。

use au4a_core::{canonical_hash, canonicalize, CoreError, CoreResult, Did};
use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

/// 单块 key 的字节上限。key 是路径/记忆键/上下文槽，必须短到能被日志与索引承载。
pub const MAX_KEY_LEN: usize = 512;
/// 单块 value 的规范 JSON 字节上限（64 KiB）。
pub const MAX_VALUE_LEN: usize = 64 * 1024;
/// 单份快照的块数上限。
pub const MAX_BLOCKS: usize = 4096;
/// 节点名长度上限。
pub const MAX_NODE_LEN: usize = 128;

/// 状态所处的区。三区缺一不可：只搬文件不搬上下文，Agent 迁移后就不再是同一个主体。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateZone {
    /// 文件系统投影。
    Fs,
    /// 工作记忆。
    Memory,
    /// 上下文 / 任务状态。
    Context,
}

impl StateZone {
    /// 全部三区，顺序固定（排序与摘要都依赖它）。
    pub const ALL: [StateZone; 3] = [StateZone::Fs, StateZone::Memory, StateZone::Context];

    pub fn as_str(self) -> &'static str {
        match self {
            StateZone::Fs => "fs",
            StateZone::Memory => "memory",
            StateZone::Context => "context",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        StateZone::ALL.into_iter().find(|z| z.as_str() == s)
    }

    /// 区在规范序中的下标；越小的区在快照里越靠前。
    pub fn index(self) -> usize {
        match self {
            StateZone::Fs => 0,
            StateZone::Memory => 1,
            StateZone::Context => 2,
        }
    }
}

/// 一个状态块：`(zone, key, value)` + 内容摘要。
///
/// 字段私有：块只能经由 [`StateBlock::new`] 或校验过的反序列化构造，
/// 因此「存在的 StateBlock 一定自洽」是这个类型的不变式。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StateBlock {
    zone: StateZone,
    key: String,
    value: Value,
    digest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StateBlockWire {
    zone: StateZone,
    key: String,
    value: Value,
    digest: String,
}

impl StateBlock {
    /// 构造并计算摘要。key 非法 / value 含浮点 / value 过大都会被拒。
    pub fn new(zone: StateZone, key: impl Into<String>, value: Value) -> CoreResult<Self> {
        let key = key.into();
        validate_key(&key)?;
        validate_value(&value)?;
        let digest = block_digest(zone, &key, &value)?;
        Ok(Self {
            zone,
            key,
            value,
            digest,
        })
    }

    pub fn zone(&self) -> StateZone {
        self.zone
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn value(&self) -> &Value {
        &self.value
    }

    /// 内容摘要：`canonical_hash({"k":key,"v":value,"z":zone})`。
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// 重新计算摘要并与记录值比对；不一致说明块被替换或篡改。
    pub fn verify(&self) -> CoreResult<()> {
        if block_digest(self.zone, &self.key, &self.value)? == self.digest {
            Ok(())
        } else {
            Err(CoreError::InvalidSignature)
        }
    }

    /// 只取「定位 + 摘要」的轻量视图：增量 diff 与传输不需要搬运 value。
    pub fn locator(&self) -> Value {
        json!({"d": self.digest, "k": self.key, "z": self.zone.as_str()})
    }

    /// 用**已算好的**摘要构造块（v1.3.7 的哈希复用路径）。
    ///
    /// 为什么这是安全的：调用方只能从 [`crate::perf::DigestCache`] 拿到摘要，
    /// 而那个缓存的键就是「(zone, key, 规范 JSON 字节)」本身——命中即代表
    /// 输入逐字节相同，SHA-256 又是确定的，所以摘要必然相同。
    /// 换句话说：类型不变式由缓存的正确性保证，而不是靠信任调用方。
    pub(crate) fn new_cached(
        zone: StateZone,
        key: impl Into<String>,
        value: Value,
        digest: String,
    ) -> CoreResult<Self> {
        let key = key.into();
        validate_key(&key)?;
        validate_value(&value)?;
        if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(CoreError::InvalidSignature);
        }
        Ok(Self {
            zone,
            key,
            value,
            digest,
        })
    }
}

impl<'de> Deserialize<'de> for StateBlock {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = StateBlockWire::deserialize(deserializer)?;
        let block = StateBlock::new(wire.zone, wire.key, wire.value).map_err(D::Error::custom)?;
        if block.digest != wire.digest {
            return Err(D::Error::custom("block digest does not match content"));
        }
        Ok(block)
    }
}

fn block_digest(zone: StateZone, key: &str, value: &Value) -> CoreResult<String> {
    canonical_hash(&json!({"k": key, "v": value, "z": zone.as_str()}))
}

pub(crate) fn validate_key(key: &str) -> CoreResult<()> {
    if key.is_empty() || key.len() > MAX_KEY_LEN {
        return Err(CoreError::Encoding);
    }
    if key.chars().any(|c| (c as u32) < 0x20) {
        return Err(CoreError::Encoding);
    }
    Ok(())
}

fn validate_value(value: &Value) -> CoreResult<()> {
    // 规范 JSON 会拒绝浮点；这里额外限制单块大小，避免一份快照吃掉全部内存。
    let encoded = canonicalize(value)?;
    if encoded.len() > MAX_VALUE_LEN {
        return Err(CoreError::FrameTooLarge);
    }
    Ok(())
}

fn validate_node(node: &str) -> CoreResult<()> {
    if node.is_empty() || node.len() > MAX_NODE_LEN {
        return Err(CoreError::Encoding);
    }
    if node.chars().any(|c| (c as u32) < 0x20) {
        return Err(CoreError::Encoding);
    }
    Ok(())
}

/// 一份内容寻址的 Agent 状态快照。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StateSnapshot {
    agent: Did,
    source_node: String,
    epoch: u64,
    blocks: Vec<StateBlock>,
    root: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotWire {
    agent: Did,
    source_node: String,
    epoch: u64,
    blocks: Vec<StateBlock>,
    root: String,
}

impl StateSnapshot {
    /// 冻结一份快照：排序 → 去重校验 → 计算 root。
    pub fn capture(
        agent: &Did,
        source_node: &str,
        epoch: u64,
        blocks: Vec<StateBlock>,
    ) -> CoreResult<Self> {
        let mut blocks = blocks;
        blocks.sort_by(|a, b| {
            a.zone
                .index()
                .cmp(&b.zone.index())
                .then_with(|| a.key.as_bytes().cmp(b.key.as_bytes()))
        });
        Self::from_sorted_parts(agent, source_node, epoch, blocks)
    }

    /// 由「已排序」的块构造；顺序不是严格升序（含重复）即拒绝，root 必须自洽。
    fn from_sorted_parts(
        agent: &Did,
        source_node: &str,
        epoch: u64,
        blocks: Vec<StateBlock>,
    ) -> CoreResult<Self> {
        validate_node(source_node)?;
        Did::parse(agent.as_str())?;
        if blocks.len() > MAX_BLOCKS {
            return Err(CoreError::FrameTooLarge);
        }
        let mut prev: Option<(usize, &str)> = None;
        for block in &blocks {
            block.verify()?;
            if let Some((pz, pk)) = prev {
                let cz = block.zone.index();
                if cz < pz || (cz == pz && block.key.as_str() <= pk) {
                    // 同一个 (zone,key) 出现两次 = 输入本身有歧义，不能靠「后者覆盖前者」蒙混。
                    return Err(CoreError::Encoding);
                }
            }
            prev = Some((block.zone.index(), block.key.as_str()));
        }
        let root = root_of(agent, source_node, epoch, &blocks)?;
        Ok(Self {
            agent: agent.clone(),
            source_node: source_node.to_string(),
            epoch,
            blocks,
            root,
        })
    }

    pub fn agent(&self) -> &Did {
        &self.agent
    }

    pub fn source_node(&self) -> &str {
        &self.source_node
    }

    /// 快照是在哪一个逻辑时刻冻结的（基元层不读墙钟）。
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn blocks(&self) -> &[StateBlock] {
        &self.blocks
    }

    /// 内容寻址根：整份快照的稳定标识。
    pub fn root(&self) -> &str {
        &self.root
    }

    pub fn block(&self, zone: StateZone, key: &str) -> Option<&StateBlock> {
        self.blocks
            .iter()
            .find(|b| b.zone == zone && b.key == key)
    }

    pub fn zone_blocks(&self, zone: StateZone) -> impl Iterator<Item = &StateBlock> {
        self.blocks.iter().filter(move |b| b.zone == zone)
    }

    pub fn block_count(&self, zone: StateZone) -> usize {
        self.zone_blocks(zone).count()
    }

    /// 三区各自的摘要：一致性检查（v1.3.5）按区定位差异，而不是只看总 root。
    pub fn zone_root(&self, zone: StateZone) -> CoreResult<String> {
        let locators: Vec<Value> = self.zone_blocks(zone).map(|b| b.locator()).collect();
        canonical_hash(&json!({"b": locators, "z": zone.as_str()}))
    }

    /// 三区摘要的有序列表（fs, memory, context）。
    pub fn zone_roots(&self) -> CoreResult<Vec<(StateZone, String)>> {
        StateZone::ALL
            .into_iter()
            .map(|z| Ok((z, self.zone_root(z)?)))
            .collect()
    }

    /// 未签名载荷：签名与内容寻址共用同一份字节。
    pub fn body(&self) -> CoreResult<Value> {
        Ok(json!({
            "agent": self.agent,
            "blocks": self.blocks.iter().map(|b| b.locator()).collect::<Vec<_>>(),
            "epoch": self.epoch,
            "node": self.source_node,
            "root": self.root,
        }))
    }

    /// 完整快照投影（含 value），供持久化与传输使用。
    pub fn to_value(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    /// 从 JSON 还原并**重新验证** root：被替换/篡改的快照在这里就被拒。
    pub fn from_value(value: &Value) -> CoreResult<Self> {
        let wire: SnapshotWire =
            serde_json::from_value(value.clone()).map_err(|_| CoreError::Encoding)?;
        // `Did` 的 Deserialize 不做格式校验，这里补上：不合法的 DID 不能进快照。
        let parsed = Did::parse(wire.agent.as_str())?;
        if parsed != wire.agent {
            return Err(CoreError::InvalidDid);
        }
        let snapshot =
            Self::from_sorted_parts(&wire.agent, &wire.source_node, wire.epoch, wire.blocks)?;
        if snapshot.root != wire.root {
            return Err(CoreError::InvalidSignature);
        }
        Ok(snapshot)
    }

    /// 两个快照是否描述同一份状态（root 相等 ⇔ 逐字节一致）。
    pub fn same_state(&self, other: &StateSnapshot) -> bool {
        self.root == other.root
    }

    /// **状态内容**标识：只覆盖 (agent, blocks)，不含来源节点与逻辑时刻。
    ///
    /// 为什么需要第二个哈希：Agent 从节点 A 迁到节点 B 之后，状态的**内容**必须逐字节一致，
    /// 但「来源节点」「冻结时刻」作为出处（provenance）必然不同。把两者混在一个哈希里，
    /// 迁移就永远无法自证「我还是我」。于是：
    ///
    /// * [`StateSnapshot::root`]：快照文档标识（含 node/epoch），用于篡改检测与内容寻址。
    /// * `content_root`：状态标识（不含 node/epoch），用于**迁移等价性**验收。
    pub fn content_root(&self) -> CoreResult<String> {
        content_root_of(&self.agent, &self.blocks)
    }

    /// 状态内容是否与另一份快照一致（跨节点迁移的验收断言）。
    pub fn same_content(&self, other: &StateSnapshot) -> bool {
        match (self.content_root(), other.content_root()) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }

    /// 换一个承载节点/逻辑时刻重新冻结（内容不变，出处改变）。
    /// 迁移后目标节点上的快照就是源快照 `rebase` 的结果；`content_root` 必须保持不变。
    pub fn rebase(&self, node: &str, epoch: u64) -> CoreResult<StateSnapshot> {
        StateSnapshot::capture(&self.agent, node, epoch, self.blocks.clone())
    }

    /// 整体自洽：每块摘要与 root 都能被重新计算出来。被篡改/替换的快照在这里被拒。
    pub fn verify(&self) -> CoreResult<()> {
        for block in &self.blocks {
            block.verify()?;
        }
        if root_of(&self.agent, &self.source_node, self.epoch, &self.blocks)? == self.root {
            Ok(())
        } else {
            Err(CoreError::InvalidSignature)
        }
    }

    /// 只含定位与摘要的投影清单：增量 diff（v1.3.2）与一致性检查（v1.3.5）的输入。
    pub fn locators(&self) -> Vec<Value> {
        self.blocks.iter().map(|b| b.locator()).collect()
    }
}

fn root_of(
    agent: &Did,
    source_node: &str,
    epoch: u64,
    blocks: &[StateBlock],
) -> CoreResult<String> {
    let locators: Vec<Value> = blocks.iter().map(|b| b.locator()).collect();
    canonical_hash(&json!({
        "agent": agent,
        "blocks": locators,
        "epoch": epoch,
        "node": source_node,
    }))
}

/// 状态内容标识（不含 node/epoch）。
fn content_root_of(agent: &Did, blocks: &[StateBlock]) -> CoreResult<String> {
    let locators: Vec<Value> = blocks.iter().map(|b| b.locator()).collect();
    canonical_hash(&json!({"agent": agent, "blocks": locators}))
}

impl<'de> Deserialize<'de> for StateSnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = SnapshotWire::deserialize(deserializer)?;
        let snapshot =
            Self::from_sorted_parts(&wire.agent, &wire.source_node, wire.epoch, wire.blocks)
                .map_err(D::Error::custom)?;
        if snapshot.root != wire.root {
            return Err(D::Error::custom("snapshot root does not match content"));
        }
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn did(seed: u8) -> Did {
        au4a_core::AgentKeys::from_seed(&[seed; 32]).did()
    }

    fn sample() -> Vec<StateBlock> {
        vec![
            StateBlock::new(StateZone::Fs, "/work/a.txt", json!({"bytes": 12})).unwrap(),
            StateBlock::new(StateZone::Memory, "last_task", json!("translate")).unwrap(),
            StateBlock::new(StateZone::Context, "goal", json!({"text": "迁移"})).unwrap(),
        ]
    }

    #[test]
    fn root_is_independent_of_insertion_order() {
        let mut shuffled = sample();
        shuffled.reverse();
        let a = StateSnapshot::capture(&did(1), "node-a", 7, sample()).unwrap();
        let b = StateSnapshot::capture(&did(1), "node-a", 7, shuffled).unwrap();
        assert_eq!(a.root(), b.root());
    }

    #[test]
    fn duplicate_zone_key_is_refused() {
        let mut blocks = sample();
        blocks.push(StateBlock::new(StateZone::Memory, "last_task", json!("other")).unwrap());
        assert_eq!(
            StateSnapshot::capture(&did(1), "node-a", 1, blocks),
            Err(CoreError::Encoding)
        );
    }

    #[test]
    fn floats_are_refused_at_block_construction() {
        assert_eq!(
            StateBlock::new(StateZone::Memory, "price", json!(1.5)),
            Err(CoreError::FloatForbidden)
        );
    }

    #[test]
    fn tampered_block_is_detected_at_deserialization() {
        let block = StateBlock::new(StateZone::Memory, "k", json!(1)).unwrap();
        // 内容被改、摘要没跟着变 → 反序列化必须拒绝。
        let tampered = json!({
            "zone": "memory",
            "key": "k",
            "value": 2,
            "digest": block.digest(),
        });
        assert!(serde_json::from_value::<StateBlock>(tampered).is_err());
    }

    #[test]
    fn zone_roots_are_distinct_and_stable() {
        let snap = StateSnapshot::capture(&did(2), "node-a", 3, sample()).unwrap();
        let roots = snap.zone_roots().unwrap();
        assert_eq!(roots.len(), 3);
        assert_ne!(roots[0].1, roots[1].1);
        assert_eq!(roots[2].0, StateZone::Context);
        assert_eq!(snap.block_count(StateZone::Fs), 1);
    }

    #[test]
    fn json_roundtrip_preserves_root() {
        let snap = StateSnapshot::capture(&did(3), "node-a", 5, sample()).unwrap();
        let back = StateSnapshot::from_value(&snap.to_value().unwrap()).unwrap();
        assert_eq!(back.root(), snap.root());
        assert_eq!(back, snap);
    }

    #[test]
    fn replaced_snapshot_is_refused() {
        let a = StateSnapshot::capture(&did(4), "node-a", 5, sample()).unwrap();
        let mut other = sample();
        other.push(StateBlock::new(StateZone::Memory, "extra", json!(1)).unwrap());
        let b = StateSnapshot::capture(&did(4), "node-a", 5, other).unwrap();
        let mut value = a.to_value().unwrap();
        value["blocks"] = b.to_value().unwrap()["blocks"].clone();
        assert_eq!(
            StateSnapshot::from_value(&value),
            Err(CoreError::InvalidSignature)
        );
    }
}
