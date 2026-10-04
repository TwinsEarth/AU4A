//! v1.6.1 经验库（Experience Store）。
//!
//! 个体学习的第一步是「Agent 记得住自己做过什么」。本模块定义经验的规范化结构与有界本地存储。
//!
//! 三条刻意的设计：
//!
//! 1. **同一语义只有一种字节表示**：协作者列表升序去重，内容键 = 规范 JSON 的 SHA-256。
//!    于是「重复记录」可以被精确识别，而不是靠时间戳或启发式去猜。
//! 2. **存储有界且淘汰可见**：容量满时按 FIFO 淘汰最旧一条，并把淘汰事实放进
//!    [`RecordOutcome::Evicted`]。学习系统最危险的失败模式是把「数据被丢了」当成「没有坏数据」。
//! 3. **经验库不读时钟**：时间戳由调用方从 `au4a_core::LogicalClock` 取。
//!    经验库只负责记录，因此它可以被逐字节重放。

use std::collections::BTreeMap;

use au4a_core::{canonical_hash, canonicalize, CoreError, CoreResult, Credits, Did, SelfCheck};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 标识类字段（`task_id` / `task_type`）长度上限。
pub const MAX_ID_LEN: usize = 128;
/// 上下文串长度上限。
pub const MAX_CONTEXT_LEN: usize = 512;
/// 动作串长度上限。
pub const MAX_ACTION_LEN: usize = 256;
/// 单条经验可携带的协作者上限：参与者超过 16 的任务在本项目里已不构成「个体经验」。
pub const MAX_PEERS: usize = 16;
/// 默认容量（条）。4096 条 × 数百字节与 PMB 帧上限同量级，是刻意选的有界内存。
pub const DEFAULT_CAPACITY: usize = 4096;

/// 任务结局。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Success,
    Failure,
    Partial,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Success => "success",
            Outcome::Failure => "failure",
            Outcome::Partial => "partial",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "success" => Some(Outcome::Success),
            "failure" => Some(Outcome::Failure),
            "partial" => Some(Outcome::Partial),
            _ => None,
        }
    }

    /// 完成质量（万分比）：`Success=10000` / `Partial=5000` / `Failure=0`。
    ///
    /// 用整数而非布尔，是因为后续的线性更新需要「部分成功」这个中间档；
    /// 把它压成 true/false 会让反馈信号丢失一半信息。
    pub fn quality_bp(self) -> i64 {
        match self {
            Outcome::Success => 10_000,
            Outcome::Partial => 5_000,
            Outcome::Failure => 0,
        }
    }

    pub fn is_success(self) -> bool {
        matches!(self, Outcome::Success)
    }
}

/// 一条经验：Agent 自己做的事 + 结果 + 参与者。
///
/// 字段与 spec 一致：`task_id` / `task_type` / `context` / `action` / `outcome` / `reward` /
/// `timestamp` / `peer_agents`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Experience {
    /// 任务标识（单 token，无空白）。
    pub task_id: String,
    /// 任务类型（学习行为按类型聚合的维度之一）。
    pub task_type: String,
    /// 上下文描述（可能包含敏感信息，对外发布前必须脱敏）。
    pub context: String,
    /// Agent 采取的动作。
    pub action: String,
    /// 结局。
    pub outcome: Outcome,
    /// 结算金额（微积分，整数）。
    pub reward: Credits,
    /// 逻辑时刻（由调用方的 `LogicalClock` 提供）。
    pub timestamp: u64,
    /// 参与者（严格升序、去重；升序是为了让内容键唯一）。
    pub peer_agents: Vec<Did>,
}

impl Experience {
    /// 构造并规范化：先排序去重协作者，再校验。非法输入返回 [`CoreError::InvalidKind`]。
    // 参数与 spec 定义的 Experience 字段一一对应（task_id/type/context/action/outcome/reward/timestamp/peers）。
    // 为了迁就 lint 计数而引入 builder 只会让「结构体字段 ↔ 构造参数」的对应关系变模糊。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        task_id: &str,
        task_type: &str,
        context: &str,
        action: &str,
        outcome: Outcome,
        reward: Credits,
        timestamp: u64,
        peers: &[Did],
    ) -> CoreResult<Self> {
        let mut exp = Self {
            task_id: task_id.to_string(),
            task_type: task_type.to_string(),
            context: context.to_string(),
            action: action.to_string(),
            outcome,
            reward,
            timestamp,
            peer_agents: peers.to_vec(),
        };
        exp.normalize();
        exp.validate()?;
        Ok(exp)
    }

    /// 规范化：协作者升序去重。同一条经验的「集合语义」因此只有一种表示。
    pub fn normalize(&mut self) {
        self.peer_agents.sort();
        self.peer_agents.dedup();
    }

    /// 校验字段约束。全部不合法的输入统一映射为 [`CoreError::InvalidKind`]
    /// （`au4a-core` 的错误变体是冻结的，本轨道不新增错误类型；映射约定见 crate 文档）。
    pub fn validate(&self) -> CoreResult<()> {
        if !is_token(&self.task_id, MAX_ID_LEN) {
            return Err(CoreError::InvalidKind);
        }
        if !is_token(&self.task_type, MAX_ID_LEN) {
            return Err(CoreError::InvalidKind);
        }
        if self.context.is_empty()
            || self.context.len() > MAX_CONTEXT_LEN
            || self.context.chars().any(char::is_control)
        {
            return Err(CoreError::InvalidKind);
        }
        if self.action.is_empty()
            || self.action.len() > MAX_ACTION_LEN
            || self.action.chars().any(char::is_control)
        {
            return Err(CoreError::InvalidKind);
        }
        if self.peer_agents.len() > MAX_PEERS {
            return Err(CoreError::InvalidKind);
        }
        if !self.peer_agents.windows(2).all(|w| w[0] < w[1]) {
            return Err(CoreError::InvalidKind);
        }
        Ok(())
    }

    /// 内容键：规范 JSON 的 SHA-256。同一条经验 → 同一个键，与插入顺序无关。
    pub fn key(&self) -> CoreResult<String> {
        canonical_hash(&self.to_value()?)
    }

    pub fn to_value(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn peer_count(&self) -> usize {
        self.peer_agents.len()
    }

    pub fn quality_bp(&self) -> i64 {
        self.outcome.quality_bp()
    }

    pub fn is_success(&self) -> bool {
        self.outcome.is_success()
    }
}

/// 单次记录的结局。**淘汰与重复都要被调用方看到**，不允许静默发生。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordOutcome {
    /// 新记录。
    Added { key: String },
    /// 内容键已存在（同一条经验被记了两次）。
    Duplicate { key: String },
    /// 新记录已写入，最旧的一条被容量淘汰。
    Evicted { key: String, evicted: String },
}

impl RecordOutcome {
    pub fn key(&self) -> &str {
        match self {
            RecordOutcome::Added { key }
            | RecordOutcome::Duplicate { key }
            | RecordOutcome::Evicted { key, .. } => key,
        }
    }

    pub fn accepted(&self) -> bool {
        !matches!(self, RecordOutcome::Duplicate { .. })
    }
}

/// 经验库统计（观察层可读；不含上下文原文与协作者明细）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreStats {
    pub len: usize,
    pub capacity: usize,
    pub duplicates: u64,
    pub evictions: u64,
    pub by_outcome: BTreeMap<String, usize>,
    pub task_types: Vec<String>,
}

/// 有界经验库。追加序即证据序；`index` 是派生的索引，不参与内容寻址。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperienceStore {
    entries: Vec<Experience>,
    /// 内容键 → 下标。派生数据：序列化时跳过，反序列化后重建，避免「索引与内容不一致」的伪造。
    #[serde(skip)]
    index: BTreeMap<String, usize>,
    capacity: usize,
    duplicates: u64,
    evictions: u64,
}

impl ExperienceStore {
    /// 容量必须 ≥ 1：容量为 0 的库无法记录任何经验，属于调用错误。
    pub fn new(capacity: usize) -> CoreResult<Self> {
        if capacity == 0 {
            return Err(CoreError::InvalidKind);
        }
        Ok(Self {
            entries: Vec::new(),
            index: BTreeMap::new(),
            capacity,
            duplicates: 0,
            evictions: 0,
        })
    }

    pub fn with_default_capacity() -> Self {
        Self {
            entries: Vec::new(),
            index: BTreeMap::new(),
            capacity: DEFAULT_CAPACITY,
            duplicates: 0,
            evictions: 0,
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

    pub fn duplicates(&self) -> u64 {
        self.duplicates
    }

    pub fn evictions(&self) -> u64 {
        self.evictions
    }

    pub fn entries(&self) -> &[Experience] {
        &self.entries
    }

    /// 最近 `n` 条（追加序的尾部），用于「近期反馈」类分析。
    pub fn recent(&self, n: usize) -> &[Experience] {
        let start = self.entries.len().saturating_sub(n);
        &self.entries[start..]
    }

    pub fn contains(&self, key: &str) -> bool {
        self.index.contains_key(key)
    }

    pub fn get(&self, key: &str) -> Option<&Experience> {
        self.index.get(key).and_then(|i| self.entries.get(*i))
    }

    /// 记录一条经验。校验在写入前完成；容量满时 FIFO 淘汰最旧一条并如实返回。
    pub fn record(&mut self, exp: Experience) -> CoreResult<RecordOutcome> {
        exp.validate()?;
        let key = exp.key()?;
        if self.index.contains_key(&key) {
            self.duplicates = self.duplicates.saturating_add(1);
            return Ok(RecordOutcome::Duplicate { key });
        }
        if self.entries.len() >= self.capacity {
            let evicted = self.entries.remove(0).key()?;
            self.evictions = self.evictions.saturating_add(1);
            self.entries.push(exp);
            self.reindex();
            return Ok(RecordOutcome::Evicted { key, evicted });
        }
        self.entries.push(exp);
        self.reindex();
        Ok(RecordOutcome::Added { key })
    }

    /// 按任务类型过滤（服务学习行为里「任务类型偏好」的统计）。
    pub fn by_task_type<'a>(
        &'a self,
        task_type: &'a str,
    ) -> impl Iterator<Item = &'a Experience> + 'a {
        self.entries
            .iter()
            .filter(move |e| e.task_type == task_type)
    }

    /// 出现过的任务类型（升序，确定性）。
    pub fn task_types(&self) -> Vec<String> {
        let mut types: Vec<String> = self
            .entries
            .iter()
            .map(|e| e.task_type.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        types.sort();
        types
    }

    pub fn stats(&self) -> StoreStats {
        let mut by_outcome: BTreeMap<String, usize> = BTreeMap::new();
        for e in &self.entries {
            *by_outcome
                .entry(e.outcome.as_str().to_string())
                .or_insert(0) += 1;
        }
        StoreStats {
            len: self.entries.len(),
            capacity: self.capacity,
            duplicates: self.duplicates,
            evictions: self.evictions,
            by_outcome,
            task_types: self.task_types(),
        }
    }

    pub fn to_value(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    /// 规范 JSON 字节串（键按字节序、无浮点）。同样内容 → 同样字节。
    pub fn canonical_json(&self) -> CoreResult<String> {
        canonicalize(&self.to_value()?)
    }

    /// 内容摘要：规范 JSON 的 SHA-256。可用来断言「两次学习得到同一个经验库」。
    pub fn digest(&self) -> CoreResult<String> {
        canonical_hash(&self.to_value()?)
    }

    /// 从规范/普通 JSON 载入，并**重建派生索引**、重新校验全部不变式。
    ///
    /// 载入路径不接受被篡改的内容：字段非法、容量非法、或序列化里含重复条目都返回错误。
    pub fn from_json_str(s: &str) -> CoreResult<Self> {
        let mut store: Self = serde_json::from_str(s).map_err(|_| CoreError::Encoding)?;
        if store.capacity == 0 || store.entries.len() > store.capacity {
            return Err(CoreError::InvalidKind);
        }
        for exp in &store.entries {
            exp.validate()?;
        }
        store.reindex();
        // 索引长度 != 条目数 ⇒ 序列化里存在重复内容键 ⇒ 不接受。
        if store.index.len() != store.entries.len() {
            return Err(CoreError::InvalidKind);
        }
        Ok(store)
    }

    fn reindex(&mut self) {
        self.index.clear();
        for (i, exp) in self.entries.iter().enumerate() {
            if let Ok(key) = exp.key() {
                self.index.insert(key, i);
            }
        }
    }

    /// 真实断言（不是占位）：去重、FIFO 淘汰计数、内容寻址、载入校验、非法输入拒绝。
    pub fn self_check() -> Vec<SelfCheck> {
        let mut checks = Vec::new();
        let peers: Vec<Did> = (0..3u8)
            .map(|s| au4a_core::AgentKeys::from_seed(&[s + 1; 32]).did())
            .collect();
        let sample = |id: &str, outcome: Outcome, reward: i64| {
            let context = format!("ctx-{id}");
            Experience::new(
                id,
                "translate.en-zh",
                &context,
                "quote",
                outcome,
                Credits(reward),
                id.len() as u64,
                &peers,
            )
        };

        // 1) 去重 + FIFO 淘汰计数
        let mut store = match ExperienceStore::new(3) {
            Ok(s) => s,
            Err(e) => {
                return vec![SelfCheck::fail(
                    super::TRACK,
                    "experience.bounded_store",
                    format!("容量 3 的库构造失败: {e}"),
                )]
            }
        };
        let mut dup_hits = 0usize;
        let mut evict_hits = 0usize;
        for id in ["t1", "t2", "t3", "t4"] {
            match sample(id, Outcome::Success, 10).and_then(|e| store.record(e)) {
                Ok(RecordOutcome::Evicted { .. }) => evict_hits += 1,
                Ok(_) => {}
                Err(_) => dup_hits += 1000,
            }
        }
        if let Ok(exp) = sample("t2", Outcome::Success, 10) {
            if matches!(store.record(exp), Ok(RecordOutcome::Duplicate { .. })) {
                dup_hits += 1;
            }
        }
        let bounded = store.len() == 3
            && store.evictions() == 1
            && evict_hits == 1
            && store.duplicates() == 1
            && dup_hits == 1;
        checks.push(if bounded {
            SelfCheck::pass(
                super::TRACK,
                "experience.bounded_store",
                "容量 3：写 4 条淘汰 1 条、重复 1 条被精确识别、条目数保持 3",
            )
        } else {
            SelfCheck::fail(
                super::TRACK,
                "experience.bounded_store",
                format!(
                    "len={} evictions={} duplicates={} dup_hits={dup_hits}",
                    store.len(),
                    store.evictions(),
                    store.duplicates()
                ),
            )
        });

        // 2) 内容寻址：同内容 → 同键；不同内容 → 不同键
        let a = sample("t1", Outcome::Partial, 7);
        let b = sample("t1", Outcome::Partial, 7);
        let c = sample("t1", Outcome::Failure, 7);
        let addressed = match (a, b, c) {
            (Ok(a), Ok(b), Ok(c)) => match (a.key(), b.key(), c.key()) {
                (Ok(ka), Ok(kb), Ok(kc)) => ka == kb && ka != kc,
                _ => false,
            },
            _ => false,
        };
        checks.push(if addressed {
            SelfCheck::pass(
                super::TRACK,
                "experience.content_addressed",
                "同内容经验键相同、不同结局经验键不同（SHA-256 内容寻址）",
            )
        } else {
            SelfCheck::fail(
                super::TRACK,
                "experience.content_addressed",
                "内容键计算失败或不唯一",
            )
        });

        // 3) 可移植：规范 JSON 往返后摘要不变
        let portable = store
            .canonical_json()
            .and_then(|s| ExperienceStore::from_json_str(&s))
            .and_then(|restored| Ok((restored.digest()?, restored.len())))
            .map(|(d, n)| d == store.digest().unwrap_or_default() && n == store.len())
            .unwrap_or(false);
        checks.push(if portable {
            SelfCheck::pass(
                super::TRACK,
                "experience.portable_roundtrip",
                "规范 JSON 往返后条目数与内容摘要逐字节一致",
            )
        } else {
            SelfCheck::fail(
                super::TRACK,
                "experience.portable_roundtrip",
                "往返后摘要或条目数变化",
            )
        });

        // 4) 非法输入必须被拒绝
        // 未排序的协作者列表：先排序再反转，保证一定是「未升序」而不是碰运气。
        let mut unordered = vec![peers[0].clone(), peers[1].clone()];
        unordered.sort();
        unordered.reverse();
        let rejects = Experience::new("", "t", "c", "a", Outcome::Success, Credits(0), 1, &[])
            .is_err()
            && Experience::new(
                "id",
                "t",
                &"x".repeat(MAX_CONTEXT_LEN + 1),
                "a",
                Outcome::Success,
                Credits(0),
                1,
                &[],
            )
            .is_err()
            && Experience::new("id", "t", "c", "a", Outcome::Success, Credits(0), 1, &[])
                .map(|mut e| {
                    e.peer_agents = unordered;
                    e.validate()
                })
                .map(|r| r.is_err())
                .unwrap_or(false)
            && ExperienceStore::new(0).is_err();
        checks.push(if rejects {
            SelfCheck::pass(
                super::TRACK,
                "experience.rejects_malformed",
                "空 task_id / 超长 context / 未排序 peer / 容量 0 全部被拒绝",
            )
        } else {
            SelfCheck::fail(
                super::TRACK,
                "experience.rejects_malformed",
                "存在未被拒绝的非法输入",
            )
        });
        checks
    }
}

/// 单 token 校验：非空、不超长、只含 `[A-Za-z0-9-_.:]`。
fn is_token(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

/// 模块级自检入口（`crate::self_check` 聚合它）。
pub fn self_check() -> Vec<SelfCheck> {
    ExperienceStore::self_check()
}
