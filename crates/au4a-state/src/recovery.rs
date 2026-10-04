//! 两阶段提交恢复协议（v1.3.4）：prepare → commit → confirm，失败必回滚，
//! **目标节点不得留下部分状态**。
//!
//! 为什么不能「边收边写」：迁移中途断线、签名不符、磁盘满、对端作恶——任何一种
//! 都会让目标节点上出现「一半旧状态 + 一半新状态」的弗兰肯斯坦。那个 Agent 醒来之后
//! 既不是原来的自己，也不是新的自己，而这是**不可检测**的（两个正确状态之间的任意
//! 混合体没有内部矛盾）。
//!
//! 所以恢复协议用**影子代 + 单点切换**：
//!
//! ```text
//! live 视图 = gen:<head>: 命名空间     （head 是唯一入口，一次 put 即切换）
//!
//! prepare : 把目标状态**完整物化**到 gen:<head+1>: 并读回校验（live 不动）
//! commit  : 把 head 从 n 改成 n+1（单次原子 put）
//! confirm : 校验 live == 目标内容根，删除旧代与意图记录
//! rollback: 删掉影子代；若已 commit 则把 head 拨回去（confirm 之前都合法）
//! ```
//!
//! 关键不变式（测试逐条断言）：
//! 1. 任意时刻 `live` 的 content_root ∈ {base, target}，**不存在第三种可能**。
//! 2. 任意阶段注入故障后回滚，`live == base`，且没有孤儿代。
//! 3. 阶段顺序不可跳跃、不可重复：顺序错了就拒绝，而不是「尽力而为」。

use au4a_core::{canonical_hash, CoreError, CoreResult, Did, RefusalCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::diff::StateDelta;
use crate::signed::{SignedSnapshot, SnapshotPolicy};
use crate::snapshot::StateSnapshot;
use crate::store::{read_snapshot, write_snapshot, StateStore};
use crate::transfer::NodeId;

/// 当前生效代的指针键。**唯一**的切换点。
pub const HEAD_KEY: &str = "head";
/// 迁移意图记录键：崩溃恢复时靠它判断「有一笔未完成的迁移」。
pub const INTENT_KEY: &str = "intent";

/// 第 `gen` 代的命名空间前缀。
pub fn generation_prefix(gen: u64) -> String {
    format!("gen:{gen}:")
}

/// 迁移阶段。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Idle,
    Prepared,
    Committed,
    Confirmed,
    RolledBack,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Idle => "idle",
            Phase::Prepared => "prepared",
            Phase::Committed => "committed",
            Phase::Confirmed => "confirmed",
            Phase::RolledBack => "rolled_back",
        }
    }
}

/// 故障注入点。真实世界里的失败（断网、超时、进程被杀）在测试里必须可以**定点**复现。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultPoint {
    /// 影子代写入之前。
    BeforeStaging,
    /// 影子代写完之后（prepare 中途）。
    AfterStaging,
    /// head 切换之前。
    BeforeFlip,
    /// head 切换之后（commit 中途，confirm 之前）。
    AfterFlip,
    /// 清理旧代之前（confirm 中途）。
    BeforeCleanup,
}

impl FaultPoint {
    pub const ALL: [FaultPoint; 5] = [
        FaultPoint::BeforeStaging,
        FaultPoint::AfterStaging,
        FaultPoint::BeforeFlip,
        FaultPoint::AfterFlip,
        FaultPoint::BeforeCleanup,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            FaultPoint::BeforeStaging => "before_staging",
            FaultPoint::AfterStaging => "after_staging",
            FaultPoint::BeforeFlip => "before_flip",
            FaultPoint::AfterFlip => "after_flip",
            FaultPoint::BeforeCleanup => "before_cleanup",
        }
    }

    /// 注入的故障在语义上对应内核的哪个拒绝码（用于观察层归因）。
    pub fn refusal_code(self) -> RefusalCode {
        match self {
            FaultPoint::BeforeStaging | FaultPoint::AfterStaging => RefusalCode::Degraded,
            FaultPoint::BeforeFlip | FaultPoint::AfterFlip => RefusalCode::Timeout,
            FaultPoint::BeforeCleanup => RefusalCode::ResourceExhausted,
        }
    }
}

/// 定点故障注入器：只炸一次，且只在指定点炸。
#[derive(Clone, Debug, Default)]
pub struct FaultInjector {
    at: Option<FaultPoint>,
    fired: bool,
}

impl FaultInjector {
    pub fn none() -> Self {
        Self {
            at: None,
            fired: false,
        }
    }

    pub fn at(point: FaultPoint) -> Self {
        Self {
            at: Some(point),
            fired: false,
        }
    }

    pub fn fired(&self) -> bool {
        self.fired
    }

    pub fn point(&self) -> Option<FaultPoint> {
        self.at
    }

    /// 命中注入点则返回注入的故障（`Overflow` = 冻结错误集里最接近「外部故障」的码；
    /// 内核语义对应 `Timeout` / `Degraded`，见 [`FaultPoint::refusal_code`]）。
    pub fn trip(&mut self, point: FaultPoint) -> CoreResult<()> {
        if !self.fired && self.at == Some(point) {
            self.fired = true;
            return Err(CoreError::Overflow);
        }
        Ok(())
    }
}

/// 迁移计划：一笔事务的全部可验证字段。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationPlan {
    pub tx: String,
    pub agent: Did,
    pub from_node: String,
    pub to_node: String,
    pub base_content_root: String,
    pub target_content_root: String,
    pub delta_id: String,
    pub epoch: u64,
}

impl MigrationPlan {
    pub fn new(
        agent: &Did,
        from_node: &NodeId,
        to_node: &NodeId,
        base_content_root: impl Into<String>,
        target_content_root: impl Into<String>,
        delta_id: impl Into<String>,
        epoch: u64,
    ) -> CoreResult<Self> {
        let base_content_root = base_content_root.into();
        let target_content_root = target_content_root.into();
        let delta_id = delta_id.into();
        let tx = canonical_hash(&json!({
            "agent": agent,
            "base": base_content_root,
            "delta": delta_id,
            "epoch": epoch,
            "from": from_node.as_str(),
            "target": target_content_root,
            "to": to_node.as_str(),
        }))?;
        Ok(Self {
            tx,
            agent: agent.clone(),
            from_node: from_node.as_str().to_string(),
            to_node: to_node.as_str().to_string(),
            base_content_root,
            target_content_root,
            delta_id,
            epoch,
        })
    }
}

/// 目标节点的存储视图：`live = gen:<head>:`。
///
/// 只有这一个类型能改 `head`，且改法只有 [`NodeStore::flip_head`] 一处——这是
/// 「不出现部分状态」的工程保证，而不是靠调用方自律。
#[derive(Clone, Debug)]
pub struct NodeStore<S: StateStore> {
    node: NodeId,
    agent: Did,
    store: S,
}

impl<S: StateStore> NodeStore<S> {
    /// 打开（或初始化）一个节点的存储。初始 head = 1（第 1 代可以为空）。
    pub fn open(node: NodeId, agent: Did, mut store: S) -> CoreResult<Self> {
        if store.get(HEAD_KEY)?.is_none() {
            store.put(HEAD_KEY, json!(1))?;
        }
        Ok(Self { node, agent, store })
    }

    pub fn node(&self) -> &NodeId {
        &self.node
    }

    pub fn agent(&self) -> &Did {
        &self.agent
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut S {
        &mut self.store
    }

    /// 当前生效代。
    pub fn head(&self) -> CoreResult<u64> {
        match self.store.get(HEAD_KEY)? {
            Some(Value::Number(n)) => n.as_u64().ok_or(CoreError::Encoding),
            Some(_) => Err(CoreError::Encoding),
            None => Err(CoreError::Encoding),
        }
    }

    /// 读某一代的状态快照。
    pub fn read_generation(&self, gen: u64) -> CoreResult<StateSnapshot> {
        read_snapshot(
            &self.store,
            &generation_prefix(gen),
            &self.agent,
            self.node.as_str(),
            gen,
        )
    }

    /// 当前 live 视图。
    pub fn live_snapshot(&self) -> CoreResult<StateSnapshot> {
        self.read_generation(self.head()?)
    }

    pub fn live_content_root(&self) -> CoreResult<String> {
        self.live_snapshot()?.content_root()
    }

    /// 直接安装一份状态（初始化/灾难恢复用）：写新代 → 切换 → 清理旧代。
    pub fn install(&mut self, snapshot: &StateSnapshot, gc: bool) -> CoreResult<u64> {
        let head = self.head()?;
        let next = head + 1;
        self.write_generation(next, snapshot)?;
        let readback = self.read_generation(next)?;
        if readback.content_root()? != snapshot.content_root()? {
            return Err(CoreError::InvalidSignature);
        }
        self.flip_head(next)?;
        if gc {
            self.gc_except(next)?;
        }
        Ok(next)
    }

    /// 写一整代（不切换 head）。prepare 阶段唯一允许的写操作。
    pub fn write_generation(&mut self, gen: u64, snapshot: &StateSnapshot) -> CoreResult<usize> {
        if gen == self.head()? {
            // 直接覆盖生效代会让 live 处于「写了一半」的状态：绝对禁止。
            // 冻结错误集里没有 Conflict，用 `DuplicateAgent`（「已存在」语义）承载，
            // 内核侧对应 RefusalCode::Conflict。
            return Err(CoreError::DuplicateAgent);
        }
        let prefix = generation_prefix(gen);
        // 先清空该代（幂等重试安全），再整体写入。
        crate::store::clear_namespace(&mut self.store, &prefix)?;
        write_snapshot(&mut self.store, &prefix, snapshot)
    }

    /// **唯一**的 head 切换点：单次 put，切换前后 live 都指向一份完整状态。
    fn flip_head(&mut self, gen: u64) -> CoreResult<()> {
        if gen == 0 {
            return Err(CoreError::Encoding);
        }
        self.store.put(HEAD_KEY, json!(gen))
    }

    /// 除了 `keep` 之外的代（孤儿代）。迁移失败后必须为空。
    pub fn orphan_generations(&self, keep: u64) -> CoreResult<Vec<u64>> {
        let mut gens = Vec::new();
        for key in self.store.list("gen:")? {
            let rest = key.strip_prefix("gen:").ok_or(CoreError::Encoding)?;
            let (num, _) = rest.split_once(':').ok_or(CoreError::Encoding)?;
            let gen: u64 = num.parse().map_err(|_| CoreError::Encoding)?;
            if gen != keep && !gens.contains(&gen) {
                gens.push(gen);
            }
        }
        gens.sort_unstable();
        Ok(gens)
    }

    /// 删除除 `keep` 之外的全部代，返回删除的代数。
    pub fn gc_except(&mut self, keep: u64) -> CoreResult<usize> {
        let orphans = self.orphan_generations(keep)?;
        for gen in &orphans {
            crate::store::clear_namespace(&mut self.store, &generation_prefix(*gen))?;
        }
        Ok(orphans.len())
    }

    pub fn intent(&self) -> CoreResult<Option<Value>> {
        self.store.get(INTENT_KEY)
    }

    fn set_intent(&mut self, value: Value) -> CoreResult<()> {
        self.store.put(INTENT_KEY, value)
    }

    fn clear_intent(&mut self) -> CoreResult<()> {
        self.store.remove(INTENT_KEY)?;
        Ok(())
    }
}

/// 一次迁移的协调者（目标节点侧）。
#[derive(Clone, Debug)]
pub struct Migration {
    plan: MigrationPlan,
    phase: Phase,
    previous_generation: Option<u64>,
    staged_generation: Option<u64>,
    /// 失败发生时所处的阶段（用于证据）。
    failed_at: Option<Phase>,
}

/// prepare 阶段的回执：目标状态已经**完整**落在影子代里。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrepareReceipt {
    pub tx: String,
    pub staged_generation: u64,
    pub previous_generation: u64,
    pub blocks: usize,
    pub target_content_root: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitReceipt {
    pub tx: String,
    pub generation: u64,
    pub live_content_root: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmReceipt {
    pub tx: String,
    pub generation: u64,
    pub cleanup_removed: usize,
    pub live_content_root: String,
}

impl Migration {
    pub fn begin(plan: MigrationPlan) -> Self {
        Self {
            plan,
            phase: Phase::Idle,
            previous_generation: None,
            staged_generation: None,
            failed_at: None,
        }
    }

    pub fn plan(&self) -> &MigrationPlan {
        &self.plan
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn staged_generation(&self) -> Option<u64> {
        self.staged_generation
    }

    pub fn previous_generation(&self) -> Option<u64> {
        self.previous_generation
    }

    /// **prepare**：语义校验 + 影子代物化 + 读回校验。live 始终不动。
    pub fn prepare<S: StateStore>(
        &mut self,
        node: &mut NodeStore<S>,
        delta: &StateDelta,
        signed: &SignedSnapshot,
        faults: &mut FaultInjector,
    ) -> CoreResult<PrepareReceipt> {
        self.expect(Phase::Idle)?;
        if node.agent() != &self.plan.agent {
            return Err(CoreError::InvalidDid);
        }
        // 1) 签名闸门：即将安装的状态必须正是 Agent 亲笔签下的那一份。
        let policy = SnapshotPolicy::for_agent(self.plan.agent.clone())
            .expecting_content_root(self.plan.target_content_root.clone())
            .with_min_epoch(self.plan.epoch);
        let verified = signed.verify_policy(&policy)?;
        // 2) 差异闸门：起点必须是我现在的 live，终点必须是签名里承诺的状态。
        if delta.agent() != &self.plan.agent {
            return Err(CoreError::InvalidDid);
        }
        if delta.id()? != self.plan.delta_id {
            return Err(CoreError::InvalidVersion);
        }
        let live = node.live_snapshot()?;
        if live.content_root()? != self.plan.base_content_root {
            // 我的起点不是这笔迁移的起点：竞争（别人的迁移先落地了）。
            return Err(CoreError::InvalidSignature);
        }
        if delta.from_content_root() != self.plan.base_content_root
            || delta.to_content_root() != self.plan.target_content_root
        {
            return Err(CoreError::InvalidVersion);
        }
        let staged = delta.apply_to(&live)?;
        if staged.content_root()? != verified.content_root() {
            return Err(CoreError::InvalidSignature);
        }

        faults.trip(FaultPoint::BeforeStaging)?;

        let previous = node.head()?;
        let staged_gen = previous + 1;
        node.write_generation(staged_gen, &staged)?;

        faults.trip(FaultPoint::AfterStaging)?;

        // 读回校验：影子代必须已经是一份**完整且内容正确**的状态。
        let readback = node.read_generation(staged_gen)?;
        if readback.content_root()? != self.plan.target_content_root {
            return Err(CoreError::InvalidSignature);
        }
        node.set_intent(json!({
            "tx": self.plan.tx,
            "staged": staged_gen,
            "previous": previous,
            "target": self.plan.target_content_root,
        }))?;

        self.previous_generation = Some(previous);
        self.staged_generation = Some(staged_gen);
        self.phase = Phase::Prepared;
        Ok(PrepareReceipt {
            tx: self.plan.tx.clone(),
            staged_generation: staged_gen,
            previous_generation: previous,
            blocks: readback.blocks().len(),
            target_content_root: readback.content_root()?,
        })
    }

    /// **commit**：单次切换 head。之前的 live 是 base，之后是 target，没有中间态。
    pub fn commit<S: StateStore>(
        &mut self,
        node: &mut NodeStore<S>,
        faults: &mut FaultInjector,
    ) -> CoreResult<CommitReceipt> {
        self.expect(Phase::Prepared)?;
        let staged = self.staged_generation.ok_or(CoreError::InvalidVersion)?;
        // 切换前再读一次影子代：防止「prepare 之后有人动了暂存区」。
        let readback = node.read_generation(staged)?;
        if readback.content_root()? != self.plan.target_content_root {
            return Err(CoreError::InvalidSignature);
        }
        faults.trip(FaultPoint::BeforeFlip)?;
        node.flip_head(staged)?;
        faults.trip(FaultPoint::AfterFlip)?;
        let live = node.live_content_root()?;
        if live != self.plan.target_content_root {
            return Err(CoreError::InvalidSignature);
        }
        self.phase = Phase::Committed;
        Ok(CommitReceipt {
            tx: self.plan.tx.clone(),
            generation: staged,
            live_content_root: live,
        })
    }

    /// **confirm**：确认目标状态已生效，清理旧代与意图记录。
    pub fn confirm<S: StateStore>(
        &mut self,
        node: &mut NodeStore<S>,
        faults: &mut FaultInjector,
    ) -> CoreResult<ConfirmReceipt> {
        self.expect(Phase::Committed)?;
        let generation = self.staged_generation.ok_or(CoreError::InvalidVersion)?;
        let live = node.live_content_root()?;
        if live != self.plan.target_content_root {
            return Err(CoreError::InvalidSignature);
        }
        faults.trip(FaultPoint::BeforeCleanup)?;
        node.clear_intent()?;
        let cleanup_removed = node.gc_except(generation)?;
        self.phase = Phase::Confirmed;
        Ok(ConfirmReceipt {
            tx: self.plan.tx.clone(),
            generation,
            cleanup_removed,
            live_content_root: live,
        })
    }

    /// **rollback**：幂等。confirm 之前任何时刻都必须能把 live 恢复到 base 且不留孤儿代。
    ///
    /// 刻意做成**状态驱动**而不是阶段驱动：故障可能正好落在「head 已切换、阶段还没推进」
    /// 的那一瞬间（真实世界里的崩溃就长这样）。所以回滚只相信两件事——
    /// head 现在指向谁、影子代是哪一个——而不是内存里的阶段字段。
    pub fn rollback<S: StateStore>(&mut self, node: &mut NodeStore<S>) -> CoreResult<usize> {
        if self.phase == Phase::Confirmed {
            // 已确认的迁移不能被回滚：那等于让两个节点对「谁是权威」产生分歧。
            return Err(CoreError::InvalidVersion);
        }
        if let (Some(staged), Some(previous)) = (self.staged_generation, self.previous_generation) {
            if node.head()? == staged {
                // 已切换但未确认 → 把 head 拨回去，再删掉影子代。
                node.flip_head(previous)?;
            }
            crate::store::clear_namespace(node.store_mut(), &generation_prefix(staged))?;
        }
        node.clear_intent()?;
        // 只有**真的写过影子代**的迁移才负有「恢复回 base」的义务。
        // 在写之前就被拒的迁移（比如 base 不匹配）不该也没必要把节点掰回 base——
        // 那不是回滚，那是破坏。
        let staged_something = self.staged_generation.is_some();
        self.failed_at = Some(self.phase);
        self.phase = Phase::RolledBack;
        let live = node.live_content_root()?;
        if staged_something && live != self.plan.base_content_root {
            // 回滚后 live 必须回到 base；不满足就是协议 bug，直接报错而不是粉饰。
            return Err(CoreError::InvalidSignature);
        }
        Ok(node.gc_except(node.head()?)?)
    }

    fn expect(&self, phase: Phase) -> CoreResult<()> {
        if self.phase == phase {
            Ok(())
        } else {
            // 阶段顺序错误 = 协议冲突（内核 RefusalCode::Conflict 语义）。
            Err(CoreError::InvalidVersion)
        }
    }

    pub fn failed_at(&self) -> Option<Phase> {
        self.failed_at
    }
}

/// 迁移结果：成功确认，或回滚（附带去过的阶段与原因）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum MigrationOutcome {
    Confirmed {
        report: MigrationReport,
    },
    RolledBack {
        tx: String,
        failed_at: String,
        reason: CoreError,
        live_content_root: String,
        /// 回滚后 live 是否恰好等于 base（必须为 true）。
        state_restored: bool,
        orphans: usize,
    },
}

impl MigrationOutcome {
    pub fn is_confirmed(&self) -> bool {
        matches!(self, MigrationOutcome::Confirmed { .. })
    }

    pub fn live_content_root(&self) -> &str {
        match self {
            MigrationOutcome::Confirmed { report } => &report.target_content_root,
            MigrationOutcome::RolledBack {
                live_content_root, ..
            } => live_content_root,
        }
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// 成功迁移的报告。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationReport {
    pub tx: String,
    pub base_content_root: String,
    pub target_content_root: String,
    pub blocks: usize,
    pub previous_generation: u64,
    pub final_generation: u64,
    pub cleanup_removed: usize,
    pub live_content_root: String,
}

/// 跑完一次完整迁移：任何一步失败 → 回滚 → 返回 `RolledBack` 证据。
pub fn migrate<S: StateStore>(
    node: &mut NodeStore<S>,
    plan: MigrationPlan,
    delta: &StateDelta,
    signed: &SignedSnapshot,
    faults: &mut FaultInjector,
) -> CoreResult<MigrationOutcome> {
    let tx = plan.tx.clone();
    let base = plan.base_content_root.clone();
    let mut migration = Migration::begin(plan);
    let attempt = (|| -> CoreResult<(PrepareReceipt, CommitReceipt, ConfirmReceipt)> {
        let prepared = migration.prepare(node, delta, signed, faults)?;
        let committed = migration.commit(node, faults)?;
        let confirmed = migration.confirm(node, faults)?;
        Ok((prepared, committed, confirmed))
    })();
    match attempt {
        Ok((prepared, _, confirmed)) => Ok(MigrationOutcome::Confirmed {
            report: MigrationReport {
                tx,
                base_content_root: base,
                target_content_root: confirmed.live_content_root.clone(),
                blocks: prepared.blocks,
                previous_generation: prepared.previous_generation,
                final_generation: confirmed.generation,
                cleanup_removed: confirmed.cleanup_removed,
                live_content_root: confirmed.live_content_root,
            },
        }),
        Err(reason) => {
            let failed_at = migration.phase();
            // 回滚本身失败也不允许掩盖原始故障：先回滚，再如实报告。
            let rollback_result = migration.rollback(node);
            let live = node.live_content_root()?;
            let orphans = node.orphan_generations(node.head()?).map(|v| v.len())?;
            if rollback_result.is_err() {
                return Err(reason);
            }
            Ok(MigrationOutcome::RolledBack {
                tx,
                failed_at: failed_at.as_str().to_string(),
                reason,
                state_restored: live == base,
                live_content_root: live,
                orphans,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::StateDelta;
    use crate::signed::SignedSnapshot;
    use crate::snapshot::{StateBlock, StateZone};
    use crate::store::MemoryStore;
    use serde_json::json;

    fn keys(seed: u8) -> au4a_core::AgentKeys {
        au4a_core::AgentKeys::from_seed(&[seed; 32])
    }

    fn state(v: i64) -> Vec<StateBlock> {
        vec![
            StateBlock::new(StateZone::Fs, "/a", json!({"v": v})).unwrap(),
            StateBlock::new(StateZone::Memory, "m", json!(v)).unwrap(),
            StateBlock::new(StateZone::Context, "goal", json!({"v": v})).unwrap(),
        ]
    }

    struct Fixture {
        base: StateSnapshot,
        target: StateSnapshot,
        delta: StateDelta,
        signed: SignedSnapshot,
        plan: MigrationPlan,
    }

    fn fixture(seed: u8) -> Fixture {
        let k = keys(seed);
        let base = StateSnapshot::capture(&k.did(), "node-a", 1, state(1)).unwrap();
        let target = StateSnapshot::capture(&k.did(), "node-a", 1, state(2)).unwrap();
        let delta = StateDelta::between(&base, &target).unwrap();
        let signed = SignedSnapshot::sign(target.clone(), &k).unwrap();
        let plan = MigrationPlan::new(
            &k.did(),
            &NodeId::new("node-a").unwrap(),
            &NodeId::new("node-b").unwrap(),
            base.content_root().unwrap(),
            target.content_root().unwrap(),
            delta.id().unwrap(),
            1,
        )
        .unwrap();
        Fixture {
            base,
            target,
            delta,
            signed,
            plan,
        }
    }

    fn node(st: &Fixture, seed: u8) -> NodeStore<MemoryStore> {
        let k = keys(seed);
        let mut n = NodeStore::open(NodeId::new("node-b").unwrap(), k.did(), MemoryStore::new())
            .unwrap();
        n.install(&st.base, true).unwrap();
        n
    }

    #[test]
    fn a_happy_path_migration_confirms_and_cleans_up() {
        let f = fixture(1);
        let mut n = node(&f, 1);
        let outcome = migrate(&mut n, f.plan.clone(), &f.delta, &f.signed, &mut FaultInjector::none())
            .unwrap();
        assert!(outcome.is_confirmed());
        assert_eq!(n.live_content_root().unwrap(), f.target.content_root().unwrap());
        assert!(n.orphan_generations(n.head().unwrap()).unwrap().is_empty());
        assert!(n.intent().unwrap().is_none());
    }

    #[test]
    fn a_fault_at_any_point_rolls_back_to_the_base_with_no_orphans() {
        for point in FaultPoint::ALL {
            let f = fixture(2);
            let mut n = node(&f, 2);
            let outcome = migrate(
                &mut n,
                f.plan.clone(),
                &f.delta,
                &f.signed,
                &mut FaultInjector::at(point),
            )
            .unwrap();
            match outcome {
                MigrationOutcome::RolledBack {
                    state_restored,
                    orphans,
                    reason,
                    ..
                } => {
                    assert!(state_restored, "{point:?}: 回滚后 live 不是 base");
                    assert_eq!(orphans, 0, "{point:?}: 留下孤儿代");
                    assert_eq!(reason, CoreError::Overflow);
                }
                MigrationOutcome::Confirmed { .. } => panic!("{point:?}: 注入的故障没有生效"),
            }
            assert_eq!(n.live_content_root().unwrap(), f.base.content_root().unwrap());
            assert!(n.intent().unwrap().is_none());
            assert!(n.orphan_generations(n.head().unwrap()).unwrap().is_empty());
        }
    }

    #[test]
    fn live_is_never_a_mixture_of_base_and_target() {
        let f = fixture(3);
        let mut n = node(&f, 3);
        let base_root = f.base.content_root().unwrap();
        let target_root = f.target.content_root().unwrap();
        // 逐点注入并逐步观察 live：任何时刻都只能等于 base 或 target。
        for point in FaultPoint::ALL {
            let mut n2 = node(&f, 3);
            let _ = migrate(
                &mut n2,
                f.plan.clone(),
                &f.delta,
                &f.signed,
                &mut FaultInjector::at(point),
            )
            .unwrap();
            let live = n2.live_content_root().unwrap();
            assert!(
                live == base_root || live == target_root,
                "{point:?}: live 是第三种状态 {live}"
            );
            // 目标节点上的块数也必须与某一份完整状态一致。
            let snap = n2.live_snapshot().unwrap();
            assert_eq!(snap.blocks().len(), 3, "{point:?}: 块数不完整");
        }
        let ok = migrate(&mut n, f.plan.clone(), &f.delta, &f.signed, &mut FaultInjector::none())
            .unwrap();
        assert!(ok.is_confirmed());
    }

    #[test]
    fn phase_order_cannot_be_skipped_or_repeated() {
        let f = fixture(4);
        let mut n = node(&f, 4);
        let mut m = Migration::begin(f.plan.clone());
        // commit 在 prepare 之前。
        assert_eq!(
            m.commit(&mut n, &mut FaultInjector::none()),
            Err(CoreError::InvalidVersion)
        );
        assert_eq!(
            m.confirm(&mut n, &mut FaultInjector::none()),
            Err(CoreError::InvalidVersion)
        );
        m.prepare(&mut n, &f.delta, &f.signed, &mut FaultInjector::none())
            .unwrap();
        // prepare 两次。
        assert_eq!(
            m.prepare(&mut n, &f.delta, &f.signed, &mut FaultInjector::none()),
            Err(CoreError::InvalidVersion)
        );
        m.commit(&mut n, &mut FaultInjector::none()).unwrap();
        // confirm 两次。
        m.confirm(&mut n, &mut FaultInjector::none()).unwrap();
        assert_eq!(
            m.confirm(&mut n, &mut FaultInjector::none()),
            Err(CoreError::InvalidVersion)
        );
    }

    #[test]
    fn rollback_is_idempotent() {
        let f = fixture(5);
        let mut n = node(&f, 5);
        let mut m = Migration::begin(f.plan.clone());
        m.prepare(&mut n, &f.delta, &f.signed, &mut FaultInjector::none())
            .unwrap();
        m.rollback(&mut n).unwrap();
        m.rollback(&mut n).unwrap();
        assert_eq!(m.phase(), Phase::RolledBack);
        assert_eq!(n.live_content_root().unwrap(), f.base.content_root().unwrap());
    }

    #[test]
    fn a_wrong_base_is_refused_before_any_write() {
        let f = fixture(6);
        let mut n = node(&f, 6);
        // 先把 live 推到目标状态：这笔迁移的 base 已经不是我的起点了。
        n.install(&f.target, true).unwrap();
        let before = n.head().unwrap();
        let mut m = Migration::begin(f.plan.clone());
        assert_eq!(
            m.prepare(&mut n, &f.delta, &f.signed, &mut FaultInjector::none()),
            Err(CoreError::InvalidSignature)
        );
        assert_eq!(n.head().unwrap(), before, "拒绝的迁移不得动 head");
        assert!(n.orphan_generations(before).unwrap().is_empty());
    }

    #[test]
    fn a_forged_signed_snapshot_blocks_the_prepare() {
        let f = fixture(7);
        let mut n = node(&f, 7);
        let mut value = f.signed.to_value().unwrap();
        value["sig"] = json!("ff".repeat(64));
        assert!(SignedSnapshot::from_value(&value).is_err());
        // 用另一份自洽但内容不同的签名快照：语义闸门必须拦住它。
        let k = keys(7);
        let other = SignedSnapshot::sign(
            StateSnapshot::capture(&k.did(), "node-a", 1, state(9)).unwrap(),
            &k,
        )
        .unwrap();
        let mut m = Migration::begin(f.plan.clone());
        assert_eq!(
            m.prepare(&mut n, &f.delta, &other, &mut FaultInjector::none()),
            Err(CoreError::InvalidSignature)
        );
    }

    #[test]
    fn generation_writes_may_not_touch_the_live_generation() {
        let f = fixture(8);
        let mut n = node(&f, 8);
        let head = n.head().unwrap();
        assert_eq!(
            n.write_generation(head, &f.target),
            Err(CoreError::DuplicateAgent)
        );
        assert_eq!(n.live_content_root().unwrap(), f.base.content_root().unwrap());
    }

    #[test]
    fn the_plan_tx_id_binds_every_field() {
        let f = fixture(9);
        let k = keys(9);
        let other = MigrationPlan::new(
            &k.did(),
            &NodeId::new("node-a").unwrap(),
            &NodeId::new("node-c").unwrap(),
            f.plan.base_content_root.clone(),
            f.plan.target_content_root.clone(),
            f.plan.delta_id.clone(),
            f.plan.epoch,
        )
        .unwrap();
        assert_ne!(other.tx, f.plan.tx);
        let signed_plan = serde_json::to_value(&f.plan).unwrap();
        assert_eq!(
            serde_json::from_value::<MigrationPlan>(signed_plan).unwrap(),
            f.plan
        );
    }
}
