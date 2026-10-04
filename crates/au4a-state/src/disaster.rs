//! 灾难恢复（v1.3.10）：备份 → 源丢失 → 2PC 重建 → 增量续跑。
//!
//! 这一版把前面九版全部拉进一个**最坏情况**里演练：
//!
//! ```text
//! 1. 备份      :  Agent 亲笔签名 + 内容寻址地写进独立介质（备份介质不是源介质）
//! 2. 源丢失    :  源节点介质被丢弃；唯一还能证明「我是谁」的是备份与签名
//! 3. 2PC 重建  :  在全新节点上从备份恢复出 base，再用两阶段提交安装最新状态
//! 4. 增量续跑  :  重建期间的传输断线 → 只补缺口；第一次 2PC 注入故障 → 回滚后重试
//! ```
//!
//! 关键设计：**备份必须能被独立验证**。备份不是「一份文件副本」，而是
//! 「Agent 签名的内容寻址对象」——损坏一个字节，恢复时就会被拒，而不是悄悄恢复出一个
//! 似是而非的 Agent。这一条比「备份成功」重要得多。

use au4a_core::{AgentKeys, CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::diff::StateDelta;
use crate::integrity::{audit_node, compare};
use crate::perf::{self, WorkCounter};
use crate::recovery::{migrate, FaultInjector, FaultPoint, MigrationPlan, MigrationOutcome, NodeStore};
use crate::signed::{SignedSnapshot, SnapshotPolicy};
use crate::snapshot::StateSnapshot;
use crate::store::{read_snapshot, write_snapshot, MemoryStore, StateStore};
use crate::transfer::{pull_into_session, send_chunks, LocalNetwork, NodeId};
use crate::udos;

/// 备份介质的前缀：备份与源在**不同命名空间**，从而「源丢失」不会带走备份。
pub const BACKUP_PREFIX: &str = "backup:";

/// 从任意介质重建签名快照并独立验证（备份、异地副本、UDOS 对象层都用这一条路径）。
///
/// 拒绝的三种情况：
/// 1. 介质内容与签名里的内容根不符（被改过 / 少了块 / 多了块）→ `InvalidSignature`；
/// 2. 签名本身无效或版本不符 → `InvalidSignature` / `InvalidVersion`；
/// 3. 主体不是我要的 Agent → `InvalidDid`。
pub fn verify_media<S: StateStore + ?Sized>(
    store: &S,
    prefix: &str,
    signed: &SignedSnapshot,
    policy: &SnapshotPolicy,
) -> CoreResult<StateSnapshot> {
    let rebuilt = read_snapshot(
        store,
        prefix,
        signed.snapshot().agent(),
        signed.snapshot().source_node(),
        signed.snapshot().epoch(),
    )?;
    if rebuilt.content_root()? != signed.snapshot().content_root()? {
        return Err(CoreError::InvalidSignature);
    }
    let verified = signed.verify_policy(policy)?;
    Ok(verified.into_snapshot())
}

/// 一份可独立验证的备份。
#[derive(Clone, Debug)]
pub struct Backup {
    store: MemoryStore,
    prefix: String,
    signed: SignedSnapshot,
    object_id: String,
}

impl Backup {
    /// 备份一份快照：内容寻址写入介质，并由 Agent 亲笔签名。
    pub fn create(agent: &AgentKeys, snapshot: &StateSnapshot) -> CoreResult<Self> {
        if snapshot.agent() != &agent.did() {
            return Err(CoreError::InvalidDid);
        }
        let mut store = MemoryStore::new();
        write_snapshot(&mut store, BACKUP_PREFIX, snapshot)?;
        let signed = SignedSnapshot::sign(snapshot.clone(), agent)?;
        let object_id = udos::UdosObject::validate(&udos::snapshot_to_object(snapshot)?)?.object_id;
        Ok(Self {
            store,
            prefix: BACKUP_PREFIX.to_string(),
            signed,
            object_id,
        })
    }

    pub fn object_id(&self) -> &str {
        &self.object_id
    }

    pub fn signed(&self) -> &SignedSnapshot {
        &self.signed
    }

    pub fn content_root(&self) -> &str {
        self.signed.snapshot().root()
    }

    pub fn keys(&self) -> usize {
        self.store.len()
    }

    /// 从介质重建并**独立验证**：内容根必须与签名里的内容根一致，签名必须过策略。
    /// 备份被改过一个字节 → 这里返回 `Err`，而不是恢复出一个「差不多」的 Agent。
    pub fn verify(&self, policy: &SnapshotPolicy) -> CoreResult<StateSnapshot> {
        verify_media(&self.store, &self.prefix, &self.signed, policy)
    }

    /// 把备份恢复成一个新节点的 base（影子代安装）。
    pub fn restore_into<S: StateStore>(
        &self,
        node: &mut NodeStore<S>,
        policy: &SnapshotPolicy,
    ) -> CoreResult<u64> {
        let snapshot = self.verify(policy)?;
        node.install(&snapshot, true)
    }

    /// 只用于测试/演练：在备份介质上做一次篡改，证明「坏备份会被拒」。
    #[cfg(test)]
    fn tamper_with_value(&mut self, key: &str, value: Value) -> CoreResult<()> {
        self.store.put(&format!("{}{key}", self.prefix), value)
    }
}

/// 演练参数。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrillOptions {
    pub chunk_ops: usize,
    /// 第一批成功送达的块数（其余在断线中丢失）。
    pub first_batch_chunks: usize,
    /// 重建后的第一次 2PC 是否注入故障（用于证明「回滚 + 重试」可行）。
    pub fault_then_retry: bool,
    pub backup_epoch: u64,
    pub rebuild_epoch: u64,
}

impl Default for DrillOptions {
    fn default() -> Self {
        Self {
            chunk_ops: 2,
            first_batch_chunks: 2,
            fault_then_retry: true,
            backup_epoch: 1,
            rebuild_epoch: 2,
        }
    }
}

/// 演练报告：每一步的证据都在这里。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrillReport {
    pub track: String,
    pub agent: String,
    pub to_node: String,
    /// 备份对象 id（`sha256:<hex>`）。
    pub backup_object_id: String,
    pub backup_blocks: usize,
    pub backup_content_root: String,
    /// 从备份恢复出的 base 是否与源逐字节一致。
    pub backup_verified: bool,
    /// 源介质是否已被丢弃（演练里恒为 true，这里如实记录）。
    pub source_lost: bool,
    /// 第一次 2PC 的结果（注入故障 → 回滚）。
    pub rebuild_first_attempt: String,
    pub rebuild_first_live_root: String,
    pub rebuild_first_clean: bool,
    /// 重试后的 2PC 结果。
    pub rebuild_committed: bool,
    /// 两次尝试的失败原因（成功则写 "confirmed"），用于诊断而不是猜。
    pub rebuild_first_reason: String,
    pub rebuild_committed_reason: String,
    pub rebuild_live_root: String,
    pub rebuild_orphans: usize,
    /// 增量续跑。
    pub resumed_from: usize,
    pub dropped_frames: usize,
    pub ops_skipped: u64,
    pub duplicates: usize,
    /// 最终状态。
    pub source_content_root: String,
    pub final_content_root: String,
    pub identical_to_source: bool,
    pub consistency_clean: bool,
    pub consistency_summary: String,
    pub node_findings: usize,
    pub udos_object_id: String,
    pub work: WorkCounter,
    pub evidence_grade: String,
}

impl DrillReport {
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// 演练成立的判据：备份可验证 + 第一次（若注入故障）回滚干净 + 重试提交 +
    /// 最终与源一致 + 节点与一致性检查都干净。
    pub fn is_ok(&self) -> bool {
        self.backup_verified
            && self.rebuild_committed
            && self.identical_to_source
            && self.consistency_clean
            && self.node_findings == 0
            && self.rebuild_orphans == 0
            && self.rebuild_first_clean
    }
}

/// 跑一次灾难恢复演练。
pub fn run_drill(
    agent: &AgentKeys,
    source: &StateSnapshot,
    evolved: &StateSnapshot,
    to_node: &NodeId,
    options: &DrillOptions,
) -> CoreResult<DrillReport> {
    if source.agent() != &agent.did() || evolved.agent() != &agent.did() {
        return Err(CoreError::InvalidDid);
    }
    if options.chunk_ops == 0 {
        return Err(CoreError::Encoding);
    }
    let mut work = WorkCounter::new();

    // 1) 备份：内容寻址 + Agent 亲笔签名。
    let backup = Backup::create(agent, source)?;
    let source_policy = SnapshotPolicy::for_agent(agent.did())
        .expecting_content_root(source.content_root()?)
        .with_min_epoch(source.epoch());
    let recovered = backup.verify(&source_policy)?;
    let backup_verified = recovered.content_root()? == source.content_root()?;
    if !backup_verified {
        return Err(CoreError::InvalidSignature);
    }
    work.store_writes += backup.keys() as u64;

    // 2) 源丢失：源介质被丢弃。唯一能证明「我是谁」的只剩备份与签名。
    //    （演练里我们用「不再引用源节点存储」来建模：源 StateStore 被 drop。）
    let source_lost = true;

    // 3) 2PC 重建：新节点从备份恢复出 base。
    let mut node = NodeStore::open(to_node.clone(), agent.did(), MemoryStore::new())?;
    backup.restore_into(&mut node, &source_policy)?;
    let rebuilt_base_root = node.live_content_root()?;
    if rebuilt_base_root != source.content_root()? {
        return Err(CoreError::InvalidSignature);
    }

    // 4) 增量续跑：丢失期间 Agent 继续工作，把 source → evolved 的差异传过去（中途断线）。
    let delta = StateDelta::between(source, evolved)?;
    let from_node = NodeId::new(source.source_node())?;
    let mut net = LocalNetwork::new(&[from_node.clone(), to_node.clone()]);
    let chunks = delta.chunked(options.chunk_ops)?;
    let mut resumed_from = 0usize;
    let mut dropped = 0usize;
    let mut duplicates = 0usize;
    if !chunks.is_empty() {
        let first = options.first_batch_chunks.clamp(1, chunks.len());
        send_chunks(&mut net, &from_node, to_node, &delta, options.chunk_ops, 0, first)?;
        let (session, _, _) = pull_into_session(&mut net, to_node, None)?;
        resumed_from = session.resume_from();
        // 第二批发出但在途丢失。
        send_chunks(
            &mut net,
            &from_node,
            to_node,
            &delta,
            options.chunk_ops,
            first,
            usize::MAX,
        )?;
        dropped = net.drop_pending(to_node)?;
        // 续跑：只补缺口。
        send_chunks(
            &mut net,
            &from_node,
            to_node,
            &delta,
            options.chunk_ops,
            resumed_from,
            usize::MAX,
        )?;
        let (session, _, dup) = pull_into_session(&mut net, to_node, Some(session))?;
        duplicates = dup;
        if !session.is_complete(chunks.len()) {
            return Err(CoreError::FrameTruncated);
        }
        let rebuilt = session.assemble(chunks.len(), agent.did())?;
        if rebuilt.id()? != delta.id()? {
            return Err(CoreError::InvalidSignature);
        }
        // 核算省下的工作量。
        let ops_per_chunk = delta.op_count().div_ceil(chunks.len());
        let (_, skipped) = perf::resume_savings(delta.op_count(), resumed_from, ops_per_chunk);
        work.ops_skipped += skipped;
        work.ops_transferred += chunks
            .iter()
            .skip(resumed_from)
            .map(|c| c.op_count() as u64)
            .sum::<u64>();
    }

    // 5) 2PC：第一次注入故障（证明回滚 + 重试可行），第二次正常提交。
    let signed = SignedSnapshot::sign(evolved.clone(), agent)?;
    let plan = MigrationPlan::new(
        &agent.did(),
        &from_node,
        to_node,
        source.content_root()?,
        evolved.content_root()?,
        delta.id()?,
        options.rebuild_epoch,
    )?;
    let first = if options.fault_then_retry {
        migrate(
            &mut node,
            plan.clone(),
            &delta,
            &signed,
            &mut FaultInjector::at(FaultPoint::AfterStaging),
        )?
    } else {
        // 不注入故障时用一次正常提交作为「第一次」。
        migrate(&mut node, plan.clone(), &delta, &signed, &mut FaultInjector::none())?
    };
    let first_attempt = match &first {
        MigrationOutcome::Confirmed { .. } => "confirmed".to_string(),
        MigrationOutcome::RolledBack { failed_at, .. } => format!("rolled_back@{failed_at}"),
    };
    let reason_of = |outcome: &MigrationOutcome| match outcome {
        MigrationOutcome::Confirmed { .. } => "confirmed".to_string(),
        MigrationOutcome::RolledBack { reason, .. } => format!("{reason:?}"),
    };
    let first_reason = reason_of(&first);
    let first_live = node.live_content_root()?;
    let first_clean = match &first {
        MigrationOutcome::Confirmed { .. } => true,
        MigrationOutcome::RolledBack {
            state_restored,
            orphans,
            ..
        } => *state_restored && *orphans == 0 && first_live == source.content_root()?,
    };

    // 重试：回滚之后节点必须还能接受同一笔迁移（这就是「可恢复」的实质）。
    let committed = if options.fault_then_retry {
        migrate(&mut node, plan, &delta, &signed, &mut FaultInjector::none())?
    } else {
        first
    };
    let committed_ok = committed.is_confirmed();
    let live = node.live_snapshot()?;
    let audit = audit_node(&node)?;
    let report = compare(evolved, &live)?;

    let udos_object = udos::UdosObject::validate(&udos::snapshot_to_object(&live)?)?;

    Ok(DrillReport {
        track: crate::TRACK.to_string(),
        agent: agent.did().as_str().to_string(),
        to_node: to_node.as_str().to_string(),
        backup_object_id: backup.object_id().to_string(),
        backup_blocks: backup.keys(),
        backup_content_root: source.content_root()?,
        backup_verified,
        source_lost,
        rebuild_first_attempt: first_attempt,
        rebuild_first_live_root: first_live,
        rebuild_first_clean: first_clean,
        rebuild_committed: committed_ok,
        rebuild_first_reason: first_reason,
        rebuild_committed_reason: reason_of(&committed),
        rebuild_live_root: live.content_root()?,
        rebuild_orphans: audit.orphan_generations.len(),
        resumed_from,
        dropped_frames: dropped,
        ops_skipped: work.ops_skipped,
        duplicates,
        source_content_root: evolved.content_root()?,
        final_content_root: live.content_root()?,
        identical_to_source: report.is_clean(),
        consistency_clean: report.is_clean(),
        consistency_summary: crate::integrity::summarize(&report),
        node_findings: audit.findings.len(),
        udos_object_id: udos_object.object_id,
        work,
        evidence_grade: "verified".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{StateBlock, StateZone};
    use serde_json::json;

    fn keys(seed: u8) -> AgentKeys {
        AgentKeys::from_seed(&[seed; 32])
    }

    fn state(seed: u8, v: i64) -> StateSnapshot {
        let blocks = vec![
            StateBlock::new(StateZone::Fs, "/a", json!({"v": v})).unwrap(),
            StateBlock::new(StateZone::Memory, "m", json!(v)).unwrap(),
            StateBlock::new(StateZone::Context, "goal", json!({"v": v})).unwrap(),
        ];
        // epoch 跟着版本走：策略里的 min_epoch 才有意义（重建发生在源之后）。
        StateSnapshot::capture(&keys(seed).did(), "node-a", v as u64, blocks).unwrap()
    }

    #[test]
    fn a_full_drill_survives_the_loss_of_the_source() {
        let k = keys(1);
        let report = run_drill(
            &k,
            &state(1, 1),
            &state(1, 2),
            &NodeId::new("node-b").unwrap(),
            &DrillOptions::default(),
        )
        .unwrap();
        assert!(report.is_ok(), "{report:#?}");
        assert!(report.backup_verified);
        assert!(report.source_lost);
        assert!(report.rebuild_first_clean, "第一次注入故障必须回滚干净");
        assert!(report.rebuild_committed);
        assert_eq!(report.final_content_root, report.source_content_root);
        assert!(report.identical_to_source);
        assert_eq!(report.rebuild_orphans, 0);
        assert_eq!(report.node_findings, 0);
        assert!(report.udos_object_id.starts_with("sha256:"));
    }

    #[test]
    fn a_corrupted_backup_is_refused_at_restore_time() {
        let k = keys(2);
        let source = state(2, 1);
        let mut backup = Backup::create(&k, &source).unwrap();
        let policy = SnapshotPolicy::for_agent(k.did())
            .expecting_content_root(source.content_root().unwrap());
        assert!(backup.verify(&policy).is_ok());
        // 备份介质里的一个块被改：恢复必须拒绝，而不是恢复出「差不多的 Agent」。
        backup
            .tamper_with_value("memory:m", json!({"v": 999}))
            .unwrap();
        assert_eq!(backup.verify(&policy), Err(CoreError::InvalidSignature));
    }

    #[test]
    fn a_backup_of_someone_elses_state_is_refused() {
        let a = keys(3);
        let b = keys(4);
        assert!(Backup::create(&b, &state(3, 1)).is_err());
        let backup = Backup::create(&a, &state(3, 1)).unwrap();
        let wrong_policy = SnapshotPolicy::for_agent(b.did());
        assert_eq!(backup.verify(&wrong_policy), Err(CoreError::InvalidDid));
    }

    #[test]
    fn the_drill_is_deterministic() {
        let run = || {
            let k = keys(5);
            run_drill(
                &k,
                &state(5, 1),
                &state(5, 4),
                &NodeId::new("node-b").unwrap(),
                &DrillOptions::default(),
            )
            .unwrap()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn the_drill_can_be_run_without_injecting_a_fault() {
        let k = keys(6);
        let options = DrillOptions {
            fault_then_retry: false,
            ..DrillOptions::default()
        };
        let report = run_drill(
            &k,
            &state(6, 1),
            &state(6, 2),
            &NodeId::new("node-b").unwrap(),
            &options,
        )
        .unwrap();
        assert!(report.rebuild_committed, "{report:#?}");
        assert!(report.identical_to_source, "{report:#?}");
        assert!(report.is_ok(), "{report:#?}");
    }
}
