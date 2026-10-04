//! 执行引擎（v1.7.4）。
//!
//! 治理如果只产出「决议」而不产出「状态变更」，那它就只是日志。执行引擎把**已通过**的动议
//! 落成三种真实变更：
//!
//! 1. 策略（`policies`）：治理层的整数参数，例如 `cpu_proto_settle_cap`；
//! 2. 信誉（`reputation`）：委员会掌握的信誉账（不可转让、不可自报）；
//! 3. 账本（`Ledger`）：Agent 之间的转账与锁定质押的罚没——**每条执行都必须保持守恒**。
//!
//! 三条不可协商的规则：
//!
//! * **只能执行 `passed` 的动议**：`open`（没表决）、`rejected`（被否决）、`blocked`（被人类
//!   否决权阻断）都拒绝执行——这是「否决只能阻断」在执行侧的落点。
//! * **执行者必须是在任委员**（`AgentIdentity`）：人类没有这条路径。
//! * **幂等**：同一条动议不可能被执行两次，因此不会重复转账或重复罚没。
//!
//! 每条执行都留下 [`ExecutionReceipt`]，其中记录了账本总量在前后各是多少，
//! 于是「这次执行到底动了什么」可以被复算，而不是靠信任。

use au4a_core::{CoreError, CoreResult, Credits, Did, RefusalCode};
use au4a_kernel::Kernel;
use serde::{Deserialize, Serialize};

use crate::committee::CommitteeKind;
use crate::proposal::{Action, ProposalState};
use crate::{AgentIdentity, Council};

/// 一条执行产生的具体效应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum ExecutionEffect {
    /// 写入治理策略。
    PolicySet {
        /// 策略键。
        key: String,
        /// 策略值。
        value: i64,
    },
    /// 写入信誉（万分比）。
    ReputationSet {
        /// 目标 Agent。
        did: Did,
        /// 新信誉。
        reputation_bp: u32,
    },
    /// 账本转账。
    Transferred {
        /// 付款方。
        from: Did,
        /// 收款方。
        to: Did,
        /// 金额。
        amount: Credits,
    },
    /// 罚没锁定质押。
    Slashed {
        /// 目标 Agent。
        did: Did,
        /// 罚没额。
        amount: Credits,
    },
}

impl ExecutionEffect {
    /// 人类可读描述（观察层与审计共用）。
    pub fn describe(&self) -> String {
        match self {
            ExecutionEffect::PolicySet { key, value } => format!("policy {key}={value}"),
            ExecutionEffect::ReputationSet { did, reputation_bp } => {
                format!(
                    "reputation {}={}bp",
                    au4a_core::short_id(did.as_str()),
                    reputation_bp
                )
            }
            ExecutionEffect::Transferred { from, to, amount } => format!(
                "transfer {}->{} amount={amount}",
                au4a_core::short_id(from.as_str()),
                au4a_core::short_id(to.as_str())
            ),
            ExecutionEffect::Slashed { did, amount } => {
                format!(
                    "slash {} amount={amount}",
                    au4a_core::short_id(did.as_str())
                )
            }
        }
    }
}

/// 一次执行的收据（内容可复算：谁、何时、动了什么、账本前后各是多少）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionReceipt {
    /// 动议 id。
    pub proposal: String,
    /// 受理委员会。
    pub committee: CommitteeKind,
    /// 执行者（必须是在任委员）。
    pub executor: Did,
    /// 执行时刻（逻辑刻度）。
    pub at: u64,
    /// 具体效应。
    pub effects: Vec<ExecutionEffect>,
    /// 执行前账本总量。
    pub total_before: Credits,
    /// 执行后账本总量。
    pub total_after: Credits,
    /// 执行前累计罚没。
    pub slashed_before: Credits,
    /// 执行后累计罚没。
    pub slashed_after: Credits,
    /// 账本守恒断言结果（执行后立刻检查）。
    pub conservation_ok: bool,
}

impl ExecutionReceipt {
    /// 效应摘要（一行）。
    pub fn summary(&self) -> String {
        self.effects
            .iter()
            .map(|e| e.describe())
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// 是否发生了账本变更。
    pub fn touches_ledger(&self) -> bool {
        self.effects.iter().any(|e| {
            matches!(
                e,
                ExecutionEffect::Transferred { .. } | ExecutionEffect::Slashed { .. }
            )
        })
    }

    /// 账本效应自洽：转账不改变总量；罚没使总量恰好减少罚没额、累计罚没恰好增加同样的量。
    pub fn ledger_effect_consistent(&self) -> bool {
        let mut slashed_total: i64 = 0;
        let mut transferred = false;
        for effect in &self.effects {
            match effect {
                ExecutionEffect::Transferred { .. } => transferred = true,
                ExecutionEffect::Slashed { amount, .. } => {
                    slashed_total = match slashed_total.checked_add(amount.get()) {
                        Some(v) => v,
                        None => return false,
                    }
                }
                ExecutionEffect::PolicySet { .. } | ExecutionEffect::ReputationSet { .. } => {}
            }
        }
        if transferred && self.total_before != self.total_after {
            return false;
        }
        let expected_total = match self.total_before.get().checked_sub(slashed_total) {
            Some(v) => v,
            None => return false,
        };
        if self.total_after.get() != expected_total {
            return false;
        }
        let expected_slashed = match self.slashed_before.get().checked_add(slashed_total) {
            Some(v) => v,
            None => return false,
        };
        self.slashed_after.get() == expected_slashed
    }
}

/// 执行一条**已通过**的动议。
///
/// `executor` 必须是受理委员会的在任委员；人类观察者没有这条路径（没有 `AgentIdentity`）。
pub fn execute(
    council: &mut Council,
    kernel: &mut Kernel,
    executor: &AgentIdentity,
    proposal_id: &str,
) -> CoreResult<ExecutionReceipt> {
    let (committee, state, action) = match council.proposal(proposal_id) {
        Some(p) => (p.committee, p.state, p.action.clone()),
        None => return Err(CoreError::UnknownAgent),
    };

    // 1. 只有 passed 的动议可以被执行。blocked（被人类否决权阻断）在这里被挡死。
    if state != ProposalState::Passed {
        kernel.refuse(
            executor.did(),
            RefusalCode::Conflict,
            format!("cannot execute a {} proposal", state.as_str()),
        );
        return Err(CoreError::InvalidKind);
    }
    // 2. 执行者必须是已注册 Agent 且为该委员会在任委员。
    if kernel.card(executor.did()).is_none() {
        kernel.refuse(
            executor.did(),
            RefusalCode::Unauthorized,
            "executor is not a registered agent",
        );
        return Err(CoreError::UnknownAgent);
    }
    let seated = council
        .committee(committee)
        .map(|c| c.has_member(executor.did()))
        .unwrap_or(false);
    if !seated {
        kernel.refuse(
            executor.did(),
            RefusalCode::Unauthorized,
            "executor is not a seated member",
        );
        return Err(CoreError::InvalidSignature);
    }
    // 3. 幂等：已经执行过的动议不会再产生任何账本效应。
    if council.execution(proposal_id).is_some() {
        kernel.refuse(
            executor.did(),
            RefusalCode::Conflict,
            "proposal already executed",
        );
        return Err(CoreError::InvalidKind);
    }

    let total_before = kernel.ledger().total()?;
    let slashed_before = kernel.ledger().slashed();
    let effects = apply(council, kernel, &action)?;
    let total_after = kernel.ledger().total()?;
    let slashed_after = kernel.ledger().slashed();
    let conservation_ok = kernel.ledger().check_conservation().is_ok();

    let at = council.tick();
    let receipt = ExecutionReceipt {
        proposal: proposal_id.to_string(),
        committee,
        executor: executor.did().clone(),
        at,
        effects,
        total_before,
        total_after,
        slashed_before,
        slashed_after,
        conservation_ok,
    };
    council.apply_state(kernel, proposal_id, ProposalState::Executed)?;
    council.store_execution(receipt.clone());
    council.record_event("proposal.executed", proposal_id, receipt.summary());
    kernel.emit(
        "council.executed",
        format!(
            "{} {} total {}→{}",
            au4a_core::short_id(proposal_id),
            receipt.summary(),
            total_before,
            total_after
        ),
    );
    Ok(receipt)
}

/// 把动作落成状态变更。任何一步失败都不留下部分变更。
fn apply(
    council: &mut Council,
    kernel: &mut Kernel,
    action: &Action,
) -> CoreResult<Vec<ExecutionEffect>> {
    match action {
        Action::SetPolicy { key, value } => {
            council.set_policy(key.clone(), *value);
            Ok(vec![ExecutionEffect::PolicySet {
                key: key.clone(),
                value: *value,
            }])
        }
        Action::SetReputation { did, reputation_bp } => {
            council.note_reputation(did, *reputation_bp);
            Ok(vec![ExecutionEffect::ReputationSet {
                did: did.clone(),
                reputation_bp: *reputation_bp,
            }])
        }
        Action::Transfer { from, to, amount } => {
            kernel.ledger_mut().transfer(from, to, *amount)?;
            Ok(vec![ExecutionEffect::Transferred {
                from: from.clone(),
                to: to.clone(),
                amount: *amount,
            }])
        }
        Action::Slash { did, amount } => {
            // 治理罚没不做静默截断：锁定余额不足就是拒绝。
            let locked = kernel.ledger().balance(did).locked;
            if locked < *amount {
                kernel.refuse(
                    did,
                    RefusalCode::PolicyDenied,
                    format!("slash {amount} exceeds locked stake {locked}"),
                );
                return Err(CoreError::InsufficientFunds);
            }
            kernel.ledger_mut().slash(did, *amount)?;
            Ok(vec![ExecutionEffect::Slashed {
                did: did.clone(),
                amount: *amount,
            }])
        }
    }
}
