//! 治理审计（v1.7.10）。
//!
//! 治理要能被事后检查，两件事缺一不可：
//!
//! 1. **不可悄悄改写的历史**：[`AuditLog`] 把治理事件串成哈希链
//!    （`entry_hash = hash(seq, at, kind, subject, detail, prev_hash)`），
//!    改任何一条、删任何一条、换任何两条的顺序都会断链，[`AuditLog::verify`] 会指出**第一处**
//!    断裂的位置。链根 [`AuditLog::root`] 是整段历史的指纹。
//! 2. **只读导出**：[`AuditExport`] 把提案、否决、紧急指令、执行收据、策略表与不变式
//!    打包成一份可序列化快照，供人类观察层展示——它只读，不提供任何写路径。
//!
//! 审计层不读文件、不开网络、不读墙钟：整条链由逻辑刻度与内容哈希构成，因此可被任何
//! 节点独立复算。

use au4a_core::{canonical_hash, CoreResult, Did, SelfCheck};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::proposal::ProposalState;
use crate::Council;

/// 创世前驱哈希（全零，表示「链从这里开始」）。
pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// 审计链上的一条记录。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntry {
    /// 序号（从 0 开始，必须连续）。
    pub seq: u64,
    /// 逻辑时刻。
    pub at: u64,
    /// 事件类型。
    pub kind: String,
    /// 主题（动议 / 选举 / 紧急指令 id）。
    pub subject: String,
    /// 细节。
    pub detail: String,
    /// 前一条的哈希。
    pub prev_hash: String,
    /// 本条哈希（覆盖除自身以外的全部字段）。
    pub entry_hash: String,
}

impl AuditEntry {
    /// 本条记录的被哈希载荷。
    pub fn payload(&self) -> Value {
        json!({
            "seq": self.seq,
            "at": self.at,
            "kind": self.kind,
            "subject": self.subject,
            "detail": self.detail,
            "prev_hash": self.prev_hash,
        })
    }

    /// 复算本条哈希。
    pub fn recompute_hash(&self) -> CoreResult<String> {
        canonical_hash(&self.payload())
    }
}

/// 验证结论。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditVerdict {
    /// 是否整条链完好。
    pub ok: bool,
    /// 检查过的条数。
    pub checked: usize,
    /// 第一处断裂的位置（`None` 表示没有）。
    pub broken_at: Option<usize>,
    /// 人类可读结论。
    pub reason: String,
}

/// 治理事件哈希链。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditLog {
    entries: Vec<AuditEntry>,
    root: String,
}

impl AuditLog {
    /// 由治理事件重建哈希链（确定性：同一状态给出同一条链）。
    pub fn rebuild(council: &Council) -> CoreResult<Self> {
        let mut entries = Vec::new();
        let mut prev = GENESIS.to_string();
        for (index, event) in council.events().iter().enumerate() {
            let entry = AuditEntry {
                seq: index as u64,
                at: event.at,
                kind: event.kind.clone(),
                subject: event.subject.clone(),
                detail: event.detail.clone(),
                prev_hash: prev.clone(),
                entry_hash: String::new(),
            };
            let hash = entry.recompute_hash()?;
            prev = hash.clone();
            entries.push(AuditEntry {
                entry_hash: hash,
                ..entry
            });
        }
        Ok(Self { entries, root: prev })
    }

    /// 由已有记录重建（用于校验**外部收到**的日志）。
    ///
    /// 注意：这里不信任传入的 `entry_hash`，`verify()` 会重新复算。
    pub fn from_entries(entries: Vec<AuditEntry>) -> Self {
        let root = entries
            .last()
            .map(|e| e.entry_hash.clone())
            .unwrap_or_else(|| GENESIS.to_string());
        Self { entries, root }
    }

    /// 链上记录。
    pub fn entries(&self) -> &[AuditEntry] {
        &self.entries
    }

    /// 链根（整段历史的指纹）。
    pub fn root(&self) -> &str {
        &self.root
    }

    /// 记录条数。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否为空链。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 逐条复算：序号连续、前驱哈希吻合、自身哈希吻合。返回第一处断裂的位置。
    pub fn verify(&self) -> AuditVerdict {
        let mut prev = GENESIS.to_string();
        for (index, entry) in self.entries.iter().enumerate() {
            if entry.seq != index as u64 {
                return AuditVerdict {
                    ok: false,
                    checked: index,
                    broken_at: Some(index),
                    reason: format!("序号不连续：第 {index} 条的 seq={}", entry.seq),
                };
            }
            if entry.prev_hash != prev {
                return AuditVerdict {
                    ok: false,
                    checked: index,
                    broken_at: Some(index),
                    reason: format!("前驱哈希不吻合：第 {index} 条"),
                };
            }
            match entry.recompute_hash() {
                Ok(expected) if expected == entry.entry_hash => {}
                Ok(_) => {
                    return AuditVerdict {
                        ok: false,
                        checked: index,
                        broken_at: Some(index),
                        reason: format!("条目哈希被改写：第 {index} 条"),
                    }
                }
                Err(err) => {
                    return AuditVerdict {
                        ok: false,
                        checked: index,
                        broken_at: Some(index),
                        reason: format!("第 {index} 条无法复算：{err}"),
                    }
                }
            }
            prev = entry.entry_hash.clone();
        }
        AuditVerdict {
            ok: true,
            checked: self.entries.len(),
            broken_at: None,
            reason: format!("{} 条记录全部通过复算，链根 {}", self.entries.len(), self.root),
        }
    }
}

/// 只读导出里的动议快照。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalSnapshot {
    /// 内容地址。
    pub id: String,
    /// 标题。
    pub title: String,
    /// 提案人。
    pub author: Did,
    /// 受理委员会。
    pub committee: String,
    /// 状态。
    pub state: ProposalState,
    /// 动作描述。
    pub action: String,
    /// 表决轮次。
    pub round: u32,
}

/// 只读导出里的否决快照。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VetoSnapshot {
    /// 否决 id。
    pub id: String,
    /// 人类观察者标签。
    pub observer: String,
    /// 被否决动议。
    pub proposal: String,
    /// 公开理由。
    pub reason: String,
    /// 时刻。
    pub at: u64,
}

/// 只读导出里的紧急指令快照。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmergencySnapshot {
    /// 指令 id。
    pub id: String,
    /// 下发者。
    pub issued_by: Did,
    /// 策略键值。
    pub key: String,
    /// 策略值。
    pub value: i64,
    /// 公开理由。
    pub reason: String,
    /// 状态。
    pub status: String,
    /// 确认结果摘要。
    pub confirmation: Option<String>,
}

/// 只读导出里的执行收据快照。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionSnapshot {
    /// 动议 id。
    pub proposal: String,
    /// 执行者。
    pub executor: Did,
    /// 效应摘要。
    pub effects: Vec<String>,
    /// 账本前。
    pub total_before: i64,
    /// 账本后。
    pub total_after: i64,
    /// 守恒是否成立。
    pub conservation_ok: bool,
}

/// 治理状态的**只读**审计导出（人类观察层用；没有任何写路径）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditExport {
    /// 轨道号。
    pub track: String,
    /// 整个治理状态的内容地址。
    pub state_digest: String,
    /// 审计链根。
    pub audit_root: String,
    /// 审计链条数。
    pub audit_entries: usize,
    /// 链是否完好。
    pub chain_ok: bool,
    /// 动议快照。
    pub proposals: Vec<ProposalSnapshot>,
    /// 否决快照。
    pub vetoes: Vec<VetoSnapshot>,
    /// 紧急指令快照。
    pub emergency: Vec<EmergencySnapshot>,
    /// 执行收据快照。
    pub executions: Vec<ExecutionSnapshot>,
    /// 策略表。
    pub policies: Vec<(String, i64)>,
    /// 不变式结论。
    pub invariants: Vec<SelfCheck>,
}

impl AuditExport {
    /// 是否一切自洽（链完好 + 不变式全绿 + 无 vm 占位）。
    pub fn is_healthy(&self) -> bool {
        self.chain_ok && au4a_core::all_passed(&self.invariants)
    }

    /// JSON 投影。
    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| au4a_core::CoreError::Encoding)
    }
}

/// 生成只读审计导出。
pub fn export(council: &Council) -> CoreResult<AuditExport> {
    let log = AuditLog::rebuild(council)?;
    let verdict = log.verify();
    let proposals = council
        .proposals()
        .into_iter()
        .map(|p| ProposalSnapshot {
            id: p.id.clone(),
            title: p.title.clone(),
            author: p.author.clone(),
            committee: p.committee.as_str().to_string(),
            state: p.state,
            action: p.action.describe(),
            round: p.round,
        })
        .collect();
    let vetoes = council
        .vetoes()
        .map(|v| VetoSnapshot {
            id: v.id().to_string(),
            observer: v.observer().to_string(),
            proposal: v.target().to_string(),
            reason: v.reason().to_string(),
            at: v.at(),
        })
        .collect();
    let emergency = council
        .emergency_directives()
        .map(|d| EmergencySnapshot {
            id: d.id.clone(),
            issued_by: d.issued_by.clone(),
            key: d.key.clone(),
            value: d.value,
            reason: d.reason.clone(),
            status: d.status.as_str().to_string(),
            confirmation: d.confirmation.as_ref().map(|c| {
                format!(
                    "{} yes={} no={} quorum={} rolled_back={}",
                    c.status.as_str(),
                    c.approvals,
                    c.rejections,
                    c.quorum,
                    c.rolled_back
                )
            }),
        })
        .collect();
    let executions = council
        .executions()
        .map(|r| ExecutionSnapshot {
            proposal: r.proposal.clone(),
            executor: r.executor.clone(),
            effects: r.effects.iter().map(|e| e.describe()).collect(),
            total_before: r.total_before.get(),
            total_after: r.total_after.get(),
            conservation_ok: r.conservation_ok,
        })
        .collect();
    let policies = council
        .policies()
        .iter()
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    Ok(AuditExport {
        track: crate::TRACK.to_string(),
        state_digest: crate::invariants::state_digest(council)?,
        audit_root: log.root().to_string(),
        audit_entries: log.len(),
        chain_ok: verdict.ok,
        proposals,
        vetoes,
        emergency,
        executions,
        policies,
        invariants: crate::invariants::check_all(council),
    })
}

/// 审计自检项：链完好 + 导出健康。
pub fn audit_checks(council: &Council) -> Vec<SelfCheck> {
    let track = crate::TRACK;
    let mut checks = Vec::new();
    match AuditLog::rebuild(council) {
        Ok(log) => {
            let verdict = log.verify();
            checks.push(if verdict.ok {
                SelfCheck::pass(
                    track,
                    "council.audit.chain",
                    format!("{} 条治理事件串成哈希链并全部通过复算，链根 {}", log.len(), au4a_core::short_id(log.root())),
                )
            } else {
                SelfCheck::fail(track, "council.audit.chain", verdict.reason)
            });
        }
        Err(err) => checks.push(SelfCheck::fail(track, "council.audit.chain", err.to_string())),
    }
    match export(council) {
        Ok(exported) => {
            checks.push(if exported.is_healthy() {
                SelfCheck::pass(
                    track,
                    "council.audit.export",
                    format!(
                        "只读导出健康：{} 条动议 / {} 条否决 / {} 条紧急指令 / {} 张执行收据 / {} 条策略",
                        exported.proposals.len(),
                        exported.vetoes.len(),
                        exported.emergency.len(),
                        exported.executions.len(),
                        exported.policies.len()
                    ),
                )
            } else {
                SelfCheck::fail(track, "council.audit.export", "只读导出不自洽（链断裂或不变式未全绿）")
            });
        }
        Err(err) => checks.push(SelfCheck::fail(track, "council.audit.export", err.to_string())),
    }
    checks
}
