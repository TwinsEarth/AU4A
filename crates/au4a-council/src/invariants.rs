//! 治理不变式与确定性重放（v1.7.7）。
//!
//! 前六版把功能做出来了，这一版回答一个更难的问题：**这些功能是否始终自洽**。
//! 两个工具：
//!
//! * [`state_digest`]：整个治理状态的内容地址。同一状态 → 同一摘要；任何一处变更 → 摘要改变。
//!   这是「重放同一段治理历史必须得到同一个状态」的可检验版本。
//! * [`replay`]：用确定性 LCG 驱动一串真实治理操作（提案 / 开轮 / 重复投票 / 双签 / 弃权 /
//!   越权投票 / 执行 / 否决），**每一步之后**跑一遍全部不变式（[`check_all`]）。
//!   同一种子必然给出同一份报告——失败可复现，不是「偶发」。
//!
//! 重放里出现的拒绝都是**预期路径**（它们正是被测试的失败模式）；只有不变式违例
//! （`violations` 非空）才算失败。

use std::collections::BTreeSet;

use au4a_core::{canonical_hash, AgentKeys, CoreError, CoreResult, Credits, SelfCheck};
use au4a_kernel::{Kernel, KernelConfig};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::proposal::{Action, ProposalState};
use crate::voting::{Choice, Vote};
use crate::{
    enroll, keys_for, seat_all_committees, AgentIdentity, CommitteeKind, Council, CouncilConfig,
    HumanObserver, ProposalDraft,
};

/// crate 承诺的全部不变式名字。测试会断言「清单与实现严格相等」——
/// 删掉任何一条不变式都会让测试变红，避免静默减少保证。
pub const INVARIANT_NAMES: [&str; 17] = [
    "council.committees.installed",
    "council.quorum.bft",
    "council.committees.nonempty",
    "council.proposals.content_addressed",
    "council.events.monotonic",
    "council.votes.tally_consistent",
    "council.votes.members_only",
    "council.rounds.decided_matches_proposal",
    "council.executions.ledger_effects",
    "council.executions.state_matches",
    "council.vetoes.blocks_only",
    "council.vetoes.reasons_public",
    "council.ongov.mapping",
    "council.ongov.no_false_chain",
    "council.events.subjects_resolve",
    "council.rounds.committee_seated",
    "council.policies.well_formed",
];

/// 整个治理状态的内容地址（可复算、可比较）。
pub fn state_digest(council: &Council) -> CoreResult<String> {
    let committees: Vec<Value> = council
        .committees()
        .map(|c| {
            json!({
                "kind": c.kind,
                "epoch": c.epoch,
                "election_id": c.election_id,
                "members": c.member_dids().iter().map(|d| d.as_str().to_string()).collect::<Vec<_>>(),
            })
        })
        .collect();
    let proposals: Vec<Value> = council
        .proposals()
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "author": p.author,
                "committee": p.committee,
                "title": p.title,
                "action": p.action,
                "state": p.state,
                "round": p.round,
            })
        })
        .collect();
    let rounds: Vec<Value> = council
        .rounds()
        .map(|r| {
            json!({
                "proposal": r.proposal,
                "round": r.round,
                "n": r.n,
                "f": r.f,
                "quorum": r.quorum,
                "outcome": r.outcome,
                "votes": r.votes.iter().map(|(did, v)| json!({"did": did, "choice": v.choice})).collect::<Vec<_>>(),
            })
        })
        .collect();
    let vetoes: Vec<Value> = council.vetoes().map(|v| v.payload()).collect();
    let executions: Vec<Value> = council
        .executions()
        .map(|e| {
            json!({
                "proposal": e.proposal,
                "effects": e.effects,
                "total_before": e.total_before,
                "total_after": e.total_after,
                "conservation_ok": e.conservation_ok,
            })
        })
        .collect();
    let policies: Vec<Value> = council
        .policies()
        .iter()
        .map(|(k, v)| json!({"key": k, "value": v}))
        .collect();
    canonical_hash(&json!({
        "committees": committees,
        "proposals": proposals,
        "rounds": rounds,
        "vetoes": vetoes,
        "executions": executions,
        "policies": policies,
        "events": council.events().len(),
        "now": council.now(),
    }))
}

/// 在给定治理层上跑**全部**不变式：`Council::checks()` + 三条跨对象一致性检查。
pub fn check_all(council: &Council) -> Vec<SelfCheck> {
    let track = crate::TRACK;
    let mut checks = council.checks();

    // 15. 事件主题必须指向真实存在的对象（防止日志指向幽灵动议/选举）。
    let proposal_ids: BTreeSet<&str> = council.proposals().iter().map(|p| p.id.as_str()).collect();
    let election_ids: BTreeSet<&str> = council.committees().map(|c| c.election_id.as_str()).collect();
    let subjects_ok = council.events().iter().all(|e| {
        if e.kind == "election.seated" {
            election_ids.contains(e.subject.as_str())
        } else {
            proposal_ids.contains(e.subject.as_str())
        }
    });
    checks.push(if subjects_ok {
        SelfCheck::pass(
            track,
            "council.events.subjects_resolve",
            format!("{} 条治理事件的主题都能解析到真实对象", council.events().len()),
        )
    } else {
        SelfCheck::fail(track, "council.events.subjects_resolve", "存在指向不存在对象的治理事件")
    });

    // 16. 表决轮所属委员会必须仍然在任，且委员会人数与开轮时记录的一致。
    let rounds_ok = council.rounds().all(|r| {
        council
            .committee(r.committee)
            .map(|c| c.size() == r.n && c.quorum() == r.quorum)
            .unwrap_or(false)
    });
    checks.push(if rounds_ok {
        SelfCheck::pass(
            track,
            "council.rounds.committee_seated",
            format!("{} 轮表决的受理委员会均在任，且 n/quorum 与开轮时一致", council.rounds().count()),
        )
    } else {
        SelfCheck::fail(track, "council.rounds.committee_seated", "存在受理委员会已变更的表决轮")
    });

    // 17. 策略表只允许由执行引擎写入的合法键（非空、小写 ASCII 标识符）。
    let policies_ok = council.policies().keys().all(|k| {
        !k.is_empty()
            && k.len() <= 64
            && k.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_')
    });
    checks.push(if policies_ok {
        SelfCheck::pass(
            track,
            "council.policies.well_formed",
            format!("{} 条治理策略的键名合法（执行引擎写入路径已校验动作）", council.policies().len()),
        )
    } else {
        SelfCheck::fail(track, "council.policies.well_formed", "存在非法策略键")
    });

    checks
}

/// 确定性线性同余发生器：重放必须可复现，所以不能用系统随机源。
#[derive(Clone, Debug)]
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(2_862_933_555_777_941_757).wrapping_add(3_037_000_493))
    }

    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 17
    }

    fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            0
        } else {
            self.next() % bound
        }
    }
}

/// 确定性重放报告。同一 `seed` + `steps` 必须给出逐字节相同的报告。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayReport {
    /// 种子。
    pub seed: u64,
    /// 请求的步数。
    pub steps: u32,
    /// 实际执行的步数（非法操作不计步）。
    pub applied: u32,
    /// 产生的动议数。
    pub proposals: usize,
    /// 表决轮数。
    pub rounds: usize,
    /// 被作废（双签）的轮数。
    pub voided_rounds: usize,
    /// 已执行的动议数。
    pub executed: usize,
    /// 被人类否决阻断的动议数。
    pub blocked: usize,
    /// 被表决否决的动议数。
    pub rejected: usize,
    /// 内核记录的拒绝条数（越权/重复/过期等预期失败路径）。
    pub refusals: usize,
    /// 每步之后执行的不变式条数。
    pub invariants_per_step: usize,
    /// 违例（空表示全程自洽）。
    pub violations: Vec<String>,
    /// 最终状态摘要。
    pub digest: String,
}

impl ReplayReport {
    /// 全程无违例。
    pub fn ok(&self) -> bool {
        self.violations.is_empty()
    }
}

/// 用确定性伪随机序列驱动一段治理历史，每步之后检查全部不变式。
///
/// 覆盖的操作：由委员提案、开轮、投赞成/反对/弃权、同轮重复投票（同选择）、
/// 同轮改投（双签作废）、非委员投票、执行、人类否决、对终态动议的非法操作。
pub fn replay(seed: u64, steps: u32) -> CoreResult<ReplayReport> {
    let mut kernel = Kernel::new(KernelConfig::default());
    let mut council = Council::new(CouncilConfig::default());
    let (agents, socks) = enroll(&mut kernel, &mut council)?;
    seat_all_committees(&mut kernel, &mut council, &agents, &socks)?;
    let outsider = AgentKeys::from_seed(&[0x99u8; 32]);
    match kernel.register(&outsider, "outsider", &["governance.vote"], Credits(20)) {
        Ok(_) | Err(CoreError::DuplicateAgent) => {}
        Err(err) => return Err(err),
    }

    let mut rng = Lcg::new(seed);
    let mut report = ReplayReport {
        seed,
        steps,
        applied: 0,
        proposals: 0,
        rounds: 0,
        voided_rounds: 0,
        executed: 0,
        blocked: 0,
        rejected: 0,
        refusals: 0,
        invariants_per_step: INVARIANT_NAMES.len(),
        violations: Vec::new(),
        digest: String::new(),
    };

    for _ in 0..steps {
        let before = record(&council);
        match rng.below(12) {
            // 0..4：提案（委员提出，内容含步号，因此不会撞内容地址）
            0..=3 => {
                let kind = CommitteeKind::ALL[rng.below(CommitteeKind::ALL.len() as u64) as usize];
                let members = council.committee(kind).map(|c| c.member_dids()).unwrap_or_default();
                if members.is_empty() {
                    continue;
                }
                let did = members[rng.below(members.len() as u64) as usize].clone();
                if let Some(keys) = keys_for(&agents, &did) {
                    let case = rng.below(5);
                    let action = match case {
                        0 => Action::SetPolicy {
                            key: format!("policy_{}", report.applied),
                            value: report.applied as i64,
                        },
                        1 => Action::SetReputation { did: did.clone(), reputation_bp: 4_000 },
                        2 => Action::Transfer {
                            from: did.clone(),
                            to: agents[0].did(),
                            amount: Credits(5),
                        },
                        3 => Action::Slash { did: did.clone(), amount: Credits(5) },
                        _ => Action::SetPolicy { key: format!("k{}", report.applied), value: 1 },
                    };
                    let draft = ProposalDraft::by(
                        keys,
                        kind,
                        format!("replay-{}-{}", seed, report.applied),
                        action,
                    )?;
                    let identity = AgentIdentity::from_keys(keys);
                    if council.propose(&mut kernel, &identity, draft).is_ok() {
                        report.proposals += 1;
                        report.applied += 1;
                    }
                }
            }
            // 4..6：开轮
            4..=5 => {
                let candidates: Vec<String> = council
                    .proposals()
                    .into_iter()
                    .filter(|p| p.state == ProposalState::Open && p.round < 3)
                    .map(|p| p.id.clone())
                    .collect();
                if candidates.is_empty() {
                    continue;
                }
                let id = candidates[rng.below(candidates.len() as u64) as usize].clone();
                // 已开过的轮次不再重开：仅在当前轮已关闭时开新轮。
                let closed = council
                    .current_round(&id)
                    .map(|r| r.outcome.is_closed())
                    .unwrap_or(true);
                if closed && council.open_round(&mut kernel, &id).is_ok() {
                    report.rounds += 1;
                    report.applied += 1;
                }
            }
            // 6..8：投票（含双签与越权）
            6..=7 => {
                let candidates: Vec<(String, u32)> = council
                    .proposals()
                    .into_iter()
                    .filter(|p| p.state == ProposalState::Open)
                    .filter_map(|p| council.current_round(&p.id).map(|r| (p.id.clone(), r.round)))
                    .filter(|(id, round)| {
                        council
                            .round(id, *round)
                            .map(|r| !r.outcome.is_closed())
                            .unwrap_or(false)
                    })
                    .collect();
                if candidates.is_empty() {
                    continue;
                }
                let (id, round) = candidates[rng.below(candidates.len() as u64) as usize].clone();
                let kind = council.proposal(&id).map(|p| p.committee);
                let members = kind
                    .and_then(|k| council.committee(k))
                    .map(|c| c.member_dids())
                    .unwrap_or_default();
                if members.is_empty() {
                    continue;
                }
                let choice = match rng.below(3) {
                    0 => Choice::Yes,
                    1 => Choice::No,
                    _ => Choice::Abstain,
                };
                // 每 7 步左右制造一次双签（同轮改投），每 11 步左右制造一次越权投票。
                let roll = rng.below(11);
                let voter_did = if roll == 0 {
                    outsider.did()
                } else {
                    members[rng.below(members.len() as u64) as usize].clone()
                };
                let choice = if roll == 3 {
                    // 与可能已投的选择相反，制造模棱两可。
                    match choice {
                        Choice::Yes => Choice::No,
                        _ => Choice::Yes,
                    }
                } else {
                    choice
                };
                let voter_keys: Option<&AgentKeys> = if roll == 0 {
                    Some(&outsider)
                } else {
                    keys_for(&agents, &voter_did)
                };
                let keys = match voter_keys {
                    Some(k) => k,
                    None => &outsider,
                };
                let vote = Vote::cast(keys, &id, round, choice)?;
                let outcome_before = council.round(&id, round).map(|r| r.outcome);
                match council.cast_vote(&mut kernel, vote) {
                    Ok(_state) => {
                        report.applied += 1;
                    }
                    Err(_) => {
                        if let Some(after) = council.round(&id, round) {
                            if after.outcome == crate::RoundOutcome::VoidAmbiguous
                                && outcome_before != Some(crate::RoundOutcome::VoidAmbiguous)
                            {
                                report.voided_rounds += 1;
                            }
                        }
                    }
                }
            }
            // 8：协同推进——全体在任委员投赞成票直到出结论（Agent 会协调，这是常态）。
            8 => {
                let candidates: Vec<(String, u32)> = council
                    .proposals()
                    .into_iter()
                    .filter(|p| p.state == ProposalState::Open)
                    .filter_map(|p| council.current_round(&p.id).map(|r| (p.id.clone(), r.round)))
                    .filter(|(id, round)| {
                        council
                            .round(id, *round)
                            .map(|r| !r.outcome.is_closed())
                            .unwrap_or(false)
                    })
                    .collect();
                if candidates.is_empty() {
                    continue;
                }
                let (id, round) = candidates[rng.below(candidates.len() as u64) as usize].clone();
                let members = council
                    .proposal(&id)
                    .and_then(|p| council.committee(p.committee))
                    .map(|c| c.member_dids())
                    .unwrap_or_default();
                for did in &members {
                    let closed = council
                        .round(&id, round)
                        .map(|r| r.outcome.is_closed())
                        .unwrap_or(true);
                    if closed {
                        break;
                    }
                    if let Some(keys) = keys_for(&agents, did) {
                        let vote = Vote::cast(keys, &id, round, Choice::Yes)?;
                        if council.cast_vote(&mut kernel, vote).is_ok() {
                            report.applied += 1;
                        }
                    }
                }
            }
            // 9：执行
            9 => {
                let candidates: Vec<String> = council
                    .proposals()
                    .into_iter()
                    .filter(|p| p.state == ProposalState::Passed)
                    .map(|p| p.id.clone())
                    .collect();
                if candidates.is_empty() {
                    continue;
                }
                let id = candidates[rng.below(candidates.len() as u64) as usize].clone();
                let members = council
                    .proposal(&id)
                    .and_then(|p| council.committee(p.committee))
                    .map(|c| c.member_dids())
                    .unwrap_or_default();
                if members.is_empty() {
                    continue;
                }
                let did = members[rng.below(members.len() as u64) as usize].clone();
                if let Some(keys) = keys_for(&agents, &did) {
                    let identity = AgentIdentity::from_keys(keys);
                    if council.execute(&mut kernel, &identity, &id).is_ok() {
                        report.executed += 1;
                        report.applied += 1;
                    }
                }
            }
            // 10..11：人类否决（每 3 次里 1 次故意用空理由，验证拒绝路径）
            _ => {
                let candidates: Vec<String> = council
                    .proposals()
                    .into_iter()
                    .filter(|p| matches!(p.state, ProposalState::Open | ProposalState::Passed))
                    .map(|p| p.id.clone())
                    .collect();
                if candidates.is_empty() {
                    continue;
                }
                let id = candidates[rng.below(candidates.len() as u64) as usize].clone();
                let human = HumanObserver::new("replay-observer");
                let reason = if rng.below(3) == 0 {
                    String::from("   ")
                } else {
                    format!("replay veto at step {}", report.applied)
                };
                if let Ok(veto) = human.veto(&council, &id, reason) {
                    if council.apply_veto(&mut kernel, &veto).is_ok() {
                        report.blocked += 1;
                        report.applied += 1;
                    }
                }
            }
        }

        // 每一步之后：全部不变式 + 状态推进检查。
        for check in check_all(&council) {
            if !check.passed {
                report
                    .violations
                    .push(format!("step {}: {} — {}", report.applied, check.name, check.detail));
            }
        }
        let after = record(&council);
        if after.0 + after.1 + after.2 + after.3 < before.0 + before.1 + before.2 + before.3 {
            report.violations.push(format!("step {}: 治理记录数量倒退", report.applied));
        }
    }

    report.rejected = council
        .proposals()
        .iter()
        .filter(|p| p.state == ProposalState::Rejected)
        .count();
    report.blocked = council
        .proposals()
        .iter()
        .filter(|p| p.state == ProposalState::Blocked)
        .count();
    report.executed = council
        .proposals()
        .iter()
        .filter(|p| p.state == ProposalState::Executed)
        .count();
    report.proposals = council.proposals().len();
    report.rounds = council.rounds().count();
    report.voided_rounds = council
        .rounds()
        .filter(|r| r.outcome == crate::RoundOutcome::VoidAmbiguous)
        .count();
    report.refusals = kernel.refusals().len();
    report.digest = state_digest(&council)?;
    Ok(report)
}

/// 记录单调计数：(动议, 轮次, 否决, 执行)。
fn record(council: &Council) -> (usize, usize, usize, usize) {
    (
        council.proposals().len(),
        council.rounds().count(),
        council.vetoes().count(),
        council.executions().count(),
    )
}

/// 一组种子的重放摘要（自检与观察层用）。
pub fn replay_report(seeds: &[u64], steps: u32) -> CoreResult<Value> {
    let mut reports = Vec::new();
    for seed in seeds {
        let report = replay(*seed, steps)?;
        reports.push(json!({
            "seed": report.seed,
            "steps": report.steps,
            "applied": report.applied,
            "proposals": report.proposals,
            "rounds": report.rounds,
            "voided_rounds": report.voided_rounds,
            "executed": report.executed,
            "blocked": report.blocked,
            "rejected": report.rejected,
            "refusals": report.refusals,
            "violations": report.violations.len(),
            "digest": report.digest,
        }));
    }
    let total_violations: usize = reports
        .iter()
        .map(|r| r["violations"].as_u64().unwrap_or(0) as usize)
        .sum();
    Ok(json!({
        "seeds": seeds,
        "steps": steps,
        "reports": reports,
        "total_violations": total_violations,
        "invariants_per_step": INVARIANT_NAMES.len(),
    }))
}
