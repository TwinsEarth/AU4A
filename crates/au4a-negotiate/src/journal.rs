//! v1.2.3 — 协商记录持久化：**纯内存快照 + 规范 JSON**，没有文件 I/O。
//!
//! 为什么不落盘：轨道必须能被确定性重放。文件系统不是确定性的输入——
//! 字节序、换行、编码、时钟都会渗进来。所以「持久化」在这里的准确含义是
//! **把会话状态编码成一段可携带、可校验、可逐字节比对的规范 JSON 文本**，
//! 由调用方决定把它放到哪里（内存、消息、外部存储都行）。
//!
//! 三条不变式：
//!
//! 1. **唯一事实来源**：归档里只存「已签名信封」与「双方签署的转换记录」；
//!    相位、序号、轮数全部由重放推导——推导出来的状态不可能和归档打架。
//! 2. **重放即重新验签**：`decode` 与 `replay` 都会重新验签、重新走相位链，
//!    不是「信字节」而是「验字节」。
//! 3. **编码定点**：`encode(decode(encode(j))) == encode(j)`，逐字节相同。

use au4a_core::{canonical_hash, canonicalize, CoreError, CoreResult, Did, Envelope};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::msg::NegotiationMsg;
use crate::state::{DualSigned, Phase, StateMachine, TransitionRecord};

/// 归档格式版本。格式变更必须抬版本，旧版本会被 `decode` 明确拒绝而不是猜。
pub const JOURNAL_VERSION: u32 = 1;

/// 归档数据（纯值，可序列化）。
///
/// 只有 `PartialEq`：`Envelope` 内含 `serde_json::Value`，而基元层允许 `Value` 承载浮点
/// （规范 JSON 编码时才拒绝浮点），所以这里不能声称 `Eq`。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u32,
    session: String,
    parties: Vec<Did>,
    messages: Vec<Envelope>,
    transitions: Vec<TransitionRecord>,
}

/// 一次协商的完整归档。
#[derive(Clone, Debug, PartialEq)]
pub struct Journal {
    session: String,
    parties: Vec<Did>,
    messages: Vec<Envelope>,
    transitions: Vec<TransitionRecord>,
    /// 由 `transitions` 推导，不独立存储——避免「两份状态互相打架」。
    machine: StateMachine,
}

impl Journal {
    /// 开一份空归档。双方必须是两个不同的合法 DID。
    pub fn open(session: &str, parties: &[Did]) -> CoreResult<Self> {
        if !crate::msg::is_label(session) || parties.len() != 2 || parties[0] == parties[1] {
            return Err(CoreError::InvalidKind);
        }
        for party in parties {
            if Did::parse(party.as_str())? != *party {
                return Err(CoreError::InvalidDid);
            }
        }
        Ok(Self {
            session: session.to_string(),
            parties: parties.to_vec(),
            messages: Vec::new(),
            transitions: Vec::new(),
            machine: StateMachine::open(session)?,
        })
    }

    pub fn session(&self) -> &str {
        self.session.as_str()
    }

    pub fn parties(&self) -> &[Did] {
        &self.parties
    }

    pub fn messages(&self) -> &[Envelope] {
        &self.messages
    }

    pub fn transitions(&self) -> &[TransitionRecord] {
        &self.transitions
    }

    /// 由归档推导出来的状态机（只读）。
    pub fn machine(&self) -> &StateMachine {
        &self.machine
    }

    pub fn phase(&self) -> Phase {
        self.machine.phase()
    }

    /// 归档一条协商消息：必须验签通过、属于本会话、且不曾归档过。
    pub fn append(&mut self, env: &Envelope) -> CoreResult<()> {
        let msg = NegotiationMsg::from_env(env)?;
        if let Some(session) = msg.negotiate_session() {
            if session != self.session {
                return Err(CoreError::InvalidKind);
            }
        }
        if self.messages.iter().any(|m| m.id == env.id) {
            // 内容寻址 id 相同就是同一条消息；重复归档只会让「记录」与事实脱钩。
            return Err(CoreError::InvalidKind);
        }
        self.messages.push(env.clone());
        Ok(())
    }

    /// 归档一条转换记录：与实时路径同一把尺子——双方签名齐备，条款授权需合约见证。
    pub fn append_transition(
        &mut self,
        record: &TransitionRecord,
        witness: Option<&dyn DualSigned>,
    ) -> CoreResult<()> {
        self.machine
            .commit(record.clone(), &self.parties, witness)?;
        self.transitions.push(record.clone());
        Ok(())
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            version: JOURNAL_VERSION,
            session: self.session.clone(),
            parties: self.parties.clone(),
            messages: self.messages.clone(),
            transitions: self.transitions.clone(),
        }
    }

    /// 编码为规范 JSON 文本（AU4A-CJ：键按字节序、无空白、禁浮点）。
    pub fn encode(&self) -> CoreResult<String> {
        let value = serde_json::to_value(self.snapshot()).map_err(|_| CoreError::Encoding)?;
        canonicalize(&value)
    }

    /// 从规范 JSON 文本恢复：版本、DID、每条信封、整条相位链全部重新校验。
    pub fn decode(text: &str) -> CoreResult<Self> {
        let value: Value = serde_json::from_str(text).map_err(|_| CoreError::Encoding)?;
        let snap: Snapshot = serde_json::from_value(value).map_err(|_| CoreError::Encoding)?;
        if snap.version != JOURNAL_VERSION {
            return Err(CoreError::InvalidVersion);
        }
        let mut journal = Self::open(&snap.session, &snap.parties)?;
        for env in &snap.messages {
            journal.append(env)?;
        }
        let machine = StateMachine::rebuild(&snap.session, &snap.parties, &snap.transitions)?;
        journal.transitions = snap.transitions;
        journal.machine = machine;
        Ok(journal)
    }

    /// 重放：不信任归档里的推导状态，从记录重新走一遍。
    pub fn replay(&self) -> CoreResult<StateMachine> {
        StateMachine::rebuild(&self.session, &self.parties, &self.transitions)
    }

    /// 重放摘要：把重放出来的状态与归档消息压成一个内容哈希，用于逐字节一致性比对。
    pub fn replay_digest(&self) -> CoreResult<String> {
        let replayed = self.replay()?;
        let mut messages: Vec<Value> = Vec::new();
        for env in &self.messages {
            messages.push(json!({"kind": env.kind.as_str(), "id": env.id}));
        }
        canonical_hash(&json!({
            "session": replayed.session(),
            "phase": replayed.phase(),
            "seq": replayed.seq(),
            "round": replayed.round(),
            "tip": replayed.verify_history(&self.parties)?,
            "messages": messages,
        }))
    }

    /// 归档统计（观察层「结果」面板用）。
    pub fn stats(&self) -> Value {
        json!({
            "session": self.session,
            "phase": self.machine.phase().as_str(),
            "messages": self.messages.len(),
            "transitions": self.transitions.len(),
            "rounds_used": self.machine.round(),
            "bytes": self.encode().map(|s| s.len()).unwrap_or(0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg::Terms;
    use crate::state::Event;
    use au4a_core::{AgentKeys, Credits, EvidenceGrade};

    fn agent(seed: u8) -> AgentKeys {
        AgentKeys::from_seed(&[seed; 32])
    }

    fn terms(price: i64) -> Terms {
        Terms::new("summarize.zh", Credits(price), 40, EvidenceGrade::Verified).unwrap()
    }

    /// 走一遍「报价 → 还价」，返回归档 + 双方密钥可复用的材料。
    fn sample() -> (Journal, AgentKeys, AgentKeys) {
        let a = agent(1);
        let b = agent(2);
        let parties = vec![a.did(), b.did()];
        let session = crate::msg::session_id(&a.did(), &b.did(), &terms(120)).unwrap();
        let mut journal = Journal::open(&session, &parties).unwrap();

        let request_env = NegotiationMsg::request(&session, terms(120))
            .unwrap()
            .signed(&a, &b.did(), 1, None)
            .unwrap();
        journal.append(&request_env).unwrap();
        let counter_env = NegotiationMsg::counter(&session, 1, terms(100))
            .unwrap()
            .signed(&b, &a.did(), 2, Some(request_env.id.clone()))
            .unwrap();
        journal.append(&counter_env).unwrap();

        let mut machine = StateMachine::open(&session).unwrap();
        let request_record = machine
            .transact(Event::Request, &a, &b, 1, &parties)
            .unwrap();
        journal.append_transition(&request_record, None).unwrap();
        let counter_record = machine
            .transact(Event::Counter, &b, &a, 2, &parties)
            .unwrap();
        journal.append_transition(&counter_record, None).unwrap();

        (journal, a, b)
    }

    #[test]
    fn encode_decode_encode_is_a_byte_exact_fixpoint() {
        let (journal, _, _) = sample();
        let bytes = journal.encode().unwrap();
        let restored = Journal::decode(&bytes).unwrap();
        assert_eq!(restored, journal);
        assert_eq!(restored.encode().unwrap(), bytes, "重放后必须逐字节一致");
        assert_eq!(
            restored.replay_digest().unwrap(),
            journal.replay_digest().unwrap()
        );
        assert_eq!(restored.machine(), journal.machine());
    }

    #[test]
    fn the_derived_state_comes_from_replay_not_from_the_bytes() {
        let (journal, _, _) = sample();
        let replayed = journal.replay().unwrap();
        assert_eq!(replayed.phase(), Phase::Negotiating);
        assert_eq!(replayed.seq(), 2);
        assert_eq!(replayed.round(), 1);
        assert_eq!(journal.phase(), replayed.phase());
        // 归档里没有「phase」字段可篡改：改相位只能改记录，而记录有签名。
        assert!(!journal.encode().unwrap().contains("\"phase\""));
    }

    #[test]
    fn malformed_or_mutated_archives_are_refused() {
        let (journal, _, _) = sample();
        let bytes = journal.encode().unwrap();

        assert_eq!(Journal::decode(""), Err(CoreError::Encoding));
        assert_eq!(Journal::decode("{"), Err(CoreError::Encoding));
        assert_eq!(
            Journal::decode(&bytes[..bytes.len() / 2]),
            Err(CoreError::Encoding)
        );

        // 版本不符：明确拒绝，不猜。
        let bumped = bytes.replace("\"version\":1", "\"version\":2");
        assert_eq!(Journal::decode(&bumped), Err(CoreError::InvalidVersion));

        // 多一个字段：拒绝（deny_unknown_fields）。
        let extra = bytes.replace("\"version\":1", "\"version\":1,\"extra\":true");
        assert_eq!(Journal::decode(&extra), Err(CoreError::Encoding));

        // 篡改信封体：验签抓住。
        let tampered = bytes.replace("\"price\":100", "\"price\":1");
        assert_ne!(tampered, bytes, "样本里应当存在可替换的价格字段");
        assert_eq!(Journal::decode(&tampered), Err(CoreError::InvalidSignature));

        // 篡改签名。
        let mut env = journal.messages()[0].clone();
        env.sig = "00".repeat(64);
        let mut forged = journal.clone();
        forged.messages[0] = env;
        let forged_bytes = serde_json::to_string(&forged.snapshot()).unwrap();
        assert_eq!(
            Journal::decode(&forged_bytes),
            Err(CoreError::InvalidSignature)
        );
    }

    #[test]
    fn out_of_order_or_foreign_records_are_refused() {
        let (journal, a, b) = sample();

        // 调换记录顺序：相位链断掉。
        let mut reordered = journal.clone();
        reordered.transitions.swap(0, 1);
        let text = serde_json::to_string(&reordered.snapshot()).unwrap();
        assert_eq!(Journal::decode(&text), Err(CoreError::InvalidKind));

        // 删掉第一条：seq 从 2 开始，链断掉。
        let mut missing = journal.clone();
        missing.transitions.remove(0);
        let text = serde_json::to_string(&missing.snapshot()).unwrap();
        assert_eq!(Journal::decode(&text), Err(CoreError::InvalidKind));

        // 别的会话的消息。
        let other_session = crate::msg::session_id(&b.did(), &a.did(), &terms(120)).unwrap();
        let foreign = NegotiationMsg::request(&other_session, terms(120))
            .unwrap()
            .signed(&a, &b.did(), 3, None)
            .unwrap();
        let mut j = journal.clone();
        assert_eq!(j.append(&foreign), Err(CoreError::InvalidKind));

        // 重复归档同一条消息。
        let again = journal.messages()[0].clone();
        let mut j = journal.clone();
        assert_eq!(j.append(&again), Err(CoreError::InvalidKind));

        // 第三方签名的记录不能进归档。
        let outsider = agent(9);
        let machine = StateMachine::open(journal.session()).unwrap();
        let mut record = machine.stage(Event::Request, &outsider, 1).unwrap();
        StateMachine::co_sign(&mut record, &b).unwrap();
        let mut j = journal.clone();
        assert_eq!(
            j.append_transition(&record, None),
            Err(CoreError::InvalidSignature)
        );
        assert_eq!(
            j.transitions().len(),
            journal.transitions().len(),
            "被拒的记录不得进入归档"
        );
    }

    #[test]
    fn journal_open_validates_the_pair() {
        let a = agent(3);
        let b = agent(4);
        assert_eq!(Journal::open("s", &[a.did()]), Err(CoreError::InvalidKind));
        assert_eq!(
            Journal::open("s", &[a.did(), a.did()]),
            Err(CoreError::InvalidKind)
        );
        assert_eq!(
            Journal::open("", &[a.did(), b.did()]),
            Err(CoreError::InvalidKind)
        );
        assert!(Journal::open("s-ok", &[a.did(), b.did()]).is_ok());
    }

    #[test]
    fn stats_reports_the_archived_shape() {
        let (journal, _, _) = sample();
        let stats = journal.stats();
        assert_eq!(stats["messages"], 2);
        assert_eq!(stats["transitions"], 2);
        assert_eq!(stats["phase"], "negotiating");
        assert_eq!(stats["rounds_used"], 1);
        assert!(stats["bytes"].as_u64().unwrap() > 100);
    }
}
