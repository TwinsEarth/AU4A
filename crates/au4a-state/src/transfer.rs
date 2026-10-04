//! 跨节点传输（v1.3.2）：**本地双节点内存实现**，证据等级 `cpu-proto`。
//!
//! 这里刻意不引入 libp2p / TCP / QUIC，也不碰文件系统：
//!
//! * 真实网络里会失败的东西（丢包、乱序、重复、断线重连）在这里用**确定性的注入**复现，
//!   因此「可续跑」这件事是被测试证伪过的，而不是靠文档承诺。
//! * 线格式复用基元层 PMB 的分帧语义：**4 字节大端长度前缀 + 规范 JSON，1 MiB 上限，
//!   超限在分配之前拒绝**。载荷是快照/差异块，不是 `Envelope`（那是 PMB 的消息语义）。
//!
//! 证据等级：`cpu-proto`——语义完整、可重放，但**没有真实网络**。文档里必须这么写。

use std::collections::BTreeMap;

use au4a_core::{canonicalize, CoreError, CoreResult, Did};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::diff::{DelOp, DeltaChunk, DeltaOp, StateDelta};
use crate::snapshot::{StateSnapshot, MAX_NODE_LEN};

/// 单帧上限：与 PMB 一致（1 MiB）。
pub const MAX_FRAME: usize = 1024 * 1024;
/// 默认每块操作数上限。
pub const DEFAULT_CHUNK_OPS: usize = 64;

/// 节点标识（本地双节点实现里的名字，不是网络地址）。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(String);

impl NodeId {
    pub fn new(name: &str) -> CoreResult<Self> {
        if name.is_empty() || name.len() > MAX_NODE_LEN {
            return Err(CoreError::Encoding);
        }
        if name.chars().any(|c| (c as u32) < 0x20) {
            return Err(CoreError::Encoding);
        }
        Ok(Self(name.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// 编码为线格式：4 字节大端长度前缀 + 规范 JSON。
pub fn encode_frame(value: &Value) -> CoreResult<Vec<u8>> {
    let payload = canonicalize(value)?;
    let bytes = payload.as_bytes();
    if bytes.len() > MAX_FRAME {
        return Err(CoreError::FrameTooLarge);
    }
    let mut out = Vec::with_capacity(4 + bytes.len());
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(out)
}

/// 解码：先读前缀，超限立刻拒绝（不分配负载），再校验长度与 JSON。
pub fn decode_frame(buf: &[u8]) -> CoreResult<Value> {
    if buf.len() < 4 {
        return Err(CoreError::FrameTruncated);
    }
    let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if len > MAX_FRAME {
        return Err(CoreError::FrameTooLarge);
    }
    if buf.len() != 4 + len {
        return Err(CoreError::FrameTruncated);
    }
    let text = std::str::from_utf8(&buf[4..]).map_err(|_| CoreError::Encoding)?;
    serde_json::from_str(text).map_err(|_| CoreError::Encoding)
}

/// 网络计数器（确定性；不读墙钟，因此可以被断言）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkStats {
    pub frames_sent: u64,
    pub frames_delivered: u64,
    pub frames_dropped: u64,
    pub bytes: u64,
}

/// 本地双节点（可扩展到多节点）内存通道。
///
/// 三个刻意的性质：
/// 1. **不联网**：帧只在本进程的 mailbox 之间移动。
/// 2. **可丢弃**：`drop_pending` 模拟断线，用来证明「续跑」不是摆设。
/// 3. **确定性**：mailbox 是 `BTreeMap`，收帧顺序与投递顺序都由调用方决定。
#[derive(Clone, Debug, Default)]
pub struct LocalNetwork {
    mailboxes: BTreeMap<String, Vec<Vec<u8>>>,
    stats: NetworkStats,
}

impl LocalNetwork {
    pub fn new(nodes: &[NodeId]) -> Self {
        let mut mailboxes = BTreeMap::new();
        for node in nodes {
            mailboxes.insert(node.as_str().to_string(), Vec::new());
        }
        Self {
            mailboxes,
            stats: NetworkStats::default(),
        }
    }

    pub fn has_node(&self, node: &NodeId) -> bool {
        self.mailboxes.contains_key(node.as_str())
    }

    /// 投递一帧。未知节点返回 `UnknownAgent`（拒绝，而不是静默丢弃）。
    pub fn deliver(&mut self, from: &NodeId, to: &NodeId, frame: Vec<u8>) -> CoreResult<()> {
        if !self.has_node(from) || !self.has_node(to) {
            return Err(CoreError::UnknownAgent);
        }
        self.stats.frames_sent += 1;
        self.stats.bytes += frame.len() as u64;
        if let Some(box_) = self.mailboxes.get_mut(to.as_str()) {
            box_.push(frame);
            self.stats.frames_delivered += 1;
            Ok(())
        } else {
            Err(CoreError::UnknownAgent)
        }
    }

    /// 取出某个节点的全部待收帧。
    pub fn take(&mut self, node: &NodeId) -> CoreResult<Vec<Vec<u8>>> {
        match self.mailboxes.get_mut(node.as_str()) {
            Some(box_) => Ok(std::mem::take(box_)),
            None => Err(CoreError::UnknownAgent),
        }
    }

    /// 模拟断线：丢弃某个节点未收的帧，返回丢弃条数。
    pub fn drop_pending(&mut self, node: &NodeId) -> CoreResult<usize> {
        match self.mailboxes.get_mut(node.as_str()) {
            Some(box_) => {
                let n = box_.len();
                box_.clear();
                self.stats.frames_dropped += n as u64;
                Ok(n)
            }
            None => Err(CoreError::UnknownAgent),
        }
    }

    pub fn pending(&self, node: &NodeId) -> usize {
        self.mailboxes
            .get(node.as_str())
            .map(|b| b.len())
            .unwrap_or(0)
    }

    pub fn stats(&self) -> NetworkStats {
        self.stats
    }
}

/// 传输报告：确定性计数，用来写证据（而不是「大概传完了」）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferReport {
    pub delta_id: String,
    pub chunks: usize,
    pub frames: u64,
    pub bytes: u64,
    pub set_ops: usize,
    pub del_ops: usize,
    pub resumed_from: usize,
}

/// 发送侧：把一份差异切成块并投递。`skip_before` 用来模拟「前 k 块已送达，不重发」。
pub fn send_delta(
    net: &mut LocalNetwork,
    from: &NodeId,
    to: &NodeId,
    delta: &StateDelta,
    max_ops: usize,
    skip_before: usize,
) -> CoreResult<TransferReport> {
    send_chunks(net, from, to, delta, max_ops, skip_before, usize::MAX)
}

/// 只发 `[start, start + count)` 这一段块；`count = usize::MAX` 表示发到结尾。
///
/// 为什么要能「只发一段」：断线/丢包是传输的常态，续跑必须能只补缺失的块。
/// 发送窗口因此成为协议的一部分，而不是调用方的巧合。
pub fn send_chunks(
    net: &mut LocalNetwork,
    from: &NodeId,
    to: &NodeId,
    delta: &StateDelta,
    max_ops: usize,
    start: usize,
    count: usize,
) -> CoreResult<TransferReport> {
    let chunks = delta.chunked(max_ops)?;
    let mut sent = 0u64;
    let mut bytes = 0u64;
    for chunk in chunks.iter().skip(start).take(count) {
        let frame = encode_frame(&chunk.to_value()?)?;
        bytes += frame.len() as u64;
        net.deliver(from, to, frame)?;
        sent += 1;
    }
    Ok(TransferReport {
        delta_id: delta.id()?,
        chunks: chunks.len(),
        frames: sent,
        bytes,
        set_ops: delta.set().len(),
        del_ops: delta.del().len(),
        resumed_from: start,
    })
}

/// 接收侧会话：按块累积，**重复块幂等**，缺块可点名重发。
///
/// 续跑的关键不变式：收到一块之后，收到的块集合只会变大，且每块的摘要都自洽；
/// 因此「断在哪里」都不影响最终结果。拼装时按 index 排序，与到达顺序无关。
#[derive(Clone, Debug)]
pub struct TransferSession {
    delta_id: String,
    from_content_root: String,
    to_content_root: String,
    total_ops: usize,
    received: BTreeMap<usize, DeltaChunk>,
    duplicates: u64,
}

impl TransferSession {
    /// 用第一块「开工」：会话身份来自块自身（delta id + 前后内容根）。
    pub fn open(chunk: &DeltaChunk) -> CoreResult<Self> {
        chunk.verify()?;
        Ok(Self {
            delta_id: chunk.delta_id().to_string(),
            from_content_root: chunk.from_content_root().to_string(),
            to_content_root: chunk.to_content_root().to_string(),
            total_ops: chunk.total_ops(),
            received: BTreeMap::new(),
            duplicates: 0,
        })
    }

    pub fn delta_id(&self) -> &str {
        &self.delta_id
    }

    pub fn total_ops(&self) -> usize {
        self.total_ops
    }

    pub fn received_chunks(&self) -> usize {
        self.received.len()
    }

    pub fn received_ops(&self) -> usize {
        self.received.values().map(|c| c.op_count()).sum()
    }

    pub fn duplicates(&self) -> u64 {
        self.duplicates
    }

    /// 接收一块。三种拒绝：会话不匹配（`InvalidVersion`）、块摘要不自洽（`InvalidSignature`）、
    /// 块内容与既有块冲突（`Encoding`）。重复块是**幂等接受**（网络重发是常态，不是错误）。
    pub fn accept(&mut self, chunk: &DeltaChunk) -> CoreResult<AcceptOutcome> {
        chunk.verify()?;
        if chunk.delta_id() != self.delta_id
            || chunk.from_content_root() != self.from_content_root
            || chunk.to_content_root() != self.to_content_root
            || chunk.total_ops() != self.total_ops
        {
            return Err(CoreError::InvalidVersion);
        }
        match self.received.get(&chunk.index()) {
            Some(existing) if existing.digest() == chunk.digest() => {
                self.duplicates += 1;
                Ok(AcceptOutcome::Duplicate)
            }
            Some(_) => Err(CoreError::Encoding),
            None => {
                self.received.insert(chunk.index(), chunk.clone());
                Ok(AcceptOutcome::Accepted)
            }
        }
    }

    /// 还缺哪些块（升序）。
    pub fn missing(&self, total_chunks: usize) -> Vec<usize> {
        (0..total_chunks)
            .filter(|i| !self.received.contains_key(i))
            .collect()
    }

    /// 已连续收到的前缀长度：续跑时从这里继续发。
    pub fn resume_from(&self) -> usize {
        let mut next = 0usize;
        while self.received.contains_key(&next) {
            next += 1;
        }
        next
    }

    pub fn is_complete(&self, total_chunks: usize) -> bool {
        self.missing(total_chunks).is_empty()
    }

    /// 拼装成完整差异。缺块 → `FrameTruncated`（拒绝一个不完整的差异）。
    pub fn assemble(&self, total_chunks: usize, agent: Did) -> CoreResult<StateDelta> {
        if !self.is_complete(total_chunks) {
            return Err(CoreError::FrameTruncated);
        }
        let mut set: Vec<DeltaOp> = Vec::with_capacity(self.total_ops);
        let mut del: Vec<DelOp> = Vec::new();
        for index in 0..total_chunks {
            let chunk = self.received.get(&index).ok_or(CoreError::FrameTruncated)?;
            set.extend(chunk.set().iter().cloned());
            del.extend(chunk.del().iter().cloned());
        }
        if set.len() + del.len() != self.total_ops {
            return Err(CoreError::Encoding);
        }
        StateDelta::from_ops(
            agent,
            self.from_content_root.clone(),
            self.to_content_root.clone(),
            set,
            del,
        )
    }
}

/// 接收结果：新块还是重复块。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptOutcome {
    Accepted,
    Duplicate,
}

/// 从 mailbox 里把帧块收进会话（一次「拉取」）。返回本次收到的块数与结果。
pub fn pull_into_session(
    net: &mut LocalNetwork,
    node: &NodeId,
    session: Option<TransferSession>,
) -> CoreResult<(TransferSession, usize, usize)> {
    let frames = net.take(node)?;
    let mut session = match session {
        Some(s) => s,
        None => {
            let first = frames.first().ok_or(CoreError::FrameTruncated)?;
            TransferSession::open(&decode_chunk(first)?)?
        }
    };
    let mut accepted = 0usize;
    let mut duplicates = 0usize;
    for frame in &frames {
        let chunk = decode_chunk(frame)?;
        match session.accept(&chunk)? {
            AcceptOutcome::Accepted => accepted += 1,
            AcceptOutcome::Duplicate => duplicates += 1,
        }
    }
    Ok((session, accepted, duplicates))
}

/// 一帧 → 差异块。
pub fn decode_chunk(buf: &[u8]) -> CoreResult<DeltaChunk> {
    let value = decode_frame(buf)?;
    DeltaChunk::from_value(&value)
}

/// 一帧 → 快照（整份快照的直传路径，v1.3.3 起会带着签名一起传）。
pub fn decode_snapshot(buf: &[u8]) -> CoreResult<StateSnapshot> {
    let value = decode_frame(buf)?;
    StateSnapshot::from_value(&value)
}

/// 快照 → 一帧。
pub fn encode_snapshot(snapshot: &StateSnapshot) -> CoreResult<Vec<u8>> {
    encode_frame(&snapshot.to_value()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{StateBlock, StateZone};
    use serde_json::json;

    fn did(seed: u8) -> Did {
        au4a_core::AgentKeys::from_seed(&[seed; 32]).did()
    }

    fn snap(pairs: &[(&str, &str, Value)]) -> StateSnapshot {
        let blocks: Vec<StateBlock> = pairs
            .iter()
            .map(|(z, k, v)| StateBlock::new(StateZone::parse(z).unwrap(), *k, v.clone()).unwrap())
            .collect();
        StateSnapshot::capture(&did(1), "node-a", 1, blocks).unwrap()
    }

    #[test]
    fn frame_roundtrip_and_prefix_limits() {
        let value = json!({"a": 1, "b": [2, 3]});
        let frame = encode_frame(&value).unwrap();
        assert_eq!(&frame[..4], &(frame.len() as u32 - 4).to_be_bytes());
        assert_eq!(decode_frame(&frame).unwrap(), value);
        assert_eq!(decode_frame(&[0, 0]), Err(CoreError::FrameTruncated));
        let mut oversized = Vec::from((MAX_FRAME as u32 + 1).to_be_bytes());
        oversized.extend_from_slice(b"{}");
        assert_eq!(decode_frame(&oversized), Err(CoreError::FrameTooLarge));
    }

    #[test]
    fn a_full_delta_travels_between_two_local_nodes() {
        let a = snap(&[("fs", "/a", json!(1)), ("memory", "m", json!("x"))]);
        let b = snap(&[("fs", "/a", json!(2)), ("context", "goal", json!("go"))]);
        let delta = StateDelta::between(&a, &b).unwrap();

        let na = NodeId::new("node-a").unwrap();
        let nb = NodeId::new("node-b").unwrap();
        let mut net = LocalNetwork::new(&[na.clone(), nb.clone()]);
        let report = send_delta(&mut net, &na, &nb, &delta, 2, 0).unwrap();
        assert_eq!(report.chunks, 2);
        assert_eq!(report.frames, 2);
        assert!(report.bytes > 0);

        let (session, accepted, duplicates) = pull_into_session(&mut net, &nb, None).unwrap();
        assert_eq!(accepted, 2);
        assert_eq!(duplicates, 0);
        assert!(session.is_complete(report.chunks));
        let rebuilt = session.assemble(report.chunks, did(1)).unwrap();
        assert_eq!(rebuilt.id().unwrap(), delta.id().unwrap());
        assert_eq!(
            rebuilt.apply_to(&a).unwrap().content_root().unwrap(),
            b.content_root().unwrap()
        );
    }

    #[test]
    fn a_dropped_transfer_resumes_without_restarting() {
        // 源上只有一块，目标上有 12 块新内容 → 12 个 set 操作，按每块 2 个操作切成 6 块。
        let from = snap(&[("memory", "k0", json!(0))]);
        let to = StateSnapshot::capture(
            &did(1),
            "node-a",
            1,
            (0..12)
                .map(|i| StateBlock::new(StateZone::Memory, format!("k{i}"), json!(i + 1)).unwrap())
                .collect(),
        )
        .unwrap();
        let delta = StateDelta::between(&from, &to).unwrap();
        let chunks = delta.chunked(2).unwrap();
        assert_eq!(chunks.len(), 6);
        assert_eq!(delta.op_count(), 12);

        let na = NodeId::new("node-a").unwrap();
        let nb = NodeId::new("node-b").unwrap();
        let mut net = LocalNetwork::new(&[na.clone(), nb.clone()]);

        // 第一段：全量发出，只「收到」前两块（其余帧被丢弃，模拟断线）。
        let full = send_delta(&mut net, &na, &nb, &delta, 2, 0).unwrap();
        assert_eq!(full.frames, 6);
        let mut session = TransferSession::open(&chunks[0]).unwrap();
        session.accept(&chunks[0]).unwrap();
        session.accept(&chunks[1]).unwrap();
        assert_eq!(session.resume_from(), 2);
        assert_eq!(net.drop_pending(&nb).unwrap(), 6);
        assert_eq!(net.stats().frames_dropped, 6);

        // 断线后重连：从第 2 块继续发（前 2 块不重发，也不重新协商）。
        let resume = send_delta(&mut net, &na, &nb, &delta, 2, session.resume_from()).unwrap();
        assert_eq!(resume.resumed_from, 2);
        assert_eq!(resume.frames, 4);
        let (session, accepted, duplicates) =
            pull_into_session(&mut net, &nb, Some(session)).unwrap();
        assert_eq!(accepted, 4);
        assert_eq!(duplicates, 0);
        assert!(session.is_complete(6));

        let rebuilt = session.assemble(6, did(1)).unwrap();
        assert_eq!(rebuilt.id().unwrap(), delta.id().unwrap());
        assert_eq!(
            rebuilt.apply_to(&from).unwrap().content_root().unwrap(),
            to.content_root().unwrap()
        );
    }

    #[test]
    fn duplicate_chunks_are_idempotent() {
        let a = snap(&[("memory", "m", json!(1))]);
        let b = snap(&[("memory", "m", json!(2)), ("memory", "n", json!(3))]);
        let delta = StateDelta::between(&a, &b).unwrap();
        let chunks = delta.chunked(1).unwrap();
        let mut session = TransferSession::open(&chunks[0]).unwrap();
        assert_eq!(session.accept(&chunks[0]).unwrap(), AcceptOutcome::Accepted);
        assert_eq!(
            session.accept(&chunks[0]).unwrap(),
            AcceptOutcome::Duplicate
        );
        assert_eq!(session.duplicates(), 1);
        // 同一 index 却是**另一份自洽内容** → 冲突，必须拒绝而不是覆盖。
        let conflicting = crate::diff::test_chunk(
            0,
            delta.id().unwrap().as_str(),
            delta.from_content_root(),
            delta.to_content_root(),
            vec![crate::diff::DeltaOp {
                zone: StateZone::Memory,
                key: "m".to_string(),
                value: json!("hijacked"),
                digest: StateBlock::new(StateZone::Memory, "m", json!("hijacked"))
                    .unwrap()
                    .digest()
                    .to_string(),
            }],
            vec![],
            delta.op_count(),
        )
        .unwrap();
        assert!(conflicting.verify().is_ok());
        assert_eq!(session.accept(&conflicting), Err(CoreError::Encoding));
    }

    #[test]
    fn an_incomplete_session_refuses_to_assemble() {
        let a = snap(&[("memory", "m", json!(1))]);
        let b = snap(&[("memory", "m", json!(2)), ("memory", "n", json!(3))]);
        let delta = StateDelta::between(&a, &b).unwrap();
        let chunks = delta.chunked(1).unwrap();
        let mut session = TransferSession::open(&chunks[0]).unwrap();
        session.accept(&chunks[0]).unwrap();
        assert_eq!(session.missing(2), vec![1]);
        assert_eq!(session.assemble(2, did(1)), Err(CoreError::FrameTruncated));
    }

    #[test]
    fn unknown_nodes_are_refused_and_drops_are_counted() {
        let na = NodeId::new("node-a").unwrap();
        let nx = NodeId::new("node-x").unwrap();
        let nb = NodeId::new("node-b").unwrap();
        let mut net = LocalNetwork::new(&[na.clone(), nb.clone()]);
        assert_eq!(
            net.deliver(&na, &nx, vec![0, 0, 0, 2, b'{', b'}']),
            Err(CoreError::UnknownAgent)
        );
        net.deliver(&na, &nb, vec![0, 0, 0, 2, b'{', b'}']).unwrap();
        assert_eq!(net.pending(&nb), 1);
        assert_eq!(net.drop_pending(&nb).unwrap(), 1);
        let stats = net.stats();
        assert_eq!(stats.frames_sent, 1);
        assert_eq!(stats.frames_dropped, 1);
    }

    #[test]
    fn snapshot_frames_travel_intact() {
        let s = snap(&[("fs", "/a", json!(1)), ("context", "goal", json!("g"))]);
        let frame = encode_snapshot(&s).unwrap();
        let back = decode_snapshot(&frame).unwrap();
        assert_eq!(back.root(), s.root());
    }

    #[test]
    fn node_ids_are_validated() {
        assert!(NodeId::new("").is_err());
        assert!(NodeId::new(&"n".repeat(MAX_NODE_LEN + 1)).is_err());
        assert!(NodeId::new("node-a").is_ok());
    }
}
