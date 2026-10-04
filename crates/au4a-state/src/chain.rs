//! 迁移全链路（v1.3.8）：把前面七版的能力串成**一次调用**，并返回可机读的总结。
//!
//! 为什么要有这个模块：分散的能力各自被测试过，不等于**串起来**之后还成立。
//! 「快照 → 签名 → 分块传输（中途断线）→ 续跑 → 2PC 提交/回滚 → 一致性检查 → UDOS 导出」
//! 这条链上任何一处接口错配，单测都不会发现。灾难恢复演练（v1.3.10）与端到端演示
//! 都需要同一条链，所以它必须是产品代码，而不是测试里的脚手架。
//!
//! 链路严格按顺序，且**每一步的产物都被下一步校验**：
//! 签名过的内容根 → 传输的目标内容根 → 2PC 安装的 live 内容根 → 一致性报告的期望值，
//! 最终全部收敛到同一个哈希。这就是「Agent 迁移后还是同一个自己」的可执行定义。

use au4a_core::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::diff::StateDelta;
use crate::integrity::{audit_node, compare, ConsistencyReport, NodeAudit};
use crate::perf::{self, DeltaPlan, WorkCounter};
use crate::recovery::{migrate, FaultInjector, FaultPoint, MigrationOutcome, NodeStore};
use crate::signed::{SignedSnapshot, SnapshotPolicy, VerifiedSnapshot};
use crate::snapshot::StateSnapshot;
use crate::store::MemoryStore;
use crate::transfer::{pull_into_session, send_chunks, LocalNetwork, NodeId};
use crate::udos;

/// 链路参数（全部显式，保证可重放）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChainOptions {
    /// 每个传输块最多包含多少个操作。
    pub chunk_ops: usize,
    /// 第一批成功送达的块数（其余在断线中丢失）。
    pub first_batch_chunks: usize,
    /// 是否模拟断线（true 时第一批之后全部丢失，再续跑）。
    pub drop_rest: bool,
    /// 注入的 2PC 故障点（`None` 表示正常提交）。
    pub fault: Option<FaultPoint>,
    /// 逻辑时刻。
    pub epoch: u64,
}

impl Default for ChainOptions {
    fn default() -> Self {
        Self {
            chunk_ops: 2,
            first_batch_chunks: 2,
            drop_rest: true,
            fault: None,
            epoch: 1,
        }
    }
}

/// 链路报告：每一步的证据都在这里，且全部是确定性值。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainReport {
    pub track: String,
    pub agent: String,
    pub from_node: String,
    pub to_node: String,
    pub base_content_root: String,
    pub source_content_root: String,
    /// 迁移后目标节点上的状态内容根（若回滚则是 base）。
    pub live_content_root: String,
    /// 源与目标是否逐字节一致（规范 JSON 哈希相等）。
    pub identical: bool,
    /// 签名是否通过策略校验。
    pub signed_and_verified: bool,
    pub chunks: usize,
    pub dropped_frames: usize,
    pub resumed_from: usize,
    pub delta_ops: usize,
    pub committed: bool,
    /// 回滚后 live 是否恰好等于 base。
    pub rollback_clean: bool,
    pub consistency: ConsistencyReportView,
    pub node_audit_findings: usize,
    pub udos_object_id: String,
    pub udos_bundle_fingerprint: String,
    pub work: WorkCounter,
    pub delta_plan: DeltaPlan,
}

/// 一致性报告的扁平视图（避免报告嵌套过深）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsistencyReportView {
    pub clean: bool,
    pub changed_blocks: usize,
    pub summary: String,
}

impl From<&ConsistencyReport> for ConsistencyReportView {
    fn from(report: &ConsistencyReport) -> Self {
        Self {
            clean: report.is_clean(),
            changed_blocks: report.changed_blocks(),
            summary: crate::integrity::summarize(report),
        }
    }
}

impl ChainReport {
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// 链路是否全程成立：内容一致 + 签名验过 + （提交且一致干净）或（回滚且回到 base）。
    pub fn is_ok(&self) -> bool {
        self.signed_and_verified
            && if self.committed {
                self.identical && self.consistency.clean && self.node_audit_findings == 0
            } else {
                self.rollback_clean && self.live_content_root == self.base_content_root
            }
    }
}

/// 跑完一整条迁移链。
pub fn run_chain(
    agent: &au4a_core::AgentKeys,
    base: &StateSnapshot,
    target: &StateSnapshot,
    to_node: &NodeId,
    options: &ChainOptions,
) -> CoreResult<ChainReport> {
    if base.agent() != &agent.did() || target.agent() != &agent.did() {
        return Err(CoreError::InvalidDid);
    }
    if options.chunk_ops == 0 {
        return Err(CoreError::Encoding);
    }
    let from_node = NodeId::new(base.source_node())?;
    let mut work = WorkCounter::new();

    // 1) 差异与计划（只比摘要）。
    let delta = StateDelta::between(base, target)?;
    let delta_plan = perf::plan(base, target)?;

    // 2) 签名：源节点把「迁移时刻的状态」签字。
    let signed = SignedSnapshot::sign(target.clone(), agent)?;
    let policy = SnapshotPolicy::for_agent(agent.did())
        .expecting_content_root(target.content_root()?)
        .with_min_epoch(options.epoch);
    let verified: VerifiedSnapshot = signed.verify_policy(&policy)?;

    // 3) 传输：第一批送达 → 断线（第二批全丢）→ 从断点续跑。
    let mut net = LocalNetwork::new(&[from_node.clone(), to_node.clone()]);
    let chunks = delta.chunked(options.chunk_ops)?;
    let mut dropped = 0usize;
    let mut resumed_from = 0usize;
    let rebuilt_delta = if chunks.is_empty() {
        // 没有差异：没有块要传，空差异直接进入 2PC。
        delta.clone()
    } else {
        // 至少发一块——接收会话需要一块「开工砖」才能建立（这是分帧协议的语义）。
        let first = options.first_batch_chunks.clamp(1, chunks.len());
        send_chunks(
            &mut net,
            &from_node,
            to_node,
            &delta,
            options.chunk_ops,
            0,
            first,
        )?;
        work.ops_transferred += chunks
            .iter()
            .take(first)
            .map(|c| c.op_count() as u64)
            .sum::<u64>();
        let (session, _, _) = pull_into_session(&mut net, to_node, None)?;
        resumed_from = session.resume_from();
        if options.drop_rest {
            // 第二批发出但在途丢失（drop_pending 把它们清掉）。
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
        }
        // 续跑：从断点补发到结尾（没断线时等价于补发剩余块）。
        let resumed = send_chunks(
            &mut net,
            &from_node,
            to_node,
            &delta,
            options.chunk_ops,
            resumed_from,
            usize::MAX,
        )?;
        if options.drop_rest && dropped > 0 && resumed.frames == 0 {
            // 断了却什么都没补：这不是「续跑」，是撒谎。
            return Err(CoreError::FrameTruncated);
        }
        work.ops_transferred += chunks
            .iter()
            .skip(resumed_from)
            .map(|c| c.op_count() as u64)
            .sum::<u64>();
        let (session, _, _) = pull_into_session(&mut net, to_node, Some(session))?;
        if !session.is_complete(chunks.len()) {
            return Err(CoreError::FrameTruncated);
        }
        session.assemble(chunks.len(), agent.did())?
    };
    let rebuilt_target = rebuilt_delta.apply_moved(base, to_node.as_str(), options.epoch)?;

    // 4) 2PC：目标节点先装 base，再把 target 迁上去。
    let mut node = NodeStore::open(to_node.clone(), agent.did(), MemoryStore::new())?;
    node.install(base, true)?;
    let mut faults = match options.fault {
        Some(point) => FaultInjector::at(point),
        None => FaultInjector::none(),
    };
    let plan = crate::recovery::MigrationPlan::new(
        &agent.did(),
        &from_node,
        to_node,
        base.content_root()?,
        target.content_root()?,
        delta.id()?,
        options.epoch,
    )?;
    let outcome = migrate(&mut node, plan, &delta, &signed, &mut faults)?;
    let committed = outcome.is_confirmed();
    let live = node.live_snapshot()?;
    let live_content_root = live.content_root()?;

    // 5) 一致性检查（提交看 target，回滚看 base）。
    let expected = if committed { target } else { base };
    let report = compare(expected, &live)?;
    let audit: NodeAudit = audit_node(&node)?;
    let rollback_clean = if committed {
        false
    } else {
        match &outcome {
            MigrationOutcome::RolledBack {
                state_restored,
                orphans,
                ..
            } => *state_restored && *orphans == 0,
            MigrationOutcome::Confirmed { .. } => false,
        }
    };

    // 6) 传输重建的状态必须与源一致（无论 2PC 是否提交，重建本身必须正确）。
    let identical = rebuilt_target.content_root()? == verified.content_root()
        && live_content_root
            == if committed {
                target.content_root()?
            } else {
                base.content_root()?
            };

    // 7) UDOS 导出。
    let udos_object = udos::snapshot_to_object(target)?;
    let udos_validated = udos::UdosObject::validate(&udos_object)?;
    let bundle = udos::context_to_transfer_bundle(target)?;

    let chunk_count = chunks.len();
    let ops_per_chunk = if chunk_count > 0 {
        delta.op_count().div_ceil(chunk_count)
    } else {
        0
    };
    let (_, skipped) = perf::resume_savings(delta.op_count(), resumed_from, ops_per_chunk);
    work.ops_skipped += skipped;
    work.blocks_materialized += delta.set().len() as u64;

    Ok(ChainReport {
        track: crate::TRACK.to_string(),
        agent: agent.did().as_str().to_string(),
        from_node: from_node.as_str().to_string(),
        to_node: to_node.as_str().to_string(),
        base_content_root: base.content_root()?,
        source_content_root: target.content_root()?,
        live_content_root,
        identical,
        signed_and_verified: true,
        chunks: chunk_count,
        dropped_frames: dropped,
        resumed_from,
        delta_ops: delta.op_count(),
        committed,
        rollback_clean,
        consistency: ConsistencyReportView::from(&report),
        node_audit_findings: audit.findings.len(),
        udos_object_id: udos_validated.object_id,
        udos_bundle_fingerprint: udos::bundle_fingerprint(&bundle)?,
        work,
        delta_plan,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{StateBlock, StateZone};
    use serde_json::json;

    fn keys(seed: u8) -> au4a_core::AgentKeys {
        au4a_core::AgentKeys::from_seed(&[seed; 32])
    }

    fn state(seed: u8, v: i64) -> StateSnapshot {
        let blocks = vec![
            StateBlock::new(StateZone::Fs, "/a", json!({"v": v})).unwrap(),
            StateBlock::new(StateZone::Memory, "m", json!(v)).unwrap(),
            StateBlock::new(StateZone::Context, "goal", json!({"v": v})).unwrap(),
        ];
        StateSnapshot::capture(&keys(seed).did(), "node-a", 1, blocks).unwrap()
    }

    #[test]
    fn the_whole_chain_is_green_end_to_end() {
        let k = keys(1);
        let report = run_chain(
            &k,
            &state(1, 1),
            &state(1, 2),
            &NodeId::new("node-b").unwrap(),
            &ChainOptions::default(),
        )
        .unwrap();
        assert!(report.is_ok(), "{report:?}");
        assert!(report.committed);
        assert!(report.identical);
        assert!(report.consistency.clean);
        assert_eq!(report.node_audit_findings, 0);
        assert_eq!(report.live_content_root, report.source_content_root);
        assert!(report.signed_and_verified);
        assert!(report.udos_object_id.starts_with("sha256:"));
        assert_eq!(report.udos_bundle_fingerprint.len(), 64);
    }

    #[test]
    fn a_fault_rolls_the_chain_back_to_the_base() {
        for point in FaultPoint::ALL {
            let k = keys(2);
            let options = ChainOptions {
                fault: Some(point),
                ..ChainOptions::default()
            };
            let report = run_chain(
                &k,
                &state(2, 1),
                &state(2, 2),
                &NodeId::new("node-b").unwrap(),
                &options,
            )
            .unwrap();
            assert!(!report.committed, "{point:?}");
            assert!(report.rollback_clean, "{point:?}");
            assert_eq!(
                report.live_content_root, report.base_content_root,
                "{point:?}"
            );
            assert!(report.consistency.clean, "{point:?}");
            assert!(report.is_ok(), "{point:?}: {report:?}");
        }
    }

    #[test]
    fn the_chain_is_deterministic() {
        let run = || {
            let k = keys(3);
            run_chain(
                &k,
                &state(3, 1),
                &state(3, 5),
                &NodeId::new("node-b").unwrap(),
                &ChainOptions::default(),
            )
            .unwrap()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn a_chain_between_two_agents_is_refused() {
        let a = keys(4);
        assert_eq!(
            run_chain(
                &a,
                &state(4, 1),
                &state(5, 2),
                &NodeId::new("node-b").unwrap(),
                &ChainOptions::default(),
            ),
            Err(CoreError::InvalidDid)
        );
    }

    #[test]
    fn zero_chunk_size_is_refused() {
        let k = keys(6);
        let options = ChainOptions {
            chunk_ops: 0,
            ..ChainOptions::default()
        };
        assert_eq!(
            run_chain(
                &k,
                &state(6, 1),
                &state(6, 2),
                &NodeId::new("node-b").unwrap(),
                &options,
            ),
            Err(CoreError::Encoding)
        );
    }
}
