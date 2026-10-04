//! v1.4.5 争议仲裁：Agent **自主**发起、自主投票、自主申诉。
//!
//! 参考项目把仲裁权交给运营方（`Authority::Operator`），于是「争议」实际上是人类裁决。
//! AU4A 里没有任何人类裁决路径：任一方 Agent 都可以立案，其他 Agent 自己加权投票，
//! 票权过阈值才罚没，输了的一方可以**申诉**（重新投票），而罚没**永远不超过锁定余额**。
//!
//! 三条硬约束：
//!
//! 1. **证据闸门**：[`EvidenceGrade::Unverified`] 不能立案（不可作为罚没依据）；
//!    [`EvidenceGrade::CpuProto`] 的罚没不超过 `cpu_proto_cap`；
//! 2. **罚没上限 = 锁定余额**：`min(索赔额, 证据上限, 锁定余额)`，罚没只会销毁，不会转给谁；
//! 3. **申诉入口**：窗口内、次数上限内可申诉；申诉重开投票，但**已销毁的罚没不可退回**
//!    （销毁不可逆，第二次裁决只会「确认或加重」，不会退款——这一点由测试钉住）。
//!
//! 错误码映射（`CoreError` 是冻结枚举，只有 16 个变体，没有 `Conflict`/`Unauthorized`）：
//! 状态机冲突（重复投票、票数不足、窗口已过、次数超限、重复裁决、越序操作）→ [`state_conflict`]
//! （`InvalidKind`：当前状态不接受这个动作）；利益相关者投票 / 非当事人申诉 → [`not_a_party`]
//! （`UnknownAgent`：该 Agent 不是本庭合格当事人）；证据等级不合法 → `InvalidKind`；
//! 未知案件 → `UnknownAgent`。映射写在代码里、也写进 `docs/tracks/1.4.md`，不新增错误类型。

use std::collections::BTreeMap;

use au4a_core::{CoreError, CoreResult, Credits, Did, EvidenceGrade, Ledger};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 案件状态机。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisputeState {
    /// 已立案，尚未投票。
    Open,
    /// 投票中。
    Voting,
    /// 已裁决（可申诉）。
    Ruled,
    /// 申诉后重新投票。
    Appealed,
    /// 已结案。
    Closed,
}

impl DisputeState {
    pub fn as_str(self) -> &'static str {
        match self {
            DisputeState::Open => "open",
            DisputeState::Voting => "voting",
            DisputeState::Ruled => "ruled",
            DisputeState::Appealed => "appealed",
            DisputeState::Closed => "closed",
        }
    }
}

/// 仲裁票（票权是基点，Agent 自己申报；不可转让信誉由 v1.9 提供权重来源）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vote {
    pub arbiter: String,
    /// `true` = 支持罚没。
    pub uphold: bool,
    pub weight_bp: i64,
}

/// 一次申诉。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Appeal {
    pub by: String,
    pub reason: String,
    pub at: u64,
    pub round: u32,
}

/// 罚没的上限来源（观察层要能看出「为什么只罚了这么多」）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PenaltyCap {
    /// 没有被上限截断。
    None,
    /// 被 `cpu-proto` 证据上限截断。
    EvidenceCap,
    /// 被锁定余额截断（罚没上限 = 锁定余额）。
    LockedBalance,
}

impl PenaltyCap {
    pub fn as_str(self) -> &'static str {
        match self {
            PenaltyCap::None => "none",
            PenaltyCap::EvidenceCap => "evidence_cap",
            PenaltyCap::LockedBalance => "locked_balance",
        }
    }
}

/// 一个争议案件。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dispute {
    pub id: String,
    pub claimant: String,
    pub respondent: String,
    pub claimed: Credits,
    pub evidence: EvidenceGrade,
    pub votes: Vec<Vote>,
    pub state: DisputeState,
    pub opened_at: u64,
    pub ruled_at: Option<u64>,
    /// 已销毁的罚没总量（不可逆）。
    pub slashed: Credits,
    pub appeals: Vec<Appeal>,
}

impl Dispute {
    /// 支持罚没的票权（基点）。
    pub fn uphold_bp(&self) -> CoreResult<i64> {
        let mut uphold = 0i64;
        let mut total = 0i64;
        for v in &self.votes {
            total = total.checked_add(v.weight_bp).ok_or(CoreError::Overflow)?;
            if v.uphold {
                uphold = uphold.checked_add(v.weight_bp).ok_or(CoreError::Overflow)?;
            }
        }
        if total <= 0 {
            return Ok(0);
        }
        uphold.checked_mul(10_000).map(|v| v / total).ok_or(CoreError::Overflow)
    }
}

/// 仲裁条款（网络底线；阈值与窗口由规则固定，人类不改写）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArbitrationTerms {
    /// 最少票数（法定人数）。
    pub quorum: usize,
    /// 支持罚没的票权比例阈值（基点）。
    pub uphold_threshold_bp: i64,
    /// 申诉窗口（逻辑时间片，从裁决时刻起算）。
    pub appeal_window_ticks: u64,
    /// 每个案件最多申诉次数。
    pub max_appeals: u32,
    /// `cpu-proto` 证据的罚没上限。
    pub cpu_proto_cap: Credits,
}

impl ArbitrationTerms {
    /// 默认条款：法定 2 票、过半票权（5000bp）、申诉窗口 10 个时间片、最多 1 次申诉、
    /// `cpu-proto` 上限 100 微积分。
    pub const DEFAULT: ArbitrationTerms = ArbitrationTerms {
        quorum: 2,
        uphold_threshold_bp: 5_000,
        appeal_window_ticks: 10,
        max_appeals: 1,
        cpu_proto_cap: Credits(100),
    };

    pub fn validate(&self) -> CoreResult<()> {
        if self.uphold_threshold_bp < 0 {
            return Err(CoreError::NegativeAmount);
        }
        if self.uphold_threshold_bp > 10_000 {
            return Err(CoreError::InvalidKind);
        }
        Ok(())
    }
}

impl Default for ArbitrationTerms {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 一次裁决的结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ruling {
    pub case_id: String,
    pub upheld: bool,
    /// 本次实际销毁的量。
    pub slashed: Credits,
    /// 本案累计销毁的量。
    pub slashed_total: Credits,
    pub locked_before: Credits,
    pub cap: PenaltyCap,
    pub uphold_bp: i64,
    pub at: u64,
}

/// 证据是否可作为罚没依据。
pub fn is_admissible(evidence: EvidenceGrade) -> bool {
    evidence != EvidenceGrade::Unverified
}

/// 状态机冲突的冻结错误映射：`CoreError` 没有 `Conflict` 变体，
/// 统一用 `InvalidKind` 表示「当前状态不接受这个动作」。
fn state_conflict() -> CoreError {
    CoreError::InvalidKind
}

/// 无权参与本案（利益相关者想投票 / 非当事人想申诉）的冻结错误映射。
fn not_a_party() -> CoreError {
    CoreError::UnknownAgent
}

/// 仲裁庭：确定性顺序（案件按 id 字节序）。
#[derive(Clone, Debug, Default)]
pub struct Court {
    cases: BTreeMap<String, Dispute>,
}

impl Court {
    pub fn new() -> Self {
        Self::default()
    }

    /// 立案（任一方 Agent 都能发起，不需要人类批准）。
    pub fn open(
        &mut self,
        claimant: &Did,
        respondent: &Did,
        claimed: Credits,
        evidence: EvidenceGrade,
        now: u64,
    ) -> CoreResult<Dispute> {
        if !is_admissible(evidence) {
            return Err(CoreError::InvalidKind);
        }
        if claimed == Credits::ZERO {
            return Err(CoreError::ZeroAmount);
        }
        if claimant == respondent {
            return Err(CoreError::InvalidKind);
        }
        let payload = json!({
            "claimant": claimant,
            "respondent": respondent,
            "claimed": claimed,
            "evidence": evidence,
            "at": now,
        });
        let id = au4a_core::canonical_hash(&payload)?;
        let dispute = Dispute {
            id: id.clone(),
            claimant: claimant.as_str().to_string(),
            respondent: respondent.as_str().to_string(),
            claimed,
            evidence,
            votes: Vec::new(),
            state: DisputeState::Open,
            opened_at: now,
            ruled_at: None,
            slashed: Credits::ZERO,
            appeals: Vec::new(),
        };
        self.cases.insert(id, dispute.clone());
        Ok(dispute)
    }

    /// Agent 自主投票。利益相关者不能投票；同一仲裁者不能重复投票。
    pub fn vote(
        &mut self,
        case_id: &str,
        arbiter: &Did,
        uphold: bool,
        weight_bp: i64,
    ) -> CoreResult<()> {
        if weight_bp < 0 {
            return Err(CoreError::NegativeAmount);
        }
        if weight_bp > 10_000 {
            return Err(CoreError::InvalidKind);
        }
        let case = self.cases.get_mut(case_id).ok_or(CoreError::UnknownAgent)?;
        match case.state {
            DisputeState::Open | DisputeState::Voting | DisputeState::Appealed => {}
            _ => return Err(state_conflict()),
        }
        let who = arbiter.as_str();
        if who == case.claimant || who == case.respondent {
            return Err(not_a_party());
        }
        if case.votes.iter().any(|v| v.arbiter == who) {
            return Err(state_conflict());
        }
        case.votes.push(Vote {
            arbiter: who.to_string(),
            uphold,
            weight_bp,
        });
        case.state = DisputeState::Voting;
        Ok(())
    }

    /// 裁决：票数达法定人数且支持票权过阈值 → 罚没，且**不超过锁定余额**。
    pub fn rule(
        &mut self,
        ledger: &mut Ledger,
        case_id: &str,
        terms: &ArbitrationTerms,
        now: u64,
    ) -> CoreResult<Ruling> {
        terms.validate()?;
        let case = self.cases.get_mut(case_id).ok_or(CoreError::UnknownAgent)?;
        match case.state {
            DisputeState::Open | DisputeState::Voting | DisputeState::Appealed => {}
            _ => return Err(state_conflict()),
        }
        if case.votes.len() < terms.quorum {
            return Err(state_conflict());
        }
        let uphold_bp = case.uphold_bp()?;
        let upheld = uphold_bp >= terms.uphold_threshold_bp;
        let respondent = Did::parse(&case.respondent)?;
        let locked_before = ledger.balance(&respondent).locked;
        let already = case.slashed;

        let mut cap = PenaltyCap::None;
        let mut target = Credits::ZERO;
        if upheld {
            target = case.claimed;
            if case.evidence == EvidenceGrade::CpuProto && target > terms.cpu_proto_cap {
                target = terms.cpu_proto_cap;
                cap = PenaltyCap::EvidenceCap;
            }
            // 罚没上限 = 锁定余额：本金与「已销毁」共享同一个上限，绝不会罚到不存在的东西。
            let ceiling = locked_before.checked_add(already)?;
            if target > ceiling {
                target = ceiling;
                cap = PenaltyCap::LockedBalance;
            }
        }
        let slash_now = if target > already {
            target.checked_sub(already)?
        } else {
            Credits::ZERO
        };
        if slash_now != Credits::ZERO {
            ledger.slash(&respondent, slash_now)?;
            ledger.check_conservation()?;
        }
        case.slashed = already.checked_add(slash_now)?;
        case.state = DisputeState::Ruled;
        case.ruled_at = Some(now);

        Ok(Ruling {
            case_id: case.id.clone(),
            upheld,
            slashed: slash_now,
            slashed_total: case.slashed,
            locked_before,
            cap,
            uphold_bp,
            at: now,
        })
    }

    /// 申诉：窗口内、次数上限内，当事人才能提起；申诉重开投票（原票作废）。
    ///
    /// 已销毁的罚没**不会**因为申诉而退回——销毁不可逆，第二次裁决只可能确认或加重。
    pub fn appeal(
        &mut self,
        case_id: &str,
        by: &Did,
        reason: &str,
        terms: &ArbitrationTerms,
        now: u64,
    ) -> CoreResult<Appeal> {
        terms.validate()?;
        if reason.trim().is_empty() {
            return Err(CoreError::InvalidKind);
        }
        let case = self.cases.get_mut(case_id).ok_or(CoreError::UnknownAgent)?;
        if case.state != DisputeState::Ruled {
            return Err(state_conflict());
        }
        let who = by.as_str();
        if who != case.claimant && who != case.respondent {
            return Err(not_a_party());
        }
        if case.appeals.len() as u32 >= terms.max_appeals {
            return Err(state_conflict());
        }
        let ruled_at = case.ruled_at.ok_or_else(state_conflict)?;
        if now > ruled_at.saturating_add(terms.appeal_window_ticks) {
            return Err(state_conflict());
        }
        let appeal = Appeal {
            by: who.to_string(),
            reason: reason.to_string(),
            at: now,
            round: case.appeals.len() as u32 + 1,
        };
        case.appeals.push(appeal.clone());
        case.votes.clear();
        case.state = DisputeState::Appealed;
        Ok(appeal)
    }

    /// 结案（已裁决且不再申诉）。
    pub fn close(&mut self, case_id: &str) -> CoreResult<()> {
        let case = self.cases.get_mut(case_id).ok_or(CoreError::UnknownAgent)?;
        if case.state != DisputeState::Ruled {
            return Err(state_conflict());
        }
        case.state = DisputeState::Closed;
        Ok(())
    }

    pub fn case(&self, case_id: &str) -> CoreResult<Dispute> {
        self.cases
            .get(case_id)
            .cloned()
            .ok_or(CoreError::UnknownAgent)
    }

    pub fn case_json(&self, case_id: &str) -> CoreResult<Value> {
        let case = self.case(case_id)?;
        serde_json::to_value(case).map_err(|_| CoreError::Encoding)
    }

    pub fn open_cases(&self) -> usize {
        self.cases
            .values()
            .filter(|c| c.state != DisputeState::Closed)
            .count()
    }

    pub fn total_cases(&self) -> usize {
        self.cases.len()
    }

    /// 全庭累计销毁量。
    pub fn total_slashed(&self) -> CoreResult<Credits> {
        let mut sum = Credits::ZERO;
        for case in self.cases.values() {
            sum = sum.checked_add(case.slashed)?;
        }
        Ok(sum)
    }

    /// 只读 JSON 投影（监控面板数据源的一部分）。
    pub fn to_json(&self) -> Value {
        json!({
            "cases": self.cases.values().collect::<Vec<_>>(),
            "total_cases": self.total_cases(),
            "open_cases": self.open_cases(),
            "total_slashed": self.total_slashed().unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use au4a_core::AgentKeys;

    fn did(seed: u8) -> Did {
        AgentKeys::from_seed(&[seed; 32]).did()
    }

    fn funded(seed: u8, amount: i64, locked: i64) -> (Did, Ledger) {
        let who = did(seed);
        let mut ledger = Ledger::new();
        ledger.mint(&who, Credits(amount)).unwrap();
        if locked > 0 {
            ledger.lock(&who, Credits(locked)).unwrap();
        }
        (who, ledger)
    }

    fn case_with_votes(
        court: &mut Court,
        claimant: &Did,
        respondent: &Did,
        claimed: i64,
        evidence: EvidenceGrade,
        uphold: bool,
    ) -> String {
        let case = court
            .open(claimant, respondent, Credits(claimed), evidence, 1)
            .unwrap();
        court.vote(&case.id, &did(90), uphold, 6_000).unwrap();
        court.vote(&case.id, &did(91), uphold, 4_000).unwrap();
        case.id
    }

    #[test]
    fn unverified_evidence_cannot_open_a_case() {
        let (a, b) = (did(1), did(2));
        let mut court = Court::new();
        assert_eq!(
            court.open(&a, &b, Credits(100), EvidenceGrade::Unverified, 1),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(court.total_cases(), 0);
        assert!(!is_admissible(EvidenceGrade::Unverified));
        assert!(is_admissible(EvidenceGrade::CpuProto));
        assert!(is_admissible(EvidenceGrade::Verified));
    }

    #[test]
    fn degenerate_cases_are_refused() {
        let a = did(3);
        let mut court = Court::new();
        assert_eq!(
            court.open(&a, &did(4), Credits::ZERO, EvidenceGrade::Verified, 1),
            Err(CoreError::ZeroAmount)
        );
        assert_eq!(
            court.open(&a, &a, Credits(10), EvidenceGrade::Verified, 1),
            Err(CoreError::InvalidKind)
        );
    }

    #[test]
    fn interested_parties_cannot_vote_and_votes_are_unique() {
        let (claimant, respondent) = (did(5), did(6));
        let mut court = Court::new();
        let case = court
            .open(&claimant, &respondent, Credits(10), EvidenceGrade::Verified, 1)
            .unwrap();
        assert_eq!(
            court.vote(&case.id, &claimant, true, 5_000),
            Err(CoreError::UnknownAgent)
        );
        assert_eq!(
            court.vote(&case.id, &respondent, false, 5_000),
            Err(CoreError::UnknownAgent)
        );
        court.vote(&case.id, &did(7), true, 5_000).unwrap();
        assert_eq!(
            court.vote(&case.id, &did(7), true, 5_000),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(
            court.vote(&case.id, &did(8), true, 10_001),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(
            court.vote(&case.id, &did(8), true, -1),
            Err(CoreError::NegativeAmount)
        );
        assert_eq!(court.vote("nope", &did(8), true, 1), Err(CoreError::UnknownAgent));
    }

    #[test]
    fn a_case_needs_quorum_and_a_threshold() {
        let (claimant, respondent) = (did(9), did(10));
        let mut court = Court::new();
        let terms = ArbitrationTerms::DEFAULT;
        let case = court
            .open(&claimant, &respondent, Credits(50), EvidenceGrade::Verified, 1)
            .unwrap();
        court.vote(&case.id, &did(11), true, 9_000).unwrap();
        let (_, mut ledger) = funded(10, 1_000, 500);
        // 只有 1 票 < 法定 2 票。
        assert_eq!(
            court.rule(&mut ledger, &case.id, &terms, 2),
            Err(CoreError::InvalidKind)
        );
        court.vote(&case.id, &did(12), false, 1_000).unwrap();
        // 支持票权 9000/10000 = 9000bp ≥ 5000bp → 通过，罚没 50（未触上限）。
        let ruling = court.rule(&mut ledger, &case.id, &terms, 3).unwrap();
        assert!(ruling.upheld);
        assert_eq!(ruling.uphold_bp, 9_000);
        assert_eq!(ruling.slashed, Credits(50));
        assert_eq!(ruling.cap, PenaltyCap::None);
        assert_eq!(ledger.slashed(), Credits(50));
        ledger.check_conservation().unwrap();
    }

    #[test]
    fn a_rejected_case_slashes_nothing() {
        let (claimant, respondent) = (did(13), did(14));
        let mut court = Court::new();
        let (_, mut ledger) = funded(14, 1_000, 500);
        let case = court
            .open(&claimant, &respondent, Credits(400), EvidenceGrade::Verified, 1)
            .unwrap();
        court.vote(&case.id, &did(15), false, 6_000).unwrap();
        court.vote(&case.id, &did(16), true, 4_000).unwrap();
        let ruling = court
            .rule(&mut ledger, &case.id, &ArbitrationTerms::DEFAULT, 5)
            .unwrap();
        assert!(!ruling.upheld);
        assert_eq!(ruling.uphold_bp, 4_000); // 4 000 < 5 000 阈值
        assert_eq!(ruling.slashed, Credits::ZERO);
        assert_eq!(ruling.cap, PenaltyCap::None);
        assert_eq!(ledger.slashed(), Credits::ZERO);
        assert_eq!(ledger.balance(&respondent).locked, Credits(500));
        ledger.check_conservation().unwrap();
    }

    #[test]
    fn penalty_never_exceeds_the_locked_balance() {
        let (claimant, respondent) = (did(20), did(21));
        let mut court = Court::new();
        // 锁定只有 50，索赔 1_000_000 —— 罚没必须被锁定余额截断。
        let (_, mut ledger) = funded(21, 10_000, 50);
        let case = court
            .open(&claimant, &respondent, Credits(1_000_000), EvidenceGrade::Verified, 1)
            .unwrap();
        court.vote(&case.id, &did(22), true, 10_000).unwrap();
        court.vote(&case.id, &did(23), true, 10_000).unwrap();
        let ruling = court
            .rule(&mut ledger, &case.id, &ArbitrationTerms::DEFAULT, 5)
            .unwrap();
        assert!(ruling.upheld);
        assert_eq!(ruling.locked_before, Credits(50));
        assert_eq!(ruling.slashed, Credits(50));
        assert_eq!(ruling.cap, PenaltyCap::LockedBalance);
        assert_eq!(ledger.slashed(), Credits(50));
        assert_eq!(ledger.balance(&respondent).locked, Credits::ZERO);
        assert_eq!(ledger.balance(&respondent).available, Credits(9_950));
        ledger.check_conservation().unwrap();
    }

    #[test]
    fn cpu_proto_penalties_are_capped_by_evidence() {
        let (claimant, respondent) = (did(24), did(25));
        let mut court = Court::new();
        let (_, mut ledger) = funded(25, 10_000, 5_000);
        let case = court
            .open(&claimant, &respondent, Credits(10_000), EvidenceGrade::CpuProto, 1)
            .unwrap();
        court.vote(&case.id, &did(26), true, 6_000).unwrap();
        court.vote(&case.id, &did(27), true, 4_000).unwrap();
        let terms = ArbitrationTerms::DEFAULT;
        let ruling = court.rule(&mut ledger, &case.id, &terms, 2).unwrap();
        assert_eq!(ruling.slashed, terms.cpu_proto_cap);
        assert_eq!(ruling.slashed, Credits(100));
        assert_eq!(ruling.cap, PenaltyCap::EvidenceCap);
        ledger.check_conservation().unwrap();
    }

    #[test]
    fn an_appeal_reopens_voting_and_can_never_refund() {
        let (claimant, respondent) = (did(30), did(31));
        let mut court = Court::new();
        let (_, mut ledger) = funded(31, 1_000, 400);
        let terms = ArbitrationTerms::DEFAULT;
        let case = court
            .open(&claimant, &respondent, Credits(300), EvidenceGrade::Verified, 1)
            .unwrap();
        court.vote(&case.id, &did(32), true, 7_000).unwrap();
        court.vote(&case.id, &did(33), true, 3_000).unwrap();
        let first = court.rule(&mut ledger, &case.id, &terms, 10).unwrap();
        assert_eq!(first.slashed, Credits(300));
        assert_eq!(ledger.slashed(), Credits(300));

        // 窗口内申诉：票作废、回到投票。
        let appeal = court.appeal(&case.id, &respondent, "new evidence", &terms, 15).unwrap();
        assert_eq!(appeal.round, 1);
        assert_eq!(court.case(&case.id).unwrap().votes.len(), 0);
        assert_eq!(court.case(&case.id).unwrap().state, DisputeState::Appealed);

        // 第二次裁决改为驳回：已销毁的 300 不会退回（销毁不可逆）。
        court.vote(&case.id, &did(32), false, 7_000).unwrap();
        court.vote(&case.id, &did(33), false, 3_000).unwrap();
        let second = court.rule(&mut ledger, &case.id, &terms, 20).unwrap();
        assert!(!second.upheld);
        assert_eq!(second.slashed, Credits::ZERO);
        assert_eq!(second.slashed_total, Credits(300));
        assert_eq!(ledger.slashed(), Credits(300));
        ledger.check_conservation().unwrap();

        // 第三次申诉：次数上限（max_appeals = 1）已用尽。
        assert_eq!(
            court.appeal(&case.id, &claimant, "again", &terms, 21),
            Err(CoreError::InvalidKind)
        );
        court.close(&case.id).unwrap();
        assert_eq!(court.case(&case.id).unwrap().state, DisputeState::Closed);
        assert_eq!(court.open_cases(), 0);
    }

    #[test]
    fn appeal_window_and_count_are_enforced() {
        let (claimant, respondent) = (did(40), did(41));
        let mut court = Court::new();
        let (_, mut ledger) = funded(41, 1_000, 200);
        let terms = ArbitrationTerms::DEFAULT; // 窗口 10、最多 1 次
        let case = court
            .open(&claimant, &respondent, Credits(100), EvidenceGrade::Verified, 1)
            .unwrap();
        court.vote(&case.id, &did(42), false, 10_000).unwrap();
        court.vote(&case.id, &did(43), false, 10_000).unwrap();
        court.rule(&mut ledger, &case.id, &terms, 100).unwrap(); // ruled_at = 100
        // 非当事人不能申诉。
        assert_eq!(
            court.appeal(&case.id, &did(44), "let me in", &terms, 101),
            Err(CoreError::UnknownAgent)
        );
        // 窗口外不能申诉。
        assert_eq!(
            court.appeal(&case.id, &claimant, "too late", &terms, 111),
            Err(CoreError::InvalidKind)
        );
        // 空理由不能申诉。
        assert_eq!(
            court.appeal(&case.id, &claimant, "   ", &terms, 101),
            Err(CoreError::InvalidKind)
        );
        court.appeal(&case.id, &claimant, "within window", &terms, 105).unwrap();
        // 已经申诉过一次：再次申诉冲突（state 也不是 Ruled）。
        assert_eq!(
            court.appeal(&case.id, &respondent, "again", &terms, 106),
            Err(CoreError::InvalidKind)
        );
        // 申诉后重新投票并裁决。
        court.vote(&case.id, &did(42), true, 10_000).unwrap();
        court.vote(&case.id, &did(43), true, 10_000).unwrap();
        let second = court.rule(&mut ledger, &case.id, &terms, 120).unwrap();
        assert!(second.upheld);
        assert_eq!(second.slashed, Credits(100));
        assert_eq!(ledger.slashed(), Credits(100));
        court.close(&case.id).unwrap();
        assert_eq!(court.open_cases(), 0);
        assert_eq!(court.total_slashed().unwrap(), Credits(100));
    }

    #[test]
    fn the_state_machine_refuses_out_of_order_actions() {
        let (claimant, respondent) = (did(50), did(51));
        let mut court = Court::new();
        let (_, mut ledger) = funded(51, 1_000, 100);
        let terms = ArbitrationTerms::DEFAULT;
        let case = court
            .open(&claimant, &respondent, Credits(10), EvidenceGrade::Verified, 1)
            .unwrap();
        // 未达法定人数不能裁决。
        assert_eq!(
            court.rule(&mut ledger, &case.id, &terms, 2),
            Err(CoreError::InvalidKind)
        );
        // 未裁决不能申诉 / 结案。
        assert_eq!(
            court.appeal(&case.id, &claimant, "early", &terms, 2),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(court.close(&case.id), Err(CoreError::InvalidKind));
        court.vote(&case.id, &did(52), false, 10_000).unwrap();
        court.vote(&case.id, &did(53), false, 10_000).unwrap();
        court.rule(&mut ledger, &case.id, &terms, 3).unwrap();
        // 裁决后不能再次裁决 / 投票。
        assert_eq!(
            court.rule(&mut ledger, &case.id, &terms, 4),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(
            court.vote(&case.id, &did(54), true, 10_000),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(court.case("nope"), Err(CoreError::UnknownAgent));
        assert_eq!(court.close(&case.id), Ok(()));
        assert_eq!(court.close(&case.id), Err(CoreError::InvalidKind));
    }

    #[test]
    fn the_court_projection_is_float_free() {
        let (claimant, respondent) = (did(60), did(61));
        let mut court = Court::new();
        let (_, mut ledger) = funded(61, 1_000, 100);
        let case = case_with_votes(
            &mut court,
            &claimant,
            &respondent,
            100,
            EvidenceGrade::Verified,
            true,
        );
        court
            .rule(&mut ledger, &case, &ArbitrationTerms::DEFAULT, 9)
            .unwrap();
        let value = court.to_json();
        let canonical = au4a_core::canonicalize(&value).unwrap();
        assert!(!canonical.contains('.'));
        assert_eq!(value["total_cases"], json!(1));
        assert_eq!(value["total_slashed"], json!(100));
        assert!(court.case_json(&case).unwrap()["id"].is_string());
    }
}
