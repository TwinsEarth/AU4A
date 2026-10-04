//! Agent 委员会（v1.0.4）：自治网络里唯一有权作出「限制某个 Agent」决定的主体。
//!
//! 三条不可协商的设计约束：
//!
//! 1. **委员只能是 Agent**：委员由 [`Council::sortition`] 从**在册 Agent** 里按内容哈希确定性抽签产生。
//!    类型里没有「人类委员」这种变体；人类要否决只能走 v1.7 的 `au4a-council`，且只能阻断、不能提案。
//! 2. **竞争不隔离**：委员会能审理的最重处置（`Quarantine`）**只对恶意 2 码开放**。
//!    `RefusalCode::ALL` 里其余 8 个竞争码，单次连动议都提不出来；重复超阈值也只到 `Warn`。
//!    这是「因为竞争而隔离，等于因为竞争而误伤」在代码里的落地（有穷举测试）。
//! 3. **失败模式具名**：`CouncilFailure` 把「不足法定人数 / 重复投票 / 非委员投票 / 平票 / 未知动议 /
//!    重复动议 / 空委员会」全部显式化，并映射回类型化拒绝码，保证「恶意 2 码 vs 竞争 8 码」的分类不被稀释。
//!
//! 本模块只做「抽签 → 动议 → 投票 → 计票 → 决定」；决定如何作用到 Agent 生命周期由 v1.0.7 的状态机执行。

use std::collections::BTreeMap;

use au4a_core::{canonical_hash, CoreResult, Did, Escalation, RefusalCode};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::Kernel;

/// 抽签盐值：写进哈希，保证「同一 epoch + 同一 DID 集合 → 同一委员会」。
pub const SORTITION_SALT: &str = "au4a-council-sortition-v1";

/// 委员会配置。人类可以设定门槛，但不能指定委员。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CouncilConfig {
    /// 委员会规模（在册 Agent 不足时取全部）。
    pub size: usize,
    /// 法定人数比例（基点，1 bp = 0.01%）。
    pub quorum_bps: i64,
    /// 通过比例（基点，按赞成/反对计）。
    pub pass_bps: i64,
}

impl Default for CouncilConfig {
    fn default() -> Self {
        Self {
            size: 5,
            quorum_bps: 6_000, // 60% 出席
            pass_bps: 5_100,   // 简单多数
        }
    }
}

/// 动议种类。`Quarantine` 是最重处置，只对恶意码开放。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionKind {
    /// 记警告（重复竞争行为）。
    Warn,
    /// 隔离（仅恶意：malformed / unauthorized）。
    Quarantine,
    /// 罚没质押。
    Slash,
    /// 解除隔离/恢复。
    Reprieve,
}

impl MotionKind {
    pub const ALL: [MotionKind; 4] = [
        MotionKind::Warn,
        MotionKind::Quarantine,
        MotionKind::Slash,
        MotionKind::Reprieve,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            MotionKind::Warn => "warn",
            MotionKind::Quarantine => "quarantine",
            MotionKind::Slash => "slash",
            MotionKind::Reprieve => "reprieve",
        }
    }
}

/// 一份动议。`id` 是内容寻址：动议内容不可被事后篡改。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Motion {
    pub id: String,
    pub kind: MotionKind,
    pub subject: Did,
    pub cause: RefusalCode,
    /// 由哪个升级判定触发（`None` 表示委员会自主提案）。
    pub escalation: Option<Escalation>,
    pub at: u64,
}

impl Motion {
    pub fn new(
        kind: MotionKind,
        subject: Did,
        cause: RefusalCode,
        escalation: Option<Escalation>,
        at: u64,
    ) -> CoreResult<Self> {
        let mut motion = Self {
            id: String::new(),
            kind,
            subject,
            cause,
            escalation,
            at,
        };
        motion.id = motion.compute_id()?;
        Ok(motion)
    }

    fn payload(&self) -> serde_json::Value {
        json!({
            "kind": self.kind.as_str(),
            "subject": self.subject,
            "cause": self.cause.as_str(),
            "escalation": self.escalation,
            "at": self.at,
        })
    }

    pub fn compute_id(&self) -> CoreResult<String> {
        canonical_hash(&self.payload())
    }

    /// 内容自洽：id 与内容一致。
    pub fn is_intact(&self) -> bool {
        match self.compute_id() {
            Ok(id) => id == self.id,
            Err(_) => false,
        }
    }

    /// 把内核的升级判定翻译成动议。
    ///
    /// **唯一的隔离入口**：`Quarantine` 只在 `RefusalCode::is_misconduct()` 为真时产生。
    pub fn from_escalation(
        who: &Did,
        code: RefusalCode,
        escalation: Escalation,
        at: u64,
    ) -> CoreResult<Option<Self>> {
        let kind = match (code.is_misconduct(), escalation) {
            (true, _) => MotionKind::Quarantine,
            (false, Escalation::Quarantine) => {
                // 竞争码永远不能升级为隔离——即使上游误传了 Quarantine，这里也不生成动议。
                return Ok(None);
            }
            (false, Escalation::Warn) => MotionKind::Warn,
            (false, Escalation::None) => return Ok(None),
        };
        Ok(Some(Motion::new(
            kind,
            who.clone(),
            code,
            Some(escalation),
            at,
        )?))
    }
}

/// 选票。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ballot {
    Uphold,
    Reject,
    Abstain,
}

impl Ballot {
    pub const ALL: [Ballot; 3] = [Ballot::Uphold, Ballot::Reject, Ballot::Abstain];

    pub fn as_str(self) -> &'static str {
        match self {
            Ballot::Uphold => "uphold",
            Ballot::Reject => "reject",
            Ballot::Abstain => "abstain",
        }
    }
}

/// 一张选票。投票人必须是委员（由 [`Council::vote`] 强制）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vote {
    pub voter: Did,
    pub motion: String,
    pub ballot: Ballot,
    pub at: u64,
}

/// 委员会的具名失败模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CouncilFailure {
    /// 非委员投票：身份不成立（恶意）。
    NonMemberVote,
    /// 重复投票：一人多票即欺诈（恶意）。
    DuplicateVote,
    /// 动议内容与其内容寻址 id 不符（被篡改，恶意）。
    MalformedMotion,
    /// 动议不存在。
    UnknownMotion,
    /// 动议重复提交。
    DuplicateMotion,
    /// 未达法定人数。
    NoQuorum,
    /// 有表决但无有效多数。
    TieInsufficient,
    /// 空委员会（无在册 Agent 可抽签）。
    EmptyCouncil,
}

impl CouncilFailure {
    pub const ALL: [CouncilFailure; 8] = [
        CouncilFailure::NonMemberVote,
        CouncilFailure::DuplicateVote,
        CouncilFailure::MalformedMotion,
        CouncilFailure::UnknownMotion,
        CouncilFailure::DuplicateMotion,
        CouncilFailure::NoQuorum,
        CouncilFailure::TieInsufficient,
        CouncilFailure::EmptyCouncil,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            CouncilFailure::NonMemberVote => "non_member_vote",
            CouncilFailure::DuplicateVote => "duplicate_vote",
            CouncilFailure::MalformedMotion => "malformed_motion",
            CouncilFailure::UnknownMotion => "unknown_motion",
            CouncilFailure::DuplicateMotion => "duplicate_motion",
            CouncilFailure::NoQuorum => "no_quorum",
            CouncilFailure::TieInsufficient => "tie_insufficient",
            CouncilFailure::EmptyCouncil => "empty_council",
        }
    }

    /// 映射回类型化拒绝码，保持「恶意 2 码 vs 竞争 8 码」不变。
    pub fn to_refusal_code(self) -> RefusalCode {
        match self {
            // 冒充委员 / 一人多票 / 篡改动议：身份与内容不成立，单次即恶意。
            CouncilFailure::NonMemberVote
            | CouncilFailure::DuplicateVote
            | CouncilFailure::MalformedMotion => RefusalCode::Unauthorized,
            CouncilFailure::UnknownMotion => RefusalCode::Unsupported,
            // 重复提交可能是重放竞争，也可能只是网络抖动：按竞争处理。
            CouncilFailure::DuplicateMotion => RefusalCode::Conflict,
            CouncilFailure::NoQuorum
            | CouncilFailure::TieInsufficient
            | CouncilFailure::EmptyCouncil => RefusalCode::Degraded,
        }
    }
}

/// 计票结论。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Upheld,
    Rejected,
    Inconclusive,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Upheld => "upheld",
            Verdict::Rejected => "rejected",
            Verdict::Inconclusive => "inconclusive",
        }
    }
}

/// 一次计票的完整结果（可序列化，供观察层展示）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tally {
    pub motion: String,
    pub uphold: u32,
    pub reject: u32,
    pub abstain: u32,
    pub participants: u32,
    pub members: u32,
    pub quorum_required: u32,
    pub quorum_met: bool,
    pub verdict: Verdict,
    pub failure: Option<CouncilFailure>,
}

/// 委员会作出的决定（决定日志；执行在 v1.0.7 状态机）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub motion: String,
    pub kind: MotionKind,
    pub subject: Did,
    pub verdict: Verdict,
    pub at: u64,
}

/// 一个任期的委员会。
#[derive(Clone, Debug)]
pub struct Council {
    config: CouncilConfig,
    epoch: u64,
    members: Vec<Did>,
    motions: BTreeMap<String, Motion>,
    votes: BTreeMap<String, BTreeMap<Did, Vote>>,
    failures: Vec<CouncilFailure>,
    decisions: Vec<Decision>,
}

impl Council {
    /// 确定性抽签：按 `hash(epoch, salt, did)` 升序取前 `size` 名在册 Agent。
    ///
    /// 没有任何人类指令参与：委员名单完全由「在册集合 + epoch + 盐」决定，任何节点都能复算。
    pub fn sortition(kernel: &Kernel, epoch: u64, config: CouncilConfig) -> CoreResult<Self> {
        let mut scored: Vec<(String, Did)> = Vec::new();
        for card in kernel.agents() {
            let score = canonical_hash(&json!({
                "epoch": epoch,
                "salt": SORTITION_SALT,
                "did": card.did.as_str(),
            }))?;
            scored.push((score, card.did.clone()));
        }
        // 以 (分数, DID) 排序：分数本身已经均匀，DID 只是确定性兜底。
        scored.sort();
        let members: Vec<Did> = scored
            .into_iter()
            .take(config.size)
            .map(|(_, did)| did)
            .collect();
        let council = Self {
            config,
            epoch,
            members,
            motions: BTreeMap::new(),
            votes: BTreeMap::new(),
            failures: Vec::new(),
            decisions: Vec::new(),
        };
        Ok(council)
    }

    pub fn config(&self) -> CouncilConfig {
        self.config
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn members(&self) -> &[Did] {
        &self.members
    }

    pub fn is_member(&self, did: &Did) -> bool {
        self.members.iter().any(|m| m == did)
    }

    pub fn failures(&self) -> &[CouncilFailure] {
        &self.failures
    }

    pub fn decisions(&self) -> &[Decision] {
        &self.decisions
    }

    pub fn motion(&self, id: &str) -> Option<&Motion> {
        self.motions.get(id)
    }

    pub fn motion_count(&self) -> usize {
        self.motions.len()
    }

    /// 达成法定人数所需的最少出席人数（向上取整，整数运算）。
    pub fn quorum_required(&self) -> u32 {
        let members = self.members.len() as i64;
        let needed = (members * self.config.quorum_bps + 9_999) / 10_000;
        needed.max(0) as u32
    }

    /// 提交动议。
    pub fn submit(&mut self, motion: Motion) -> Result<(), CouncilFailure> {
        if !motion.is_intact() {
            self.failures.push(CouncilFailure::MalformedMotion);
            return Err(CouncilFailure::MalformedMotion);
        }
        if self.motions.contains_key(&motion.id) {
            self.failures.push(CouncilFailure::DuplicateMotion);
            return Err(CouncilFailure::DuplicateMotion);
        }
        self.motions.insert(motion.id.clone(), motion);
        Ok(())
    }

    /// 投票：非委员与重复投票都被具名拒绝。
    pub fn vote(&mut self, vote: Vote) -> Result<(), CouncilFailure> {
        if !self.is_member(&vote.voter) {
            self.failures.push(CouncilFailure::NonMemberVote);
            return Err(CouncilFailure::NonMemberVote);
        }
        if !self.motions.contains_key(&vote.motion) {
            self.failures.push(CouncilFailure::UnknownMotion);
            return Err(CouncilFailure::UnknownMotion);
        }
        let slot = self.votes.entry(vote.motion.clone()).or_default();
        if slot.contains_key(&vote.voter) {
            self.failures.push(CouncilFailure::DuplicateVote);
            return Err(CouncilFailure::DuplicateVote);
        }
        slot.insert(vote.voter.clone(), vote);
        Ok(())
    }

    /// 计票（不记录决定）。
    pub fn tally(&self, motion_id: &str) -> Tally {
        let members = self.members.len() as u32;
        let mut tally = Tally {
            motion: motion_id.to_string(),
            uphold: 0,
            reject: 0,
            abstain: 0,
            participants: 0,
            members,
            quorum_required: self.quorum_required(),
            quorum_met: false,
            verdict: Verdict::Inconclusive,
            failure: None,
        };
        if members == 0 {
            tally.failure = Some(CouncilFailure::EmptyCouncil);
            return tally;
        }
        if !self.motions.contains_key(motion_id) {
            tally.failure = Some(CouncilFailure::UnknownMotion);
            return tally;
        }
        if let Some(votes) = self.votes.get(motion_id) {
            for vote in votes.values() {
                match vote.ballot {
                    Ballot::Uphold => tally.uphold += 1,
                    Ballot::Reject => tally.reject += 1,
                    Ballot::Abstain => tally.abstain += 1,
                }
            }
        }
        tally.participants = tally.uphold + tally.reject + tally.abstain;
        tally.quorum_met = tally.participants >= tally.quorum_required;
        if !tally.quorum_met {
            tally.failure = Some(CouncilFailure::NoQuorum);
            return tally;
        }
        let decisive = tally.uphold + tally.reject;
        if decisive == 0 {
            tally.failure = Some(CouncilFailure::TieInsufficient);
            return tally;
        }
        // 整数基点判定：uphold * 10000 >= decisive * pass_bps
        let scaled = (tally.uphold as i64) * 10_000;
        let needed = (decisive as i64) * self.config.pass_bps;
        if scaled >= needed {
            tally.verdict = Verdict::Upheld;
        } else {
            tally.verdict = Verdict::Rejected;
        }
        tally
    }

    /// 计票并记录决定。只有 `Upheld` 才产生可执行的决定。
    pub fn decide(&mut self, motion_id: &str, at: u64) -> Tally {
        let tally = self.tally(motion_id);
        match (tally.verdict, tally.failure) {
            (Verdict::Upheld, _) => {
                if let Some(motion) = self.motions.get(motion_id) {
                    self.decisions.push(Decision {
                        motion: motion.id.clone(),
                        kind: motion.kind,
                        subject: motion.subject.clone(),
                        verdict: Verdict::Upheld,
                        at,
                    });
                }
            }
            (_, Some(failure)) => self.failures.push(failure),
            _ => {}
        }
        tally
    }

    /// 决定日志的确定性摘要（审计与观察层用）。
    pub fn decisions_json(&self) -> CoreResult<serde_json::Value> {
        Ok(json!({
            "epoch": self.epoch,
            "members": self.members.len(),
            "motions": self.motions.len(),
            "decisions": self.decisions,
            "failures": self.failures.iter().map(|f| f.as_str()).collect::<Vec<&str>>(),
            "fingerprint": canonical_hash(&json!(self.decisions))?,
        }))
    }
}

/// 由内核的升级判定生成动议（竞争码永远拿不到隔离动议）。
pub fn motions_for_kernel(kernel: &Kernel, at: u64) -> CoreResult<Vec<Motion>> {
    let mut out = Vec::new();
    for (who, refusal) in kernel.refusals() {
        let escalation = kernel.escalation_for(who, refusal.code);
        if let Some(motion) = Motion::from_escalation(who, refusal.code, escalation, at)? {
            if !out.iter().any(|m: &Motion| m.id == motion.id) {
                out.push(motion);
            }
        }
    }
    Ok(out)
}

/// 编译期证据：动议构造不接受任何「人类主体」类型；投票人只能是 DID。
const _: fn(MotionKind, Did, RefusalCode, Option<Escalation>, u64) -> CoreResult<Motion> =
    Motion::new;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KernelConfig;
    use au4a_core::{AgentKeys, Credits, EvidenceGrade};

    fn kernel_with(n: usize) -> Kernel {
        let mut k = Kernel::new(KernelConfig::default());
        for i in 0..n {
            let keys = AgentKeys::from_seed(&[100 + i as u8; 32]);
            k.register(&keys, format!("agent-{i}"), &["x"], Credits(20))
                .unwrap();
        }
        k
    }

    fn member(i: usize) -> Did {
        AgentKeys::from_seed(&[100 + i as u8; 32]).did()
    }

    fn motion(kind: MotionKind, subject: &Did, cause: RefusalCode) -> Motion {
        Motion::new(kind, subject.clone(), cause, None, 1).unwrap()
    }

    #[test]
    fn sortition_is_deterministic_and_only_draws_registered_agents() {
        let k = kernel_with(7);
        let a = Council::sortition(&k, 1, CouncilConfig::default()).unwrap();
        let b = Council::sortition(&k, 1, CouncilConfig::default()).unwrap();
        assert_eq!(a.members(), b.members());
        assert_eq!(a.members().len(), 5);
        for did in a.members() {
            assert!(k.is_registered(did), "委员必须是在册 Agent");
        }
        // 在册不足时取全部。
        let small = kernel_with(2);
        let c = Council::sortition(&small, 1, CouncilConfig::default()).unwrap();
        assert_eq!(c.members().len(), 2);
        // 空网络：没有委员，计票会具名报 EmptyCouncil。
        let empty = Council::sortition(
            &Kernel::new(KernelConfig::default()),
            1,
            CouncilConfig::default(),
        )
        .unwrap();
        assert!(empty.members().is_empty());
        assert_eq!(
            empty.tally("whatever").failure,
            Some(CouncilFailure::EmptyCouncil)
        );
    }

    #[test]
    fn non_member_and_duplicate_votes_are_named_failures() {
        let k = kernel_with(5);
        let mut council = Council::sortition(&k, 1, CouncilConfig::default()).unwrap();
        let subject = member(0);
        let m = motion(MotionKind::Quarantine, &subject, RefusalCode::Unauthorized);
        council.submit(m.clone()).unwrap();

        // 非委员投票：恶意（unauthorized），单次即成立。
        let outsider = AgentKeys::from_seed(&[200u8; 32]).did();
        let bad = Vote {
            voter: outsider,
            motion: m.id.clone(),
            ballot: Ballot::Uphold,
            at: 2,
        };
        assert_eq!(council.vote(bad), Err(CouncilFailure::NonMemberVote));
        assert_eq!(
            CouncilFailure::NonMemberVote.to_refusal_code(),
            RefusalCode::Unauthorized
        );
        assert!(CouncilFailure::NonMemberVote
            .to_refusal_code()
            .is_misconduct());

        // 重复投票：一人多票即欺诈。
        let voter = council.members()[0].clone();
        let first = Vote {
            voter: voter.clone(),
            motion: m.id.clone(),
            ballot: Ballot::Uphold,
            at: 3,
        };
        assert!(council.vote(first).is_ok());
        let again = Vote {
            voter,
            motion: m.id.clone(),
            ballot: Ballot::Reject,
            at: 4,
        };
        assert_eq!(council.vote(again), Err(CouncilFailure::DuplicateVote));
        assert!(CouncilFailure::DuplicateVote
            .to_refusal_code()
            .is_misconduct());

        // 重复动议按竞争处理（冲突），未知动议按 unsupported。
        assert_eq!(
            council.submit(m.clone()),
            Err(CouncilFailure::DuplicateMotion)
        );
        assert_eq!(
            CouncilFailure::DuplicateMotion.to_refusal_code(),
            RefusalCode::Conflict
        );
        assert!(!CouncilFailure::DuplicateMotion
            .to_refusal_code()
            .is_misconduct());
        let ghost = Vote {
            voter: council.members()[1].clone(),
            motion: "deadbeef".to_string(),
            ballot: Ballot::Uphold,
            at: 5,
        };
        assert_eq!(council.vote(ghost), Err(CouncilFailure::UnknownMotion));
        assert_eq!(council.failures().len(), 4);
    }

    #[test]
    fn quorum_and_majority_are_integer_thresholds() {
        let k = kernel_with(5);
        let config = CouncilConfig::default();
        let mut council = Council::sortition(&k, 1, config).unwrap();
        assert_eq!(council.quorum_required(), 3, "5 人的 60% 向上取整是 3");
        let subject = member(0);
        let m = motion(MotionKind::Warn, &subject, RefusalCode::Timeout);
        council.submit(m.clone()).unwrap();

        // 只有 2 人出席 → 不足法定人数。
        for i in 0..2 {
            council
                .vote(Vote {
                    voter: council.members()[i].clone(),
                    motion: m.id.clone(),
                    ballot: Ballot::Uphold,
                    at: 1,
                })
                .unwrap();
        }
        let t = council.tally(&m.id);
        assert!(!t.quorum_met);
        assert_eq!(t.failure, Some(CouncilFailure::NoQuorum));
        assert_eq!(t.verdict, Verdict::Inconclusive);

        // 第三人出席，3 票赞成 → 通过。
        council
            .vote(Vote {
                voter: council.members()[2].clone(),
                motion: m.id.clone(),
                ballot: Ballot::Uphold,
                at: 2,
            })
            .unwrap();
        let t = council.tally(&m.id);
        assert!(t.quorum_met);
        assert_eq!(t.verdict, Verdict::Upheld);
        assert_eq!(t.failure, None);

        // 再来两票反对：3 赞成 / 2 反对，pass_bps = 51% → 仍然通过。
        for i in 3..5 {
            council
                .vote(Vote {
                    voter: council.members()[i].clone(),
                    motion: m.id.clone(),
                    ballot: Ballot::Reject,
                    at: 3,
                })
                .unwrap();
        }
        let t = council.tally(&m.id);
        assert_eq!((t.uphold, t.reject), (3, 2));
        assert_eq!(t.verdict, Verdict::Upheld);
    }

    #[test]
    fn a_tie_or_all_abstain_is_inconclusive() {
        let k = kernel_with(4);
        let mut council = Council::sortition(&k, 1, CouncilConfig::default()).unwrap();
        let subject = member(0);
        let m = motion(MotionKind::Warn, &subject, RefusalCode::Conflict);
        council.submit(m.clone()).unwrap();
        let voters: Vec<Did> = council.members().iter().take(3).cloned().collect();
        for (i, voter) in voters.into_iter().enumerate() {
            council
                .vote(Vote {
                    voter,
                    motion: m.id.clone(),
                    ballot: Ballot::Abstain,
                    at: i as u64,
                })
                .unwrap();
        }
        let t = council.tally(&m.id);
        assert_eq!(t.failure, Some(CouncilFailure::TieInsufficient));
        assert_eq!(t.verdict, Verdict::Inconclusive);
        assert_eq!(council.decide(&m.id, 2).verdict, Verdict::Inconclusive);
        assert!(council.decisions().is_empty(), "未决动议不产生决定");
    }

    #[test]
    fn only_misconduct_codes_can_ever_reach_a_quarantine_motion() {
        // 穷举 10 个拒绝码 × 3 个升级级别：竞争码在任何升级级别下都拿不到隔离动议。
        let subject = member(0);
        for code in RefusalCode::ALL {
            for escalation in [Escalation::None, Escalation::Warn, Escalation::Quarantine] {
                let motion = Motion::from_escalation(&subject, code, escalation, 1).unwrap();
                match motion {
                    Some(m) if m.kind == MotionKind::Quarantine => {
                        assert!(
                            code.is_misconduct(),
                            "竞争码 {} 竟然拿到了隔离动议（升级级别 {:?}）",
                            code.as_str(),
                            escalation
                        );
                    }
                    Some(m) => assert_ne!(m.kind, MotionKind::Quarantine),
                    None => {}
                }
            }
        }
        // 恶意码单次即隔离动议。
        for code in RefusalCode::ALL.iter().filter(|c| c.is_misconduct()) {
            let m = Motion::from_escalation(&subject, *code, Escalation::Quarantine, 1)
                .unwrap()
                .unwrap();
            assert_eq!(m.kind, MotionKind::Quarantine);
            assert!(m.is_intact());
        }
    }

    #[test]
    fn motions_from_a_live_kernel_follow_the_classification() {
        let mut k = Kernel::new(KernelConfig::default());
        let payer = AgentKeys::from_seed(&[100u8; 32]);
        k.register(&payer, "payer", &["x"], Credits(20)).unwrap();
        let target = AgentKeys::from_seed(&[9u8; 32]).did();
        // 竞争：unverified 结算被证据闸门拒绝，重复到阈值。
        for _ in 0..au4a_core::refusal::REPEAT_THRESHOLD {
            assert!(k
                .settle(&payer.did(), &target, Credits(1), EvidenceGrade::Unverified)
                .is_err());
        }
        let motions = motions_for_kernel(&k, 10).unwrap();
        assert_eq!(motions.len(), 1);
        assert_eq!(motions[0].kind, MotionKind::Warn, "竞争只到警告");
        assert_eq!(motions[0].cause, RefusalCode::PolicyDenied);

        // 违规升级为隔离的唯一入口是恶意码；这里没有恶意码，因此没有任何隔离动议。
        assert!(motions.iter().all(|m| m.kind != MotionKind::Quarantine));
        assert!(motions
            .iter()
            .all(|m| m.cause.is_misconduct() == (m.kind == MotionKind::Quarantine)));
    }

    #[test]
    fn a_council_decision_is_deterministic_and_recorded() {
        let run = || {
            let k = kernel_with(5);
            let mut council = Council::sortition(&k, 7, CouncilConfig::default()).unwrap();
            let subject = member(1);
            let m = motion(MotionKind::Quarantine, &subject, RefusalCode::Unauthorized);
            council.submit(m.clone()).unwrap();
            for i in 0..4 {
                council
                    .vote(Vote {
                        voter: council.members()[i].clone(),
                        motion: m.id.clone(),
                        ballot: if i == 3 {
                            Ballot::Reject
                        } else {
                            Ballot::Uphold
                        },
                        at: i as u64,
                    })
                    .unwrap();
            }
            let tally = council.decide(&m.id, 99);
            (
                tally,
                council.decisions().to_vec(),
                council.decisions_json().unwrap(),
            )
        };
        let a = run();
        let b = run();
        assert_eq!(a, b);
        assert_eq!(a.0.verdict, Verdict::Upheld);
        assert_eq!(a.1.len(), 1);
        assert_eq!(a.2["fingerprint"], b.2["fingerprint"]);
        assert_eq!(a.2["failures"].as_array().map(|f| f.len()), Some(0));
    }
}
