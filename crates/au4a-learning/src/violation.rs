//! v1.6.3 违规记录（Violation Log）。
//!
//! 协作对象选择策略需要两类完全不同的输入：
//!
//! * **质量信号**：这个协作者过往交付得好不好（来自经验库，见 `feedback`）。
//! * **违规记录**：这个协作者有没有做过「收了钱不交付」这类越界行为。
//!
//! 两者不能混为一谈：低质量可以靠多给机会改善，违规必须立刻降低被选中概率。
//! 所以违规单独记账、按协作者计数，并且**永不转让**（没有转出/合并接口）。

use std::collections::BTreeMap;

use au4a_core::{CoreError, CoreResult, Did, SelfCheck};
use serde::{Deserialize, Serialize};

/// 违规原因串长度上限（防止一条记录吃掉内存）。
pub const MAX_REASON_LEN: usize = 128;

/// 一条违规记录。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Violation {
    /// 违规的协作者。
    pub peer: Did,
    /// 关联任务（本地保留，对外不发布）。
    pub task_id: String,
    /// 类型化原因（单 token，例如 `peer-withheld-deliverable`）。
    pub reason: String,
    /// 逻辑时刻。
    pub at: u64,
}

impl Violation {
    pub fn new(peer: &Did, task_id: &str, reason: &str, at: u64) -> CoreResult<Self> {
        let v = Self {
            peer: peer.clone(),
            task_id: task_id.to_string(),
            reason: reason.to_string(),
            at,
        };
        v.validate()?;
        Ok(v)
    }

    pub fn validate(&self) -> CoreResult<()> {
        if !is_token(&self.task_id) || !is_token(&self.reason) {
            return Err(CoreError::InvalidKind);
        }
        Ok(())
    }
}

/// 违规台账：按协作者累计计数，同时保留完整记录（本地证据）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViolationLog {
    entries: Vec<Violation>,
    by_peer: BTreeMap<Did, u32>,
}

impl ViolationLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, violation: Violation) -> CoreResult<()> {
        violation.validate()?;
        let count = self.by_peer.entry(violation.peer.clone()).or_insert(0);
        *count = count.saturating_add(1);
        self.entries.push(violation);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 台账里的违规总数（跨协作者）。
    pub fn total(&self) -> u32 {
        self.by_peer
            .values()
            .copied()
            .fold(0u32, |a, b| a.saturating_add(b))
    }

    pub fn count_of(&self, peer: &Did) -> u32 {
        self.by_peer.get(peer).copied().unwrap_or(0)
    }

    /// 有过违规的协作者（升序，确定性）。
    pub fn peers(&self) -> Vec<Did> {
        self.by_peer.keys().cloned().collect()
    }

    pub fn entries(&self) -> &[Violation] {
        &self.entries
    }

    /// 公开投影：只发布「几个协作者、共几条违规、原因分布」，不发布 DID 与任务标识。
    pub fn public_json(&self) -> CoreResult<serde_json::Value> {
        let mut reasons: BTreeMap<String, u32> = BTreeMap::new();
        for e in &self.entries {
            let slot = reasons.entry(e.reason.clone()).or_insert(0);
            *slot = slot.saturating_add(1);
        }
        Ok(serde_json::json!({
            "peers_with_violations": self.by_peer.len(),
            "total": self.total(),
            "reasons": reasons,
        }))
    }

    /// 真实断言：计数正确、拒绝非法原因、公开投影不含 DID。
    pub fn self_check() -> Vec<SelfCheck> {
        let mut checks = Vec::new();
        let peer = au4a_core::AgentKeys::from_seed(&[0x40; 32]).did();
        let mut log = ViolationLog::new();
        let mut recorded = 0usize;
        for i in 0..3u64 {
            if let Ok(v) =
                Violation::new(&peer, &format!("task-{i}"), "peer-withheld-deliverable", i)
            {
                if log.record(v).is_ok() {
                    recorded += 1;
                }
            }
        }
        let counted =
            recorded == 3 && log.len() == 3 && log.total() == 3 && log.count_of(&peer) == 3;
        checks.push(crate::check(
            "violation.counting",
            counted,
            format!(
                "记录 3 条 → len={} total={} count_of={}",
                log.len(),
                log.total(),
                log.count_of(&peer)
            ),
        ));

        let rejected = Violation::new(&peer, "task", "", 0).is_err()
            && Violation::new(&peer, "", "r", 0).is_err()
            && Violation::new(&peer, "task", &"x".repeat(MAX_REASON_LEN + 1), 0).is_err();
        let redacted = log
            .public_json()
            .map(|v| !v.to_string().contains("did:au4a:") && v["total"] == serde_json::json!(3))
            .unwrap_or(false);
        checks.push(crate::check(
            "violation.rejects_and_redacts",
            rejected && redacted,
            format!("空原因/空任务/超长原因被拒：{rejected}；公开投影无 DID：{redacted}"),
        ));
        checks
    }
}

fn is_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_REASON_LEN
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

/// 模块级自检入口（`crate::self_check` 聚合它）。
pub fn self_check() -> Vec<SelfCheck> {
    ViolationLog::self_check()
}
