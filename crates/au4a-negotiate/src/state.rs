//! v1.2.2 — 协商状态机。
//!
//! ```text
//! IDLE ──Request──▶ NEGOTIATING ──Accept──▶ ACCEPTED ──Sign──▶ CONTRACT_SIGNED
//!                       │  ▲                                          │
//!              Counter ─┘  └─ Reject（不消耗轮数额度）                  Execute
//!                                                                     ▼
//!   CONTRACT_SIGNED ──Breach──▶ ARBITRATION ──Resolve──▶ SETTLED ◀── Settle ── EXECUTING
//!   EXECUTING       ──Breach──▶ ARBITRATION
//! ```
//!
//! 两条不可协商的规则：
//!
//! 1. **非法转换必须被拒**：`transition()` 对全部 (相位 × 事件) 组合给出唯一答案，
//!    组合之外一律 `Err`，绝不「容忍式推进」。测试穷举 7 × 9 = 63 种组合。
//! 2. **每次转换双方签名**：一条 [`TransitionRecord`] 只有两种授权方式——
//!    * [`Basis::DualConsent`]：双方当场各自用自己的 Ed25519 私钥签署同一条转换；
//!    * [`Basis::ContractClause`]：违约申诉由**双方已签署的合约**预先授权，
//!      申诉方单签 + 出示一份双签合约（[`DualSigned`] 见证）。
//!
//!    任何第三方签名、重复签名、缺一方签名的记录都在 `commit` 阶段被拒，
//!    且签名覆盖 `session/seq/round/from/to/event/at/basis` 全字段——改一个字节就废。
//!
//! 错误映射：`au4a_core::CoreError` 是冻结枚举（不新增变体），本轨道把
//! 「该事件在当前相位下不是合法消息类型」映射为 [`CoreError::InvalidKind`]，
//! 把「签名/授权不成立」映射为 [`CoreError::InvalidSignature`] 或 [`CoreError::NotSealed`]。

use au4a_core::{canonical_hash, canonicalize, AgentKeys, CoreError, CoreResult, Did};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 协商相位。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// 尚未开局。
    Idle,
    /// 多轮报价中。
    Negotiating,
    /// 条款已被接受，等待签合约。
    Accepted,
    /// 合约已双方签署。
    ContractSigned,
    /// 执行中（交付尚未结算）。
    Executing,
    /// 已结算（终态）。
    Settled,
    /// 违约争议中（v1.2.7 仲裁接入）。
    Arbitration,
}

impl Phase {
    /// 七个相位的固定顺序（测试与文档共用）。
    pub const ALL: [Phase; 7] = [
        Phase::Idle,
        Phase::Negotiating,
        Phase::Accepted,
        Phase::ContractSigned,
        Phase::Executing,
        Phase::Settled,
        Phase::Arbitration,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Idle => "idle",
            Phase::Negotiating => "negotiating",
            Phase::Accepted => "accepted",
            Phase::ContractSigned => "contract_signed",
            Phase::Executing => "executing",
            Phase::Settled => "settled",
            Phase::Arbitration => "arbitration",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Phase::ALL.into_iter().find(|p| p.as_str() == s)
    }

    /// 终态：不接受任何后续事件。
    pub fn is_terminal(self) -> bool {
        matches!(self, Phase::Settled)
    }

    /// 合约是否已经存在（违约条款只在这种情况下可用）。
    pub fn has_contract(self) -> bool {
        matches!(
            self,
            Phase::ContractSigned | Phase::Executing | Phase::Settled | Phase::Arbitration
        )
    }
}

/// 驱动相位变化的事件。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    /// 首次报价。
    Request,
    /// 还价（唯一的轮数额度消耗者）。
    Counter,
    /// 接受对端条款。
    Accept,
    /// 拒绝条款：协商继续，**不**消耗轮数额度。
    Reject,
    /// 签署合约。
    Sign,
    /// 开始执行。
    Execute,
    /// 结算。
    Settle,
    /// 违约申诉。
    Breach,
    /// 仲裁裁决结案。
    Resolve,
}

impl Event {
    /// 九个事件的固定顺序（穷举测试用）。
    pub const ALL: [Event; 9] = [
        Event::Request,
        Event::Counter,
        Event::Accept,
        Event::Reject,
        Event::Sign,
        Event::Execute,
        Event::Settle,
        Event::Breach,
        Event::Resolve,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Event::Request => "request",
            Event::Counter => "counter",
            Event::Accept => "accept",
            Event::Reject => "reject",
            Event::Sign => "sign",
            Event::Execute => "execute",
            Event::Settle => "settle",
            Event::Breach => "breach",
            Event::Resolve => "resolve",
        }
    }

    /// 是否消耗报价轮数额度。`Reject` 明确不占额度：拒绝不是还价。
    pub fn consumes_round(self) -> bool {
        matches!(self, Event::Counter)
    }
}

/// 合法转换表——状态机的**唯一**事实来源，`transition()` 与穷举测试都读它。
pub const LEGAL_TRANSITIONS: [(Phase, Event, Phase); 10] = [
    (Phase::Idle, Event::Request, Phase::Negotiating),
    (Phase::Negotiating, Event::Counter, Phase::Negotiating),
    (Phase::Negotiating, Event::Reject, Phase::Negotiating),
    (Phase::Negotiating, Event::Accept, Phase::Accepted),
    (Phase::Accepted, Event::Sign, Phase::ContractSigned),
    (Phase::ContractSigned, Event::Execute, Phase::Executing),
    (Phase::ContractSigned, Event::Breach, Phase::Arbitration),
    (Phase::Executing, Event::Settle, Phase::Settled),
    (Phase::Executing, Event::Breach, Phase::Arbitration),
    (Phase::Arbitration, Event::Resolve, Phase::Settled),
];

/// 纯函数转换。非法组合返回 `Err`——**不是** panic，也不是「保持不变」。
pub fn transition(from: Phase, event: Event) -> CoreResult<Phase> {
    for (f, e, t) in LEGAL_TRANSITIONS {
        if f == from && e == event {
            return Ok(t);
        }
    }
    Err(CoreError::InvalidKind)
}

/// 一条转换记录的授权方式。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Basis {
    /// 双方当场联署。
    DualConsent,
    /// 违约条款预授权：合约在签订时已被双方接受「违约进仲裁」。
    ContractClause {
        contract_id: String,
        contract_hash: String,
    },
}

/// 一方的签名。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PartySignature {
    pub did: Did,
    /// hex 编码的 Ed25519 签名。
    pub sig: String,
}

/// 双签合约的最小见证接口。
///
/// 状态机在 v1.2.2 就要求「违约进仲裁」必须有双签合约撑腰，但合约类型要到 v1.2.5 才出现；
/// 用这个窄 trait 解耦，避免为了一个布尔判断让状态机依赖尚未实现的模块。
pub trait DualSigned {
    fn contract_id(&self) -> &str;
    fn contract_hash(&self) -> &str;
    fn is_dual_signed(&self) -> bool;
}

/// 一次状态转换的记录。签名覆盖除 `sigs` 外的全部字段。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionRecord {
    /// 会话 id（内容寻址）。
    pub session: String,
    /// 会话内序号，从 1 开始连续。
    pub seq: u32,
    /// 提交这条记录**之前**已消耗的轮数额度。
    pub round: u32,
    pub from: Phase,
    pub to: Phase,
    pub event: Event,
    /// 逻辑时钟读数（基元层不读墙钟）。
    pub at: u64,
    pub basis: Basis,
    /// 按 DID 字节序排序，保证「同样的签名集合 → 同样的字节」。
    pub sigs: Vec<PartySignature>,
}

impl TransitionRecord {
    /// 被签名的载荷（不含 `sigs`）。
    pub fn payload(&self) -> Value {
        json!({
            "session": self.session,
            "seq": self.seq,
            "round": self.round,
            "from": self.from,
            "to": self.to,
            "event": self.event,
            "at": self.at,
            "basis": self.basis,
        })
    }

    /// 内容寻址哈希：规范 JSON 的 SHA-256。journal 用它串成链。
    pub fn record_hash(&self) -> CoreResult<String> {
        canonical_hash(&self.payload())
    }

    /// 追加一方签名；同一 DID 重复签名是协议违规。
    pub fn sign(&mut self, keys: &AgentKeys) -> CoreResult<()> {
        let did = keys.did();
        if self.sigs.iter().any(|s| s.did == did) {
            return Err(CoreError::InvalidSignature);
        }
        let sig = keys.sign_json(&self.payload())?;
        self.sigs.push(PartySignature { did, sig });
        // 排序而不是保持插入序：签名顺序不应影响记录字节。
        self.sigs.sort_by(|a, b| a.did.cmp(&b.did));
        Ok(())
    }

    pub fn signed_by(&self, did: &Did) -> bool {
        self.sigs.iter().any(|s| &s.did == did)
    }

    /// 校验签名集合本身：非空、无重复 DID、每个签名都真的覆盖了本记录载荷。
    pub fn verify(&self) -> CoreResult<()> {
        if self.sigs.is_empty() {
            return Err(CoreError::NotSealed);
        }
        let bytes = canonicalize(&self.payload())?;
        let mut seen: Vec<&Did> = Vec::new();
        for s in &self.sigs {
            if seen.contains(&&s.did) {
                return Err(CoreError::InvalidSignature);
            }
            s.did.verify(bytes.as_bytes(), &s.sig)?;
            seen.push(&s.did);
        }
        match &self.basis {
            // 双方当场联署 ⇒ 恰好两方。
            Basis::DualConsent => {
                if self.sigs.len() != 2 {
                    return Err(CoreError::NotSealed);
                }
            }
            // 条款授权 ⇒ 恰好申诉方一方，且只能通向仲裁。
            Basis::ContractClause { .. } => {
                if self.sigs.len() != 1 || self.to != Phase::Arbitration {
                    return Err(CoreError::NotSealed);
                }
            }
        }
        Ok(())
    }

    /// 对「这两个人」校验：签名不得缺任何一方，也不得混进第三方。
    pub fn verify_against(&self, parties: &[Did]) -> CoreResult<()> {
        if parties.len() != 2 {
            return Err(CoreError::InvalidKind);
        }
        self.verify()?;
        if self.sigs.iter().any(|s| !parties.contains(&s.did)) {
            return Err(CoreError::InvalidSignature);
        }
        if let Basis::DualConsent = self.basis {
            for p in parties {
                if !self.signed_by(p) {
                    return Err(CoreError::NotSealed);
                }
            }
        }
        Ok(())
    }
}

/// 会话状态机。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateMachine {
    session: String,
    phase: Phase,
    seq: u32,
    round: u32,
    history: Vec<TransitionRecord>,
}

impl StateMachine {
    /// 开一个处于 `IDLE` 的会话。
    pub fn open(session: &str) -> CoreResult<Self> {
        if !crate::msg::is_label(session) {
            return Err(CoreError::InvalidKind);
        }
        Ok(Self {
            session: session.to_string(),
            phase: Phase::Idle,
            seq: 0,
            round: 0,
            history: Vec::new(),
        })
    }

    /// 由已校验的分量重建（journal 重放路径）。`history` 会立刻被校验，重建不等于免检。
    pub fn from_parts(
        session: &str,
        phase: Phase,
        seq: u32,
        round: u32,
        history: Vec<TransitionRecord>,
    ) -> CoreResult<Self> {
        let machine = Self {
            session: session.to_string(),
            phase,
            seq,
            round,
            history,
        };
        // 分量必须与历史一致：假装出来的相位不会被接受。
        let mut probe = Self::open(session)?;
        for rec in &machine.history {
            probe.check_chain(rec)?;
            probe.apply(rec.clone());
        }
        if probe.phase != machine.phase || probe.seq != machine.seq || probe.round != machine.round
        {
            return Err(CoreError::InvalidKind);
        }
        Ok(machine)
    }

    pub fn session(&self) -> &str {
        self.session.as_str()
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn seq(&self) -> u32 {
        self.seq
    }

    /// 已消耗的报价轮数额度。
    pub fn round(&self) -> u32 {
        self.round
    }

    pub fn history(&self) -> &[TransitionRecord] {
        &self.history
    }

    /// 阶段性转换：算出目标相位并产出**只签了发起方**的记录，尚未落地。
    pub fn stage(
        &self,
        event: Event,
        initiator: &AgentKeys,
        at: u64,
    ) -> CoreResult<TransitionRecord> {
        self.stage_with(event, initiator, at, Basis::DualConsent)
    }

    /// 违约条款下的阶段性转换（单签 + 双签合约见证）。
    pub fn stage_under_contract(
        &self,
        event: Event,
        initiator: &AgentKeys,
        at: u64,
        contract_id: &str,
        contract_hash: &str,
    ) -> CoreResult<TransitionRecord> {
        if event != Event::Breach {
            // 只有违约可以绕过「当场双方联署」，其余事件必须当场同意。
            return Err(CoreError::InvalidKind);
        }
        self.stage_with(
            event,
            initiator,
            at,
            Basis::ContractClause {
                contract_id: contract_id.to_string(),
                contract_hash: contract_hash.to_string(),
            },
        )
    }

    fn stage_with(
        &self,
        event: Event,
        initiator: &AgentKeys,
        at: u64,
        basis: Basis,
    ) -> CoreResult<TransitionRecord> {
        let to = transition(self.phase, event)?;
        let mut record = TransitionRecord {
            session: self.session.clone(),
            seq: self.seq + 1,
            round: self.round,
            from: self.phase,
            to,
            event,
            at,
            basis,
            sigs: Vec::new(),
        };
        record.sign(initiator)?;
        Ok(record)
    }

    /// 对端当场联署。
    pub fn co_sign(record: &mut TransitionRecord, counterparty: &AgentKeys) -> CoreResult<()> {
        record.sign(counterparty)
    }

    /// 落地一条转换记录。四项检查缺一不可：会话/序号/相位链自洽、转换合法、授权齐备。
    pub fn commit(
        &mut self,
        record: TransitionRecord,
        parties: &[Did],
        witness: Option<&dyn DualSigned>,
    ) -> CoreResult<()> {
        record.verify_against(parties)?;
        if let Basis::ContractClause {
            contract_id,
            contract_hash,
        } = &record.basis
        {
            let w = witness.ok_or(CoreError::NotSealed)?;
            if w.contract_id() != contract_id
                || w.contract_hash() != contract_hash
                || !w.is_dual_signed()
            {
                return Err(CoreError::InvalidSignature);
            }
        }
        self.check_chain(&record)?;
        self.apply(record);
        Ok(())
    }

    /// 只检查「结构 + 合法性」（不查签名），不改状态。
    fn check_chain(&self, record: &TransitionRecord) -> CoreResult<()> {
        if record.session != self.session || record.seq != self.seq + 1 || record.from != self.phase
        {
            return Err(CoreError::InvalidKind);
        }
        if record.to != transition(self.phase, record.event)? {
            return Err(CoreError::InvalidKind);
        }
        Ok(())
    }

    /// 已通过 [`StateMachine::check_chain`] 的记录在此推进状态；**只调用一次**。
    fn apply(&mut self, record: TransitionRecord) {
        if record.event.consumes_round() {
            self.round += 1;
        }
        self.seq = record.seq;
        self.phase = record.to;
        self.history.push(record);
    }

    /// 一次调用完成「发起方签 → 对端联署 → 落地」的正常路径。
    pub fn transact(
        &mut self,
        event: Event,
        initiator: &AgentKeys,
        counterparty: &AgentKeys,
        at: u64,
        parties: &[Did],
    ) -> CoreResult<TransitionRecord> {
        let mut record = self.stage(event, initiator, at)?;
        Self::co_sign(&mut record, counterparty)?;
        self.commit(record.clone(), parties, None)?;
        Ok(record)
    }

    /// 重放整条历史：重建相位/序号/轮数，逐条校验双方签名与相位链。
    ///
    /// 返回末端记录哈希（空历史时返回会话开局哈希），v1.2.3 用它做逐字节一致性比对。
    pub fn verify_history(&self, parties: &[Did]) -> CoreResult<String> {
        let mut tip = canonical_hash(&json!({"session": self.session, "seq": 0}))?;
        let mut machine = Self::open(&self.session)?;
        for record in &self.history {
            record.verify_against(parties)?;
            machine.check_chain(record)?;
            machine.apply(record.clone());
            tip = record.record_hash()?;
        }
        if machine.phase != self.phase || machine.seq != self.seq || machine.round != self.round {
            return Err(CoreError::InvalidKind);
        }
        Ok(tip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg::Terms;
    use au4a_core::{Credits, EvidenceGrade};

    fn agent(seed: u8) -> AgentKeys {
        AgentKeys::from_seed(&[seed; 32])
    }

    /// 测试用的双签合约见证：只暴露状态机需要的那三个事实。
    struct Witness {
        id: String,
        hash: String,
        dual: bool,
    }

    impl DualSigned for Witness {
        fn contract_id(&self) -> &str {
            &self.id
        }
        fn contract_hash(&self) -> &str {
            &self.hash
        }
        fn is_dual_signed(&self) -> bool {
            self.dual
        }
    }

    fn parties(a: &AgentKeys, b: &AgentKeys) -> Vec<Did> {
        vec![a.did(), b.did()]
    }

    #[test]
    fn legal_table_is_exactly_the_specification() {
        assert_eq!(LEGAL_TRANSITIONS.len(), 10);
        assert_eq!(Phase::ALL.len(), 7);
        assert_eq!(Event::ALL.len(), 9);
        assert_eq!(transition(Phase::Idle, Event::Request).unwrap(), Phase::Negotiating);
        assert_eq!(transition(Phase::Negotiating, Event::Accept).unwrap(), Phase::Accepted);
        assert_eq!(transition(Phase::Accepted, Event::Sign).unwrap(), Phase::ContractSigned);
        assert_eq!(transition(Phase::ContractSigned, Event::Execute).unwrap(), Phase::Executing);
        assert_eq!(transition(Phase::Executing, Event::Settle).unwrap(), Phase::Settled);
        assert_eq!(transition(Phase::Executing, Event::Breach).unwrap(), Phase::Arbitration);
        assert_eq!(transition(Phase::Arbitration, Event::Resolve).unwrap(), Phase::Settled);
    }

    #[test]
    fn every_illegal_combination_is_refused_exhaustively() {
        let mut legal = 0usize;
        let mut refused = 0usize;
        for phase in Phase::ALL {
            for event in Event::ALL {
                let expected = LEGAL_TRANSITIONS
                    .iter()
                    .find(|(f, e, _)| *f == phase && *e == event)
                    .map(|(_, _, t)| *t);
                match (transition(phase, event), expected) {
                    (Ok(got), Some(want)) => {
                        assert_eq!(got, want, "{phase:?}+{event:?}");
                        legal += 1;
                    }
                    (Err(CoreError::InvalidKind), None) => refused += 1,
                    (other, _) => panic!("{phase:?}+{event:?} 得到 {other:?}，与合法表不符"),
                }
            }
        }
        assert_eq!(legal, 10);
        assert_eq!(refused, 63 - 10);
    }

    #[test]
    fn settled_is_terminal_and_contract_phases_are_flagged() {
        assert!(Phase::Settled.is_terminal());
        for phase in Phase::ALL.iter().filter(|p| **p != Phase::Settled) {
            assert!(!phase.is_terminal());
        }
        for event in Event::ALL {
            assert_eq!(transition(Phase::Settled, event), Err(CoreError::InvalidKind));
        }
        assert!(!Phase::Idle.has_contract());
        assert!(Phase::ContractSigned.has_contract());
        assert!(Phase::Arbitration.has_contract());
    }

    #[test]
    fn the_happy_path_needs_both_signatures_at_every_step() {
        let a = agent(1);
        let b = agent(2);
        let parties = parties(&a, &b);
        let mut m = StateMachine::open("s-happy").unwrap();
        assert_eq!(m.phase(), Phase::Idle);

        let r1 = m.transact(Event::Request, &a, &b, 1, &parties).unwrap();
        assert_eq!(r1.from, Phase::Idle);
        assert_eq!(r1.to, Phase::Negotiating);
        assert_eq!(r1.sigs.len(), 2);
        assert_eq!(m.phase(), Phase::Negotiating);

        m.transact(Event::Counter, &b, &a, 2, &parties).unwrap();
        m.transact(Event::Accept, &a, &b, 3, &parties).unwrap();
        m.transact(Event::Sign, &a, &b, 4, &parties).unwrap();
        m.transact(Event::Execute, &b, &a, 5, &parties).unwrap();
        m.transact(Event::Settle, &a, &b, 6, &parties).unwrap();
        assert_eq!(m.phase(), Phase::Settled);
        assert_eq!(m.seq(), 6);
        assert_eq!(m.round(), 1);
        let tip = m.verify_history(&parties).unwrap();
        assert_eq!(tip, m.history().last().unwrap().record_hash().unwrap());

        // 签名顺序不影响记录字节（确定性来自排序，而不是运气）。
        let mut ordered = m.history()[0].clone();
        ordered.sigs.reverse();
        assert_eq!(ordered.record_hash().unwrap(), m.history()[0].record_hash().unwrap());
        ordered.sigs.reverse();
        assert_eq!(ordered, m.history()[0]);
    }

    #[test]
    fn a_single_signed_transition_is_never_committed() {
        let a = agent(3);
        let b = agent(4);
        let parties = parties(&a, &b);
        let mut m = StateMachine::open("s-single").unwrap();
        let staged = m.stage(Event::Request, &a, 1).unwrap();
        assert_eq!(staged.sigs.len(), 1);
        assert_eq!(staged.verify(), Err(CoreError::NotSealed));
        assert_eq!(
            m.commit(staged.clone(), &parties, None),
            Err(CoreError::NotSealed)
        );
        assert_eq!(m.phase(), Phase::Idle, "拒绝的转换不得改变相位");
        assert!(m.history().is_empty());

        let mut two = staged;
        StateMachine::co_sign(&mut two, &b).unwrap();
        assert_eq!(two.verify().unwrap(), ());
        m.commit(two, &parties, None).unwrap();
        assert_eq!(m.phase(), Phase::Negotiating);
    }

    #[test]
    fn third_party_duplicate_and_tampered_signatures_are_refused() {
        let a = agent(5);
        let b = agent(6);
        let c = agent(7);
        let parties = parties(&a, &b);
        let mut m = StateMachine::open("s-forge").unwrap();

        // 第三方联署：签名本身有效，但不是当事人。
        let mut with_outsider = m.stage(Event::Request, &a, 1).unwrap();
        StateMachine::co_sign(&mut with_outsider, &c).unwrap();
        assert_eq!(
            m.commit(with_outsider.clone(), &parties, None),
            Err(CoreError::InvalidSignature)
        );

        // 同一方重复签名。
        let mut twice = m.stage(Event::Request, &a, 1).unwrap();
        assert_eq!(twice.sign(&a), Err(CoreError::InvalidSignature));

        // 签名后篡改事件字段。
        let mut tampered = m.stage(Event::Request, &a, 1).unwrap();
        StateMachine::co_sign(&mut tampered, &b).unwrap();
        tampered.event = Event::Settle;
        assert_eq!(tampered.verify(), Err(CoreError::InvalidSignature));
        assert_eq!(m.commit(tampered, &parties, None), Err(CoreError::InvalidSignature));

        // 签名后篡改相位。
        let mut moved = m.stage(Event::Request, &a, 1).unwrap();
        StateMachine::co_sign(&mut moved, &b).unwrap();
        moved.to = Phase::Settled;
        assert_eq!(m.commit(moved, &parties, None), Err(CoreError::InvalidSignature));
    }

    #[test]
    fn reject_does_not_consume_round_quota_but_counter_does() {
        let a = agent(8);
        let b = agent(9);
        let parties = parties(&a, &b);
        let mut m = StateMachine::open("s-rounds").unwrap();
        m.transact(Event::Request, &a, &b, 1, &parties).unwrap();
        assert_eq!(m.round(), 0);
        for i in 0..3 {
            m.transact(Event::Reject, &b, &a, 10 + i, &parties).unwrap();
        }
        assert_eq!(m.round(), 0, "REJECT 不占轮数额度");
        assert_eq!(m.seq(), 4, "但每次拒绝仍是一条双方签署的记录");
        m.transact(Event::Counter, &a, &b, 20, &parties).unwrap();
        assert_eq!(m.round(), 1);
        m.transact(Event::Counter, &b, &a, 21, &parties).unwrap();
        assert_eq!(m.round(), 2);
        assert!(Event::Counter.consumes_round());
        assert!(!Event::Reject.consumes_round());
    }

    #[test]
    fn breach_requires_a_dual_signed_contract_witness() {
        let a = agent(10);
        let b = agent(11);
        let parties = parties(&a, &b);
        let mut m = StateMachine::open("s-breach").unwrap();
        m.transact(Event::Request, &a, &b, 1, &parties).unwrap();
        m.transact(Event::Accept, &b, &a, 2, &parties).unwrap();
        m.transact(Event::Sign, &a, &b, 3, &parties).unwrap();
        m.transact(Event::Execute, &b, &a, 4, &parties).unwrap();

        let good = Witness {
            id: "c-1".into(),
            hash: "a".repeat(64),
            dual: true,
        };
        let single = Witness {
            id: "c-1".into(),
            hash: "a".repeat(64),
            dual: false,
        };
        let other = Witness {
            id: "c-2".into(),
            hash: "b".repeat(64),
            dual: true,
        };

        let record = m
            .stage_under_contract(Event::Breach, &a, 5, "c-1", &"a".repeat(64))
            .unwrap();
        assert_eq!(record.sigs.len(), 1, "违约申诉只有申诉方当场签名");
        assert_eq!(
            m.commit(record.clone(), &parties, None),
            Err(CoreError::NotSealed),
            "没有合约见证不得进仲裁"
        );
        assert_eq!(
            m.commit(record.clone(), &parties, Some(&single)),
            Err(CoreError::InvalidSignature),
            "单方签署的合约不能授权"
        );
        assert_eq!(
            m.commit(record.clone(), &parties, Some(&other)),
            Err(CoreError::InvalidSignature),
            "哈希/编号对不上不能授权"
        );
        m.commit(record, &parties, Some(&good)).unwrap();
        assert_eq!(m.phase(), Phase::Arbitration);

        // 只有 Breach 能走条款授权。
        assert_eq!(
            m.stage_under_contract(Event::Resolve, &a, 6, "c-1", &"a".repeat(64)),
            Err(CoreError::InvalidKind)
        );
        m.transact(Event::Resolve, &b, &a, 6, &parties).unwrap();
        assert_eq!(m.phase(), Phase::Settled);
        m.verify_history(&parties).unwrap();
    }

    #[test]
    fn replay_detects_a_tampered_history_entry() {
        let a = agent(12);
        let b = agent(13);
        let parties = parties(&a, &b);
        let mut m = StateMachine::open("s-replay").unwrap();
        m.transact(Event::Request, &a, &b, 1, &parties).unwrap();
        m.transact(Event::Counter, &b, &a, 2, &parties).unwrap();
        m.transact(Event::Accept, &a, &b, 3, &parties).unwrap();
        let tip = m.verify_history(&parties).unwrap();

        let mut tampered = m.clone();
        tampered.history[1].at = 99;
        assert_eq!(
            tampered.verify_history(&parties),
            Err(CoreError::InvalidSignature),
            "改一个字节就该被历史重放抓住"
        );

        // 伪造一个「跳步」的历史：IDLE 直接 Sign。
        let mut jumped = m.clone();
        jumped.history.remove(1);
        jumped.round = 0;
        jumped.seq = 2;
        assert_eq!(jumped.verify_history(&parties), Err(CoreError::InvalidKind));

        // 原记录仍可复算同一个末端哈希。
        assert_eq!(m.verify_history(&parties).unwrap(), tip);
    }

    #[test]
    fn from_parts_refuses_components_that_disagree_with_the_history() {
        let a = agent(14);
        let b = agent(15);
        let parties = parties(&a, &b);
        let mut m = StateMachine::open("s-parts").unwrap();
        m.transact(Event::Request, &a, &b, 1, &parties).unwrap();
        m.transact(Event::Accept, &b, &a, 2, &parties).unwrap();

        let rebuilt = StateMachine::from_parts(
            "s-parts",
            Phase::Accepted,
            2,
            0,
            m.history().to_vec(),
        )
        .unwrap();
        assert_eq!(rebuilt.phase(), Phase::Accepted);

        assert_eq!(
            StateMachine::from_parts("s-parts", Phase::Settled, 2, 0, m.history().to_vec()),
            Err(CoreError::InvalidKind),
            "分量必须与历史一致，假装出来的相位不被接受"
        );
        assert_eq!(
            StateMachine::from_parts("s-parts", Phase::Accepted, 3, 0, m.history().to_vec()),
            Err(CoreError::InvalidKind)
        );
    }

    #[test]
    fn phases_and_events_roundtrip_through_strings() {
        for p in Phase::ALL {
            assert_eq!(Phase::parse(p.as_str()), Some(p));
        }
        assert_eq!(Phase::parse("nowhere"), None);
        for e in Event::ALL {
            assert!(!e.as_str().is_empty());
        }
        let t = Terms::new("summarize.zh", Credits(100), 5, EvidenceGrade::Verified).unwrap();
        assert_eq!(t.price, Credits(100));
    }
}
