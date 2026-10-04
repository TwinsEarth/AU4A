//! v1.2.4 — 多轮协商引擎。
//!
//! 把 v1.2.1 的消息、v1.2.2 的状态机、v1.2.3 的归档串成两个 Agent 真实跑得起来的流程：
//!
//! ```text
//! open(报价 120) → counter(110) → counter(100) → reject(不占额度) → counter(95) → accept
//! ```
//!
//! 三条规则，全部可测：
//!
//! 1. **轮数上限**：只有 `Counter` 消耗额度；达到上限后再还价，向内核登记
//!    [`RefusalCode::PolicyDenied`] 类型化拒绝并返回 `Err(Overflow)`——不是 panic，也不是静默截断。
//! 2. **REJECT 不占额度**：拒绝可以无限次表达，它只记一条双方签署的转换记录，轮数不变。
//! 3. **轮转与认账**：还价必须由上一次报价的**对端**发出（禁止自问自答刷轮数）；
//!    接受者不得是当前报价的作者（不能自己接受自己）。
//!
//! 每次状态转换照旧**双方签名**：对端的签名是「这次转换确实发生过」的收据
//! （对争议中的条款不等于同意其内容——条款本身由报价方签名）。

use au4a_core::{CoreError, CoreResult, Did, RefusalCode};
use au4a_kernel::Kernel;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::contract::{Contract, ANCHOR_EVENT};
use crate::journal::Journal;
use crate::msg::{self, NegotiationMsg, Terms};
use crate::state::{Event, Phase, StateMachine, TransitionRecord};

/// 默认轮数上限：开局不计轮，最多六次还价。
pub const DEFAULT_MAX_ROUNDS: u32 = 6;

/// 一次报价（含开局报价）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    /// 0 表示开局报价，其余为还价轮次。
    pub round: u32,
    pub by: Did,
    pub terms: Terms,
    pub terms_hash: String,
    /// 承载这次报价的 PMB 信封 id（内容寻址）。
    pub message_id: String,
}

/// 一次拒绝。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rejection {
    pub round: u32,
    pub by: Did,
    pub reason: String,
    pub message_id: String,
}

/// 一次双人协商。
#[derive(Clone, Debug, PartialEq)]
pub struct Negotiation {
    session: String,
    parties: Vec<Did>,
    max_rounds: u32,
    /// 全部报价，按轮次升序；开局报价是第 0 条，因此永不为空。
    offers: Vec<Offer>,
    /// 当前有效条款（与 `offers` 的最后一条同步，单独持有避免在库代码里出现 `expect`）。
    current_terms: Terms,
    /// 当前报价的作者：轮转与「不能接受自己的报价」都靠它判断。
    last_offer_by: Did,
    rejections: Vec<Rejection>,
    /// 最后一条协商消息的 id，用作下一条的 `in_reply_to`。
    tip: Option<String>,
    /// 达成后签署的合约（v1.2.5）。
    contract: Option<Contract>,
    machine: StateMachine,
    journal: Journal,
}

impl Negotiation {
    /// 开局：提议方报价，相位 `IDLE → NEGOTIATING`。
    pub fn open(
        kernel: &mut Kernel,
        proposer: &au4a_core::AgentKeys,
        responder: &au4a_core::AgentKeys,
        terms: Terms,
        max_rounds: u32,
    ) -> CoreResult<Self> {
        if max_rounds == 0 {
            // 零轮上限等于不允许还价，那是「固定价」而不是协商；明确拒绝而不是猜。
            return Err(CoreError::InvalidKind);
        }
        let session = msg::session_id(&proposer.did(), &responder.did(), &terms)?;
        let parties = vec![proposer.did(), responder.did()];
        let mut journal = Journal::open(&session, &parties)?;
        let mut machine = StateMachine::open(&session)?;

        let at = kernel.tick();
        let env = NegotiationMsg::request(&session, terms.clone())?.signed(
            proposer,
            &responder.did(),
            at,
            None,
        )?;
        kernel.send(&env)?;
        journal.append(&env)?;
        let record = machine.transact(Event::Request, proposer, responder, at, &parties)?;
        journal.append_transition(&record, None)?;

        Ok(Self {
            session,
            parties,
            max_rounds,
            offers: vec![Offer {
                round: 0,
                by: proposer.did(),
                terms_hash: terms.hash()?,
                terms: terms.clone(),
                message_id: env.id.clone(),
            }],
            current_terms: terms,
            last_offer_by: proposer.did(),
            rejections: Vec::new(),
            tip: Some(env.id),
            contract: None,
            machine,
            journal,
        })
    }

    pub fn session(&self) -> &str {
        self.session.as_str()
    }

    pub fn parties(&self) -> &[Did] {
        &self.parties
    }

    pub fn phase(&self) -> Phase {
        self.machine.phase()
    }

    /// 已消耗的轮数额度（`REJECT` 不计入）。
    pub fn rounds_used(&self) -> u32 {
        self.machine.round()
    }

    pub fn max_rounds(&self) -> u32 {
        self.max_rounds
    }

    pub fn offers(&self) -> &[Offer] {
        &self.offers
    }

    pub fn rejections(&self) -> &[Rejection] {
        &self.rejections
    }

    /// 当前有效条款（最后一条报价）。
    pub fn current_terms(&self) -> &Terms {
        &self.current_terms
    }

    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    pub fn machine(&self) -> &StateMachine {
        &self.machine
    }

    /// 价格轨迹（微积分整数），供证据与观察层展示。
    pub fn price_trail(&self) -> Vec<i64> {
        self.offers.iter().map(|o| o.terms.price.0).collect()
    }

    /// 还价：消耗一轮额度。超限或轮次违规都走类型化拒绝。
    pub fn counter(
        &mut self,
        kernel: &mut Kernel,
        by: &au4a_core::AgentKeys,
        to: &au4a_core::AgentKeys,
        terms: Terms,
    ) -> CoreResult<Offer> {
        self.ensure_party(&by.did())?;
        self.ensure_party(&to.did())?;
        if self.phase() != Phase::Negotiating {
            return Err(CoreError::InvalidKind);
        }
        if self.last_offer_by == by.did() {
            // 自问自答会把轮数刷成噪音，也绕过了「还价」的语义。
            kernel.refuse(&by.did(), RefusalCode::Conflict, "counter by the author of the last offer");
            return Err(CoreError::InvalidKind);
        }
        if self.rounds_used() >= self.max_rounds {
            // 额度耗尽：类型化拒绝（PolicyDenied）+ 类型化错误（Overflow），不 panic、不截断。
            kernel.refuse(
                &by.did(),
                RefusalCode::PolicyDenied,
                format!("round quota exhausted (max_rounds={})", self.max_rounds),
            );
            return Err(CoreError::Overflow);
        }

        let round = self.rounds_used() + 1;
        let terms_hash = terms.hash()?;
        let at = kernel.tick();
        let env = NegotiationMsg::counter(&self.session, round, terms.clone())?.signed(
            by,
            &to.did(),
            at,
            self.tip.clone(),
        )?;
        kernel.send(&env)?;
        self.journal.append(&env)?;
        let record = self
            .machine
            .transact(Event::Counter, by, to, at, &self.parties)?;
        self.journal.append_transition(&record, None)?;

        let offer = Offer {
            round,
            by: by.did(),
            terms_hash,
            terms: terms.clone(),
            message_id: env.id.clone(),
        };
        self.offers.push(offer.clone());
        self.current_terms = terms;
        self.last_offer_by = by.did();
        self.tip = Some(env.id);
        Ok(offer)
    }

    /// 接受当前条款：`NEGOTIATING → ACCEPTED`。
    pub fn accept(
        &mut self,
        kernel: &mut Kernel,
        by: &au4a_core::AgentKeys,
        to: &au4a_core::AgentKeys,
    ) -> CoreResult<TransitionRecord> {
        self.ensure_party(&by.did())?;
        self.ensure_party(&to.did())?;
        if self.phase() != Phase::Negotiating {
            return Err(CoreError::InvalidKind);
        }
        if self.last_offer_by == by.did() {
            kernel.refuse(&by.did(), RefusalCode::Conflict, "cannot accept your own offer");
            return Err(CoreError::InvalidKind);
        }
        let terms_hash = self.current_terms().hash()?;
        let at = kernel.tick();
        let env = NegotiationMsg::accept(&self.session, self.rounds_used(), &terms_hash)?.signed(
            by,
            &to.did(),
            at,
            self.tip.clone(),
        )?;
        kernel.send(&env)?;
        self.journal.append(&env)?;
        let record = self.machine.transact(Event::Accept, by, to, at, &self.parties)?;
        self.journal.append_transition(&record, None)?;
        self.tip = Some(env.id);
        Ok(record)
    }

    /// 拒绝当前条款：`NEGOTIATING → NEGOTIATING` 自环，**不消耗轮数额度**。
    pub fn reject(
        &mut self,
        kernel: &mut Kernel,
        by: &au4a_core::AgentKeys,
        to: &au4a_core::AgentKeys,
        reason: &str,
    ) -> CoreResult<TransitionRecord> {
        self.ensure_party(&by.did())?;
        self.ensure_party(&to.did())?;
        if self.phase() != Phase::Negotiating {
            return Err(CoreError::InvalidKind);
        }
        let at = kernel.tick();
        let env = NegotiationMsg::reject(&self.session, self.rounds_used(), reason)?.signed(
            by,
            &to.did(),
            at,
            self.tip.clone(),
        )?;
        kernel.send(&env)?;
        self.journal.append(&env)?;
        let record = self.machine.transact(Event::Reject, by, to, at, &self.parties)?;
        self.journal.append_transition(&record, None)?;
        self.rejections.push(Rejection {
            round: self.rounds_used(),
            by: by.did(),
            reason: reason.to_string(),
            message_id: env.id.clone(),
        });
        self.tip = Some(env.id);
        Ok(record)
    }

    /// 归档字节（规范 JSON）。协商结束后可由任意一方带走并重放。
    pub fn archive(&self) -> CoreResult<String> {
        self.journal.encode()
    }

    /// 已签署的合约（若有）。
    pub fn contract(&self) -> Option<&Contract> {
        self.contract.as_ref()
    }

    /// 签订合约：`ACCEPTED → CONTRACT_SIGNED`。
    ///
    /// 步骤全部真实发生：双方各自签署同一份条款载荷 → 各自发一条 `CONTRACT_SIGN` 消息（可被任何
    /// 观察者验签）→ 双方联署状态机转换记录 → 由已签署方锚定合约哈希到只读进度流。
    pub fn sign_contract(
        &mut self,
        kernel: &mut Kernel,
        proposer: &au4a_core::AgentKeys,
        responder: &au4a_core::AgentKeys,
    ) -> CoreResult<Contract> {
        if self.phase() != Phase::Accepted {
            return Err(CoreError::InvalidKind);
        }
        self.ensure_party(&proposer.did())?;
        self.ensure_party(&responder.did())?;
        if proposer.did() == responder.did() {
            return Err(CoreError::InvalidKind);
        }

        let at = kernel.tick();
        let mut contract = Contract::draft(
            proposer,
            &responder.did(),
            &self.current_terms,
            &self.session,
            at,
        )?;
        contract.sign(proposer)?;
        contract.sign(responder)?;
        contract.verify()?;

        // 双方各发一条 CONTRACT_SIGN：合约的承认是**两条**可独立验签的声明，而不是一条。
        for (keys, peer) in [(proposer, responder), (responder, proposer)] {
            let env = NegotiationMsg::sign_contract(&contract.id, &contract.hash)?.signed(
                keys,
                &peer.did(),
                kernel.tick(),
                self.tip.clone(),
            )?;
            kernel.send(&env)?;
            self.journal.append(&env)?;
            self.tip = Some(env.id);
        }

        let record = self.machine.transact(Event::Sign, proposer, responder, at, &self.parties)?;
        self.journal.append_transition(&record, None)?;

        let anchor = contract.anchor(proposer, kernel.tick())?;
        kernel.emit(
            ANCHOR_EVENT,
            format!(
                "{} hash={} price={}",
                anchor.contract_id, anchor.contract_hash, contract.terms.price.0
            ),
        );
        self.contract = Some(contract.clone());
        Ok(contract)
    }

    /// 协商摘要（观察层与 scenario 共用）。
    pub fn summary(&self) -> CoreResult<Value> {
        Ok(json!({
            "session": self.session,
            "phase": self.phase().as_str(),
            "rounds_used": self.rounds_used(),
            "max_rounds": self.max_rounds,
            "offers": self.offers.len(),
            "rejections": self.rejections.len(),
            "price_trail": self.price_trail(),
            "journal_bytes": self.archive()?.len(),
            "history_tip": self.machine.verify_history(&self.parties)?,
            "contract": self.contract.as_ref().map(|c| c.summary()),
        }))
    }

    fn ensure_party(&self, did: &Did) -> CoreResult<()> {
        if self.parties.contains(did) {
            Ok(())
        } else {
            Err(CoreError::UnknownAgent)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use au4a_core::{AgentKeys, Credits, EvidenceGrade};
    use au4a_kernel::KernelConfig;

    fn kernel() -> Kernel {
        Kernel::new(KernelConfig::default())
    }

    fn agent(seed: u8) -> AgentKeys {
        AgentKeys::from_seed(&[seed; 32])
    }

    fn terms(price: i64) -> Terms {
        Terms::new("summarize.zh", Credits(price), 40, EvidenceGrade::Verified).unwrap()
    }

    fn pair(k: &mut Kernel, s1: u8, s2: u8) -> (AgentKeys, AgentKeys) {
        let a = agent(s1);
        let b = agent(s2);
        k.register(&a, "a", &["summarize.zh"], Credits(20)).unwrap();
        k.register(&b, "b", &["summarize.zh"], Credits(20)).unwrap();
        (a, b)
    }

    #[test]
    fn multi_round_bargaining_reaches_acceptance() {
        let mut k = kernel();
        let (a, b) = pair(&mut k, 1, 2);
        let mut n = Negotiation::open(&mut k, &a, &b, terms(120), DEFAULT_MAX_ROUNDS).unwrap();
        assert_eq!(n.phase(), Phase::Negotiating);
        assert_eq!(n.rounds_used(), 0);

        n.counter(&mut k, &b, &a, terms(110)).unwrap();
        n.counter(&mut k, &a, &b, terms(100)).unwrap();
        n.reject(&mut k, &b, &a, "still too high").unwrap();
        n.counter(&mut k, &b, &a, terms(98)).unwrap();
        let record = n.accept(&mut k, &a, &b).unwrap();

        assert_eq!(n.phase(), Phase::Accepted);
        assert_eq!(n.rounds_used(), 3, "三次还价消耗三轮，REJECT 不占");
        assert_eq!(n.price_trail(), vec![120, 110, 100, 98]);
        assert_eq!(n.rejections().len(), 1);
        assert_eq!(n.offers().len(), 4);
        assert_eq!(record.sigs.len(), 2);
        assert_eq!(n.current_terms().price, Credits(98));

        // 每条历史记录都是双方签名。
        for rec in n.machine().history() {
            assert_eq!(rec.sigs.len(), 2, "{} 必须双签", rec.event.as_str());
        }
        // 归档可逐字节重放。
        let bytes = n.archive().unwrap();
        let restored = Journal::decode(&bytes).unwrap();
        assert_eq!(restored.encode().unwrap(), bytes);
        assert_eq!(restored.replay_digest().unwrap(), n.journal().replay_digest().unwrap());
        n.summary().unwrap();
    }

    #[test]
    fn the_round_cap_is_a_typed_refusal_not_a_panic() {
        let mut k = kernel();
        let (a, b) = pair(&mut k, 3, 4);
        let mut n = Negotiation::open(&mut k, &a, &b, terms(200), 2).unwrap();
        n.counter(&mut k, &b, &a, terms(180)).unwrap();
        n.counter(&mut k, &a, &b, terms(160)).unwrap();
        assert_eq!(n.rounds_used(), 2);

        let before = n.phase();
        let refusals_before = k.refusals().len();
        assert_eq!(
            n.counter(&mut k, &b, &a, terms(140)),
            Err(CoreError::Overflow)
        );
        assert_eq!(n.rounds_used(), 2, "被拒的还价不得计入");
        assert_eq!(n.phase(), before);
        assert_eq!(n.offers().len(), 3, "被拒的还价不得进入报价历史");
        assert_eq!(k.refusals().len(), refusals_before + 1);
        assert_eq!(k.refusals().last().unwrap().1.code, RefusalCode::PolicyDenied);
        assert!(!k.refusals().last().unwrap().1.code.is_misconduct());

        // 额度耗尽不影响接受／拒绝：协商仍可正常收尾。
        n.accept(&mut k, &b, &a).unwrap();
        assert_eq!(n.phase(), Phase::Accepted);
    }

    #[test]
    fn rejections_never_consume_quota_even_in_bulk() {
        let mut k = kernel();
        let (a, b) = pair(&mut k, 5, 6);
        let mut n = Negotiation::open(&mut k, &a, &b, terms(100), 1).unwrap();
        for i in 0..8 {
            let by = if i % 2 == 0 { &b } else { &a };
            let to = if i % 2 == 0 { &a } else { &b };
            n.reject(&mut k, by, to, "no").unwrap();
        }
        assert_eq!(n.rounds_used(), 0);
        assert_eq!(n.rejections().len(), 8);
        assert_eq!(n.phase(), Phase::Negotiating);
        // 上限 1 轮仍然可用。
        n.counter(&mut k, &b, &a, terms(90)).unwrap();
        assert_eq!(n.rounds_used(), 1);
        assert_eq!(n.counter(&mut k, &a, &b, terms(80)), Err(CoreError::Overflow));
    }

    #[test]
    fn turn_taking_and_self_acceptance_are_refused() {
        let mut k = kernel();
        let (a, b) = pair(&mut k, 7, 8);
        let mut n = Negotiation::open(&mut k, &a, &b, terms(100), 4).unwrap();
        // 报价方不能自己还价。
        assert_eq!(n.counter(&mut k, &a, &b, terms(90)), Err(CoreError::InvalidKind));
        assert_eq!(k.refusals().last().unwrap().1.code, RefusalCode::Conflict);
        // 报价方不能接受自己的报价。
        assert_eq!(n.accept(&mut k, &a, &b), Err(CoreError::InvalidKind));
        assert_eq!(k.refusals().last().unwrap().1.code, RefusalCode::Conflict);
        assert_eq!(n.phase(), Phase::Negotiating);
        assert_eq!(n.rounds_used(), 0);
        // 对端可以还价，然后报价方可以接受。
        n.counter(&mut k, &b, &a, terms(95)).unwrap();
        n.accept(&mut k, &a, &b).unwrap();
        assert_eq!(n.phase(), Phase::Accepted);
    }

    #[test]
    fn outsiders_are_refused_and_invalid_open_is_refused() {
        let mut k = kernel();
        let (a, b) = pair(&mut k, 9, 10);
        let outsider = agent(11);
        k.register(&outsider, "outsider", &[], Credits(20)).unwrap();
        assert_eq!(
            Negotiation::open(&mut k, &a, &b, terms(100), 0),
            Err(CoreError::InvalidKind)
        );
        let mut n = Negotiation::open(&mut k, &a, &b, terms(100), 4).unwrap();
        assert_eq!(
            n.counter(&mut k, &outsider, &a, terms(1)),
            Err(CoreError::UnknownAgent)
        );
        assert_eq!(n.reject(&mut k, &outsider, &a, "x"), Err(CoreError::UnknownAgent));
        assert_eq!(n.accept(&mut k, &outsider, &a), Err(CoreError::UnknownAgent));
        assert_eq!(n.offers().len(), 1);
    }

    #[test]
    fn terminals_refuse_further_bargaining() {
        let mut k = kernel();
        let (a, b) = pair(&mut k, 12, 13);
        let mut n = Negotiation::open(&mut k, &a, &b, terms(100), 4).unwrap();
        n.accept(&mut k, &b, &a).unwrap();
        assert_eq!(n.counter(&mut k, &a, &b, terms(90)), Err(CoreError::InvalidKind));
        assert_eq!(n.reject(&mut k, &a, &b, "no"), Err(CoreError::InvalidKind));
        assert_eq!(n.accept(&mut k, &a, &b), Err(CoreError::InvalidKind));
    }

    #[test]
    fn contract_signing_requires_acceptance_and_stays_dual_signed() {
        let mut k = kernel();
        let (a, b) = pair(&mut k, 30, 31);
        let mut n = Negotiation::open(&mut k, &a, &b, terms(100), 4).unwrap();
        // 还没接受就不能签。
        assert_eq!(n.sign_contract(&mut k, &a, &b), Err(CoreError::InvalidKind));
        assert!(n.contract().is_none());

        n.accept(&mut k, &b, &a).unwrap();
        let contract = n.sign_contract(&mut k, &a, &b).unwrap();
        assert_eq!(n.phase(), Phase::ContractSigned);
        contract.verify().unwrap();
        assert!(contract.is_dual_signed());
        assert_eq!(contract.terms.price, Credits(100));
        assert_eq!(contract.negotiation, n.session());
        contract.verify_anchor().unwrap();
        assert_eq!(n.contract().unwrap().hash, contract.hash);

        // 双方各发了一条 CONTRACT_SIGN；锚点事件进了只读进度流。
        let delivered = k.drain();
        assert_eq!(delivered.len(), 4, "request + accept + 两条 CONTRACT_SIGN");
        let sign_messages = delivered
            .iter()
            .filter(|e| e.kind.as_str() == crate::kinds::CONTRACT_SIGN)
            .count();
        assert_eq!(sign_messages, 2);
        assert!(k
            .observe()
            .progress
            .iter()
            .any(|p| p.kind == ANCHOR_EVENT && p.detail.contains(&contract.hash)));

        // 合约签完就不再是 ACCEPTED：不能重复签。
        assert_eq!(n.sign_contract(&mut k, &a, &b), Err(CoreError::InvalidKind));
    }

    #[test]
    fn the_same_seed_gives_the_same_archive() {
        let mut k1 = kernel();
        let (a1, b1) = pair(&mut k1, 20, 21);
        let mut n1 = Negotiation::open(&mut k1, &a1, &b1, terms(120), 4).unwrap();
        n1.counter(&mut k1, &b1, &a1, terms(100)).unwrap();
        n1.accept(&mut k1, &a1, &b1).unwrap();

        let mut k2 = kernel();
        let (a2, b2) = pair(&mut k2, 20, 21);
        let mut n2 = Negotiation::open(&mut k2, &a2, &b2, terms(120), 4).unwrap();
        n2.counter(&mut k2, &b2, &a2, terms(100)).unwrap();
        n2.accept(&mut k2, &a2, &b2).unwrap();

        assert_eq!(n1.archive().unwrap(), n2.archive().unwrap(), "同种子必须同归档");
        assert_eq!(n1.summary().unwrap(), n2.summary().unwrap());
    }
}
