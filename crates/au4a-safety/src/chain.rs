//! v1.5.2 哈希链事件记录：每一次状态变更都写进一条不可篡改的记录。
//!
//! 记录形状刻意做成最朴素的一种（`prev` + 内容 → `hash`），因为可审计性的价值
//! 全部来自「任何人都能复算」，而不是来自算法的新奇程度：
//!
//! ```text
//! hash_i = SHA256( canonical_json( { seq, at, kind, payload, prev } ) )
//! prev_i = hash_{i-1}         prev_0 = GENESIS_PREV（64 个 0）
//! ```
//!
//! 因此改一个字节、删一条记录、调一次顺序，都会在某个序号上暴露为
//! `hash_mismatch` / `prev_mismatch` / `seq_mismatch`——三种断裂是可区分的，
//! 审计者能说出「哪里断了」，而不是只知道「断了」。

use au4a_core::{canonical_hash, CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 创世前驱：64 个 0。它不是任何事件的哈希，因此天然标记「链的起点」。
pub const GENESIS_PREV: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// 事件种类。每一次**状态变更**恰好对应一种。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafetyEventKind {
    /// 举报受理（案件进入 reported）。
    Reported,
    /// 申诉受理（案件进入 appealed）。
    Appealed,
    /// 处罚执行（案件进入 penalized）。
    Penalized,
    /// 仲裁裁决（案件进入 arbitrated）。
    Arbitrated,
    /// 订阅建立。
    Subscribed,
    /// 订阅撤销。
    Unsubscribed,
}

impl SafetyEventKind {
    pub const ALL: [SafetyEventKind; 6] = [
        SafetyEventKind::Reported,
        SafetyEventKind::Appealed,
        SafetyEventKind::Penalized,
        SafetyEventKind::Arbitrated,
        SafetyEventKind::Subscribed,
        SafetyEventKind::Unsubscribed,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            SafetyEventKind::Reported => "reported",
            SafetyEventKind::Appealed => "appealed",
            SafetyEventKind::Penalized => "penalized",
            SafetyEventKind::Arbitrated => "arbitrated",
            SafetyEventKind::Subscribed => "subscribed",
            SafetyEventKind::Unsubscribed => "unsubscribed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        SafetyEventKind::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// 一条链上事件。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafetyEvent {
    /// 序号：从 0 开始，必须与在链中的位置一致。
    pub seq: u64,
    /// 逻辑时钟读数（不读墙钟）。
    pub at: u64,
    pub kind: SafetyEventKind,
    /// 该次状态变更的完整记录（规范 JSON，禁止浮点）。
    pub payload: Value,
    /// 上一条事件的 `hash`；创世事件为 [`GENESIS_PREV`]。
    pub prev: String,
    /// 本条事件的哈希。
    pub hash: String,
}

impl SafetyEvent {
    /// 被哈希的载荷。
    pub fn hashing_payload(
        seq: u64,
        at: u64,
        kind: SafetyEventKind,
        payload: &Value,
        prev: &str,
    ) -> Value {
        json!({
            "seq": seq,
            "at": at,
            "kind": kind,
            "payload": payload,
            "prev": prev,
        })
    }

    /// 计算哈希（不构造事件）。
    pub fn compute_hash(
        seq: u64,
        at: u64,
        kind: SafetyEventKind,
        payload: &Value,
        prev: &str,
    ) -> CoreResult<String> {
        canonical_hash(&Self::hashing_payload(seq, at, kind, payload, prev))
    }

    /// 追加一条事件：哈希在构造时算好，之后不可变。
    pub fn seal(
        seq: u64,
        at: u64,
        kind: SafetyEventKind,
        payload: Value,
        prev: &str,
    ) -> CoreResult<Self> {
        let hash = Self::compute_hash(seq, at, kind, &payload, prev)?;
        Ok(Self {
            seq,
            at,
            kind,
            payload,
            prev: prev.to_string(),
            hash,
        })
    }

    /// 自校验：内容与哈希一致。
    pub fn verify_hash(&self) -> CoreResult<()> {
        if !is_hex64(&self.prev) || !is_hex64(&self.hash) {
            return Err(CoreError::Encoding);
        }
        if Self::compute_hash(self.seq, self.at, self.kind, &self.payload, &self.prev)? == self.hash
        {
            Ok(())
        } else {
            Err(CoreError::InvalidSignature)
        }
    }

    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn from_json(value: &Value) -> CoreResult<Self> {
        serde_json::from_value(value.clone()).map_err(|_| CoreError::Encoding)
    }
}

/// 断裂原因。三类可区分，审计者能定位「怎么断的」。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainBreak {
    /// 序号与位置不一致（删除/插入记录）。
    SeqMismatch,
    /// `prev` 不等于上一条的 `hash`（替换/重排记录）。
    PrevMismatch,
    /// 内容与自身 `hash` 不一致（篡改内容）。
    HashMismatch,
}

impl ChainBreak {
    pub fn as_str(self) -> &'static str {
        match self {
            ChainBreak::SeqMismatch => "seq_mismatch",
            ChainBreak::PrevMismatch => "prev_mismatch",
            ChainBreak::HashMismatch => "hash_mismatch",
        }
    }
}

/// 链校验结论。它是一个**值**，不是一个 `Result`：断链也要能被审计报告描述。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainVerdict {
    pub ok: bool,
    pub len: usize,
    /// 链头哈希；空链为 [`GENESIS_PREV`]。
    pub head: String,
    /// 断裂位置（事件序号）。
    pub broken_at: Option<u64>,
    pub reason: Option<ChainBreak>,
}

impl ChainVerdict {
    fn broken(len: usize, at: u64, reason: ChainBreak) -> Self {
        Self {
            ok: false,
            len,
            head: GENESIS_PREV.to_string(),
            broken_at: Some(at),
            reason: Some(reason),
        }
    }

    pub fn to_json(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }
}

/// 全链校验：序号、前驱、哈希三项。
pub fn verify_chain(events: &[SafetyEvent]) -> ChainVerdict {
    let mut prev = GENESIS_PREV.to_string();
    for (index, event) in events.iter().enumerate() {
        if event.seq != index as u64 {
            return ChainVerdict::broken(events.len(), index as u64, ChainBreak::SeqMismatch);
        }
        if event.prev != prev {
            return ChainVerdict::broken(events.len(), event.seq, ChainBreak::PrevMismatch);
        }
        if event.verify_hash().is_err() {
            return ChainVerdict::broken(events.len(), event.seq, ChainBreak::HashMismatch);
        }
        prev = event.hash.clone();
    }
    ChainVerdict {
        ok: true,
        len: events.len(),
        head: prev,
        broken_at: None,
        reason: None,
    }
}

/// 链头：空链返回创世前驱。
pub fn chain_head(events: &[SafetyEvent]) -> String {
    events
        .last()
        .map(|e| e.hash.clone())
        .unwrap_or_else(|| GENESIS_PREV.to_string())
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(seq: u64, payload: Value, prev: &str) -> SafetyEvent {
        SafetyEvent::seal(seq, seq + 100, SafetyEventKind::Reported, payload, prev).unwrap()
    }

    fn chain(n: u64) -> Vec<SafetyEvent> {
        let mut events = Vec::new();
        for seq in 0..n {
            let prev = chain_head(&events);
            events.push(event(seq, json!({"seq": seq}), &prev));
        }
        events
    }

    #[test]
    fn an_empty_chain_is_trivially_intact_and_starts_at_genesis() {
        let verdict = verify_chain(&[]);
        assert!(verdict.ok);
        assert_eq!(verdict.head, GENESIS_PREV);
        assert_eq!(verdict.len, 0);
        assert_eq!(chain_head(&[]), GENESIS_PREV);
    }

    #[test]
    fn a_grown_chain_is_intact_and_content_addressed() {
        let events = chain(4);
        let verdict = verify_chain(&events);
        assert!(verdict.ok, "{verdict:?}");
        assert_eq!(verdict.head, events[3].hash);
        assert_eq!(events[0].prev, GENESIS_PREV);
        for pair in events.windows(2) {
            assert_eq!(pair[1].prev, pair[0].hash);
        }
        assert_eq!(verdict.head.len(), 64);
    }

    #[test]
    fn same_payload_different_prev_gives_different_hash() {
        let a = event(0, json!({"x": 1}), GENESIS_PREV);
        let b = event(0, json!({"x": 1}), &"a".repeat(64));
        assert_ne!(a.hash, b.hash);
    }

    #[test]
    fn tampering_with_content_breaks_the_chain_at_that_seq() {
        let mut events = chain(3);
        events[1].payload = json!({"forged": true});
        let verdict = verify_chain(&events);
        assert!(!verdict.ok);
        assert_eq!(verdict.broken_at, Some(1));
        assert_eq!(verdict.reason, Some(ChainBreak::HashMismatch));
        assert_eq!(events[1].verify_hash(), Err(CoreError::InvalidSignature));
    }

    #[test]
    fn removing_or_relinking_records_is_detected() {
        // 删除一条：序号错位。
        let mut removed = chain(3);
        removed.remove(1);
        let verdict = verify_chain(&removed);
        assert_eq!(verdict.broken_at, Some(1));
        assert_eq!(verdict.reason, Some(ChainBreak::SeqMismatch));

        // 把前驱指向别处（重排/嫁接）：前驱校验失败。
        let mut relinked = chain(3);
        relinked[1].prev = "c".repeat(64);
        let verdict = verify_chain(&relinked);
        assert!(!verdict.ok);
        assert_eq!(verdict.broken_at, Some(1));
        assert_eq!(verdict.reason, Some(ChainBreak::PrevMismatch));
    }

    #[test]
    fn forged_hash_is_detected_even_when_content_is_untouched() {
        let mut events = chain(2);
        events[1].hash = "b".repeat(64);
        let verdict = verify_chain(&events);
        assert_eq!(verdict.broken_at, Some(1));
        assert_eq!(verdict.reason, Some(ChainBreak::HashMismatch));
    }

    #[test]
    fn events_roundtrip_through_json_and_kinds_are_unique() {
        let events = chain(2);
        let json = events[0].to_json().unwrap();
        assert_eq!(SafetyEvent::from_json(&json).unwrap(), events[0]);
        assert_eq!(SafetyEventKind::ALL.len(), 6);
        for kind in SafetyEventKind::ALL {
            assert_eq!(SafetyEventKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(SafetyEventKind::parse("ignored"), None);
        let verdict = verify_chain(&events);
        assert_eq!(verdict.to_json().unwrap()["ok"], json!(true));
    }

    #[test]
    fn malformed_hashes_are_refused() {
        let mut events = chain(1);
        events[0].prev = "zz".to_string();
        assert_eq!(events[0].verify_hash(), Err(CoreError::Encoding));
    }
}
