//! v1.2.1 — 协商消息类型。
//!
//! 协商是**Agent 之间**的协议，所以每一种消息都是一枚先签名后发送的 PMB 信封：
//! 人类既不发起也不批准任何一条消息。
//!
//! 六种语义消息（[`kinds`]）覆盖协商全生命周期：
//!
//! | Rust 常量 | PMB 类型名 | 语义 |
//! |---|---|---|
//! | `NEGOTIATE_REQUEST` | `negotiate.request` | 首次报价 |
//! | `NEGOTIATE_COUNTER` | `negotiate.counter` | 还价（消耗轮数额度） |
//! | `NEGOTIATE_ACCEPT`  | `negotiate.accept`  | 接受对端条款 |
//! | `NEGOTIATE_REJECT`  | `negotiate.reject`  | 拒绝（**不**消耗轮数额度） |
//! | `CONTRACT_SIGN`     | `contract.sign`     | 合约签署 |
//! | `CONTRACT_BREACH`   | `contract.breach`   | 违约申诉 |
//!
//! 为什么常量名大写而线格式小写：`au4a_core::MsgKind` 在 v1.0.1 冻结时要求
//! 小写开头的受限标识符，PMB 类型名因此只能是 `negotiate.request`；
//! Rust 侧的常量名保持规格书里的大写拼写，两者一一对应且被测试断言。

use au4a_core::{
    canonical_hash, AgentKeys, CoreError, CoreResult, Credits, Did, Envelope, EvidenceGrade,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 六种协商消息的 PMB 类型名（线格式，必须满足 `MsgKind` 的字符约束）。
pub mod kinds {
    pub const NEGOTIATE_REQUEST: &str = "negotiate.request";
    pub const NEGOTIATE_COUNTER: &str = "negotiate.counter";
    pub const NEGOTIATE_ACCEPT: &str = "negotiate.accept";
    pub const NEGOTIATE_REJECT: &str = "negotiate.reject";
    pub const CONTRACT_SIGN: &str = "contract.sign";
    pub const CONTRACT_BREACH: &str = "contract.breach";
}

/// 六种类型名的固定顺序。测试、自检与文档共用同一份清单，避免三处各写一遍。
pub const ALL_KINDS: [&str; 6] = [
    kinds::NEGOTIATE_REQUEST,
    kinds::NEGOTIATE_COUNTER,
    kinds::NEGOTIATE_ACCEPT,
    kinds::NEGOTIATE_REJECT,
    kinds::CONTRACT_SIGN,
    kinds::CONTRACT_BREACH,
];

/// 单个标签（会话 id、任务名、合约 id）的最大字节数。
pub const MAX_LABEL: usize = 64;
/// 自由文本（拒绝理由、违约说明）的最大字节数。
pub const MAX_NOTE: usize = 200;
/// 单条消息里 `round` 字段的绝对上界（真正的轮数上限由 v1.2.4 的协商引擎按会话配置执行）。
pub const MAX_MESSAGE_ROUND: u32 = 64;

/// 是否是小写 hex 的 SHA-256 摘要。
pub fn is_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// 是否是合法标签：非空、≤64 字节、无空白与不可打印字符。
pub fn is_label(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_LABEL && s.bytes().all(|b| b.is_ascii_graphic())
}

/// 协商条款。金额一律整数微积分，绝不出现浮点。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Terms {
    /// 任务/能力标识，例如 `summarize.zh`。
    pub task: String,
    /// 报酬（微积分，必须为正）。
    pub price: Credits,
    /// 交付期限（逻辑时钟读数）。
    pub deadline: u64,
    /// 交付物自报的证据等级——结算闸门直接消费它。
    pub evidence: EvidenceGrade,
}

impl Terms {
    pub fn new(
        task: impl Into<String>,
        price: Credits,
        deadline: u64,
        evidence: EvidenceGrade,
    ) -> CoreResult<Self> {
        let terms = Self {
            task: task.into(),
            price,
            deadline,
            evidence,
        };
        terms.validate()?;
        Ok(terms)
    }

    pub fn validate(&self) -> CoreResult<()> {
        if !is_label(&self.task) {
            return Err(CoreError::InvalidKind);
        }
        if self.price.0 < 0 {
            return Err(CoreError::NegativeAmount);
        }
        if self.price.0 == 0 {
            return Err(CoreError::ZeroAmount);
        }
        Ok(())
    }

    /// 规范 JSON 投影。哈希与签名都建立在这串字节上，所以只能有一个来源。
    pub fn value(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn from_value(value: Value) -> CoreResult<Self> {
        let terms: Self = serde_json::from_value(value).map_err(|_| CoreError::Encoding)?;
        terms.validate()?;
        Ok(terms)
    }

    /// 条款内容哈希（规范 JSON 的 SHA-256）。接受方签的就是它。
    pub fn hash(&self) -> CoreResult<String> {
        canonical_hash(&self.value()?)
    }
}

/// 违约类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BreachKind {
    /// 完全没有交付。
    NonDelivery,
    /// 超过期限才交付。
    LateDelivery,
    /// 交付不完整。
    UnderDelivery,
    /// 交付物证据与声明等级不符。
    WrongEvidence,
}

impl BreachKind {
    pub const ALL: [BreachKind; 4] = [
        BreachKind::NonDelivery,
        BreachKind::LateDelivery,
        BreachKind::UnderDelivery,
        BreachKind::WrongEvidence,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            BreachKind::NonDelivery => "non_delivery",
            BreachKind::LateDelivery => "late_delivery",
            BreachKind::UnderDelivery => "under_delivery",
            BreachKind::WrongEvidence => "wrong_evidence",
        }
    }
}

/// 六种协商消息的语义体。带上 `type` 标签，因此消息体自身就是自描述的规范 JSON。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NegotiationMsg {
    /// 首次报价。
    Request { session: String, terms: Terms },
    /// 还价：消耗一轮额度的唯一事件。
    Counter {
        session: String,
        round: u32,
        terms: Terms,
    },
    /// 接受对端条款（按内容哈希认账，不按“看起来一样”）。
    Accept {
        session: String,
        round: u32,
        terms_hash: String,
    },
    /// 拒绝条款：不消耗轮数额度，也不会推进状态机。
    Reject {
        session: String,
        round: u32,
        reason: String,
    },
    /// 合约签署回执：签名覆盖合约的规范哈希。
    SignContract {
        contract_id: String,
        contract_hash: String,
    },
    /// 违约申诉：附带合约锚点，供 v1.2.7 仲裁接入消费。
    Breach {
        contract_id: String,
        contract_hash: String,
        kind: BreachKind,
        note: String,
    },
}

impl NegotiationMsg {
    /// 对应的 PMB 类型名。
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Request { .. } => kinds::NEGOTIATE_REQUEST,
            Self::Counter { .. } => kinds::NEGOTIATE_COUNTER,
            Self::Accept { .. } => kinds::NEGOTIATE_ACCEPT,
            Self::Reject { .. } => kinds::NEGOTIATE_REJECT,
            Self::SignContract { .. } => kinds::CONTRACT_SIGN,
            Self::Breach { .. } => kinds::CONTRACT_BREACH,
        }
    }

    pub fn session(&self) -> &str {
        match self {
            Self::Request { session, .. }
            | Self::Counter { session, .. }
            | Self::Accept { session, .. }
            | Self::Reject { session, .. } => session,
            Self::SignContract { contract_id, .. } | Self::Breach { contract_id, .. } => contract_id,
        }
    }

    /// 报价轮次。`Request` 是第 0 轮（开局不算还价）。
    pub fn round(&self) -> u32 {
        match self {
            Self::Request { .. } => 0,
            Self::Counter { round, .. }
            | Self::Accept { round, .. }
            | Self::Reject { round, .. } => *round,
            Self::SignContract { .. } | Self::Breach { .. } => 0,
        }
    }

    /// 接受方签的是条款哈希；违约申诉挂的是合约哈希。
    pub fn subject_hash(&self) -> Option<&str> {
        match self {
            Self::Accept { terms_hash, .. } => Some(terms_hash),
            Self::SignContract { contract_hash, .. } | Self::Breach { contract_hash, .. } => {
                Some(contract_hash)
            }
            Self::Request { .. } | Self::Counter { .. } | Self::Reject { .. } => None,
        }
    }

    /// 结构校验：任何一条不合规的消息都在进入状态机之前被拒。
    pub fn validate(&self) -> CoreResult<()> {
        let session_ok = is_label(self.session());
        if !session_ok {
            return Err(CoreError::InvalidKind);
        }
        if self.round() > MAX_MESSAGE_ROUND {
            return Err(CoreError::InvalidKind);
        }
        match self {
            Self::Request { terms, .. } | Self::Counter { terms, .. } => terms.validate()?,
            Self::Accept { terms_hash, .. } => {
                if !is_hash(terms_hash) {
                    return Err(CoreError::InvalidKind);
                }
            }
            Self::Reject { reason, .. } => {
                if reason.len() > MAX_NOTE {
                    return Err(CoreError::InvalidKind);
                }
            }
            Self::SignContract {
                contract_id,
                contract_hash,
            } => {
                if !is_label(contract_id) || !is_hash(contract_hash) {
                    return Err(CoreError::InvalidKind);
                }
            }
            Self::Breach {
                contract_id,
                contract_hash,
                note,
                ..
            } => {
                if !is_label(contract_id) || !is_hash(contract_hash) || note.len() > MAX_NOTE {
                    return Err(CoreError::InvalidKind);
                }
            }
        }
        Ok(())
    }

    /// 消息体的规范 JSON 投影（即信封的 `body`）。
    pub fn body(&self) -> CoreResult<Value> {
        self.validate()?;
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    /// 由 PMB 类型名 + 消息体还原语义消息，并断言两者一致。
    pub fn from_body(kind: &str, body: Value) -> CoreResult<Self> {
        let msg: Self = serde_json::from_value(body).map_err(|_| CoreError::Encoding)?;
        msg.validate()?;
        if msg.kind() != kind {
            // 信封说自己是 A、体说是 B：这是伪造或实现错误，必须拒绝而不是猜。
            return Err(CoreError::InvalidKind);
        }
        Ok(msg)
    }

    /// 解出信封面消息：验签 → 解析 → 校验。
    pub fn from_env(env: &Envelope) -> CoreResult<Self> {
        env.verify()?;
        Self::from_body(env.kind.as_str(), env.body.clone())
    }

    /// 用发送方私钥封一枚信封。这是本模块唯一的发送出口。
    pub fn signed(
        &self,
        from: &AgentKeys,
        to: &Did,
        ts: u64,
        in_reply_to: Option<String>,
    ) -> CoreResult<Envelope> {
        Envelope::new(
            from.did(),
            Some(to.clone()),
            self.kind(),
            ts,
            in_reply_to,
            self.body()?,
        )?
        .seal(from)
    }

    // ---- 六种消息的具名构造器（规格书里的名字，逐个可测） ----

    pub fn request(session: &str, terms: Terms) -> CoreResult<Self> {
        let msg = Self::Request {
            session: session.to_string(),
            terms,
        };
        msg.validate()?;
        Ok(msg)
    }

    pub fn counter(session: &str, round: u32, terms: Terms) -> CoreResult<Self> {
        let msg = Self::Counter {
            session: session.to_string(),
            round,
            terms,
        };
        msg.validate()?;
        Ok(msg)
    }

    pub fn accept(session: &str, round: u32, terms_hash: &str) -> CoreResult<Self> {
        let msg = Self::Accept {
            session: session.to_string(),
            round,
            terms_hash: terms_hash.to_string(),
        };
        msg.validate()?;
        Ok(msg)
    }

    pub fn reject(session: &str, round: u32, reason: &str) -> CoreResult<Self> {
        let msg = Self::Reject {
            session: session.to_string(),
            round,
            reason: reason.to_string(),
        };
        msg.validate()?;
        Ok(msg)
    }

    pub fn sign_contract(contract_id: &str, contract_hash: &str) -> CoreResult<Self> {
        let msg = Self::SignContract {
            contract_id: contract_id.to_string(),
            contract_hash: contract_hash.to_string(),
        };
        msg.validate()?;
        Ok(msg)
    }

    pub fn breach(
        contract_id: &str,
        contract_hash: &str,
        kind: BreachKind,
        note: &str,
    ) -> CoreResult<Self> {
        let msg = Self::Breach {
            contract_id: contract_id.to_string(),
            contract_hash: contract_hash.to_string(),
            kind,
            note: note.to_string(),
        };
        msg.validate()?;
        Ok(msg)
    }
}

/// 会话 id：对（提议方, 应答方, 首轮条款）做内容寻址。
///
/// 这样任何一方都无法用“另一个会话”偷换上下文：id 本身就是条款的承诺。
pub fn session_id(proposer: &Did, responder: &Did, terms: &Terms) -> CoreResult<String> {
    canonical_hash(&serde_json::json!({
        "proposer": proposer.as_str(),
        "responder": responder.as_str(),
        "terms": terms.value()?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(seed: u8) -> AgentKeys {
        AgentKeys::from_seed(&[seed; 32])
    }

    fn terms(price: i64) -> Terms {
        Terms::new("summarize.zh", Credits(price), 42, EvidenceGrade::Verified).unwrap()
    }

    #[test]
    fn six_kinds_are_valid_pmb_names_and_match_the_spec() {
        assert_eq!(ALL_KINDS.len(), 6);
        for k in ALL_KINDS {
            assert!(au4a_core::MsgKind::new(k).is_ok(), "{k}");
        }
        assert_eq!(kinds::NEGOTIATE_REQUEST, "negotiate.request");
        assert_eq!(kinds::CONTRACT_BREACH, "contract.breach");
    }

    #[test]
    fn every_kind_roundtrips_through_a_signed_envelope() {
        let a = agent(1);
        let b = agent(2);
        let t = terms(100);
        let h = t.hash().unwrap();
        let msgs = vec![
            NegotiationMsg::request("s1", t.clone()).unwrap(),
            NegotiationMsg::counter("s1", 1, terms(90)).unwrap(),
            NegotiationMsg::accept("s1", 1, &h).unwrap(),
            NegotiationMsg::reject("s1", 1, "too expensive").unwrap(),
            NegotiationMsg::sign_contract("c1", &h).unwrap(),
            NegotiationMsg::breach("c1", &h, BreachKind::NonDelivery, "nothing arrived").unwrap(),
        ];
        for msg in msgs {
            let env = msg.signed(&a, &b.did(), 7, None).unwrap();
            env.verify().unwrap();
            assert_eq!(NegotiationMsg::from_env(&env).unwrap(), msg);
            assert_eq!(env.kind.as_str(), msg.kind());
        }
    }

    #[test]
    fn a_body_that_disagrees_with_the_envelope_kind_is_refused() {
        let a = agent(3);
        let b = agent(4);
        let ok = NegotiationMsg::request("s1", terms(10)).unwrap();
        let lying = NegotiationMsg::counter("s1", 1, terms(9)).unwrap();
        let env = Envelope::new(
            a.did(),
            Some(b.did()),
            ok.kind(),
            1,
            None,
            lying.body().unwrap(),
        )
        .unwrap()
        .seal(&a)
        .unwrap();
        assert_eq!(NegotiationMsg::from_env(&env), Err(CoreError::InvalidKind));
    }

    #[test]
    fn tampered_bodies_and_unsealed_envelopes_are_refused() {
        let a = agent(5);
        let b = agent(6);
        let env = NegotiationMsg::counter("s1", 1, terms(50))
            .unwrap()
            .signed(&a, &b.did(), 3, None)
            .unwrap();
        let mut tampered = env.clone();
        tampered.body = NegotiationMsg::counter("s1", 1, terms(1)).unwrap().body().unwrap();
        assert_eq!(NegotiationMsg::from_env(&tampered), Err(CoreError::InvalidSignature));

        let unsealed = Envelope::new(
            a.did(),
            Some(b.did()),
            kinds::NEGOTIATE_REQUEST,
            1,
            None,
            NegotiationMsg::request("s1", terms(10)).unwrap().body().unwrap(),
        )
        .unwrap();
        assert_eq!(NegotiationMsg::from_env(&unsealed), Err(CoreError::NotSealed));
    }

    #[test]
    fn malformed_payloads_are_refused_without_panicking() {
        assert_eq!(
            Terms::new("", Credits(1), 0, EvidenceGrade::Verified),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(
            Terms::new("t", Credits(0), 0, EvidenceGrade::Verified),
            Err(CoreError::ZeroAmount)
        );
        assert_eq!(
            Terms::new("t", Credits(-1), 0, EvidenceGrade::Verified),
            Err(CoreError::NegativeAmount)
        );
        assert_eq!(
            Terms::new("has space", Credits(1), 0, EvidenceGrade::Verified),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(
            NegotiationMsg::accept("s1", 1, "not-a-hash"),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(
            NegotiationMsg::breach("c1", &"z".repeat(64), BreachKind::NonDelivery, "x"),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(
            NegotiationMsg::counter("s1", MAX_MESSAGE_ROUND + 1, terms(1)),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(
            NegotiationMsg::reject("s1", 1, &"x".repeat(MAX_NOTE + 1)),
            Err(CoreError::InvalidKind)
        );
    }

    #[test]
    fn terms_hash_is_canonical_and_price_sensitive() {
        let a = Terms::new("t", Credits(100), 5, EvidenceGrade::Verified).unwrap();
        let b = Terms::new("t", Credits(100), 5, EvidenceGrade::Verified).unwrap();
        let c = Terms::new("t", Credits(101), 5, EvidenceGrade::Verified).unwrap();
        assert_eq!(a.hash().unwrap(), b.hash().unwrap());
        assert_ne!(a.hash().unwrap(), c.hash().unwrap());
        assert!(is_hash(&a.hash().unwrap()));
        // 规范 JSON 里金额是整数，任何浮点都会被基元层拒绝。
        assert_eq!(
            au4a_core::canonicalize(&serde_json::json!({"price": 1.5})),
            Err(CoreError::FloatForbidden)
        );
    }

    #[test]
    fn session_id_binds_both_parties_and_the_opening_terms() {
        let d1 = agent(7).did();
        let d2 = agent(8).did();
        let d3 = agent(9).did();
        let t = terms(100);
        let s = session_id(&d1, &d2, &t).unwrap();
        assert_eq!(s, session_id(&d1, &d2, &t).unwrap());
        assert_ne!(s, session_id(&d2, &d1, &t).unwrap());
        assert_ne!(s, session_id(&d1, &d3, &t).unwrap());
        assert_ne!(s, session_id(&d1, &d2, &terms(101)).unwrap());
        assert!(is_label(&s));
    }

    #[test]
    fn body_is_self_describing_and_parses_back() {
        let m = NegotiationMsg::counter("s1", 2, terms(77)).unwrap();
        let body = m.body().unwrap();
        assert_eq!(body["type"], serde_json::json!("counter"));
        assert_eq!(body["round"], serde_json::json!(2));
        assert_eq!(NegotiationMsg::from_body(kinds::NEGOTIATE_COUNTER, body).unwrap(), m);
    }
}
