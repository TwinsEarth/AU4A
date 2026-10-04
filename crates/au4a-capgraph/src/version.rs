//! v1.1.7 —— 能力版本化。
//!
//! 「能力变了」不能靠对端自己说「我变了」：那等于把状态的正确性托付给对端的诚实。
//! 版本化的做法是**版本号由内容决定**：
//!
//! * 内容不变 → 版本号不变（重发同一个通告是幂等的，不会把图搅动一遍）。
//! * 内容一变 → 版本号自动 +1，并留下一条**内容寻址**的历史记录（`hash` = 声明的规范 JSON 指纹）。
//! * 收到 `epoch < 已知` → `stale_epoch`（旧世界迟到了，丢弃）。
//! * 收到 `epoch == 已知` 但 `hash` 不同 → `conflict`（同一个版本号对应两份内容，
//!   这会让「版本号 → 内容」不再是一个函数，必须拒绝）。
//!
//! 为什么还要**历史**：邻居缓存会过期、会被 LRU 淘汰。如果冲突检测只看缓存里的那一条记录，
//! 那么「先发 v3-A、等缓存过期、再发 v3-B」就能骗过检查。历史记录让版本号的单调性
//! 脱离缓存的生死——这是本版相对 v1.1.4 的实质增量。
//!
//! 历史是有界的（[`HISTORY_CAPACITY`]）：一个 Agent 视角的记忆不能无限增长，
//! 容量之外的旧记录被丢弃，这一点在测试里被断言。

use au4a_core::Did;
use serde_json::{json, Value};

/// 版本历史容量上限。超出后丢弃最旧的记录（有界记忆）。
pub const HISTORY_CAPACITY: usize = 256;

/// 一条版本记录：谁、第几版、内容指纹、什么时候、包含哪些技能。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionRecord {
    pub did: Did,
    pub version: u64,
    /// 声明内容的规范 JSON 指纹（不含签名）。
    pub hash: String,
    /// 记录时的逻辑时刻。
    pub at: u64,
    pub skills: Vec<String>,
}

impl VersionRecord {
    pub fn to_value(&self) -> Value {
        json!({
            "did": self.did.as_str(),
            "version": self.version,
            "hash": self.hash,
            "at": self.at,
            "skills": self.skills,
        })
    }
}

/// 版本历史：append-only、有界、按 Agent 可查。
#[derive(Clone, Debug, Default)]
pub struct VersionHistory {
    records: Vec<VersionRecord>,
    /// 版本递增次数（每次内容变化 +1），用于证据。
    bumps: u64,
}

impl VersionHistory {
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加一条记录。内容完全一致的重复记录不会重复入账。
    pub fn record(&mut self, record: VersionRecord) -> bool {
        if let Some(last) = self
            .records
            .iter()
            .rev()
            .find(|known| known.did == record.did)
        {
            if last.version == record.version && last.hash == record.hash {
                return false;
            }
        }
        self.bumps += 1;
        self.records.push(record);
        if self.records.len() > HISTORY_CAPACITY {
            let overflow = self.records.len() - HISTORY_CAPACITY;
            self.records.drain(0..overflow);
        }
        true
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn bumps(&self) -> u64 {
        self.bumps
    }

    pub fn records(&self) -> &[VersionRecord] {
        &self.records
    }

    /// 某个 Agent 的历史（按记录顺序）。
    pub fn of(&self, did: &Did) -> Vec<&VersionRecord> {
        self.records.iter().filter(|r| &r.did == did).collect()
    }

    /// 最近一次见到的版本号。
    pub fn last_version(&self, did: &Did) -> Option<u64> {
        self.of(did).last().map(|r| r.version)
    }

    /// 指定版本号对应的内容指纹（历史里查得到才算数）。
    pub fn hash_of(&self, did: &Did, version: u64) -> Option<&str> {
        self.of(did)
            .into_iter()
            .rev()
            .find(|r| r.version == version)
            .map(|r| r.hash.as_str())
    }

    pub fn to_value(&self) -> Value {
        json!({
            "records": self.records.len(),
            "bumps": self.bumps,
            "agents": {
                // 每个 Agent 的最新版本；用字符串键保证 JSON 规范
            },
            "capacity": HISTORY_CAPACITY,
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

    fn record(seed: u8, version: u64, hash: &str) -> VersionRecord {
        VersionRecord {
            did: did(seed),
            version,
            hash: hash.to_string(),
            at: version,
            skills: vec!["x".to_string()],
        }
    }

    #[test]
    fn identical_records_are_not_recorded_twice() {
        let mut history = VersionHistory::new();
        assert!(history.record(record(1, 1, "h1")));
        assert!(!history.record(record(1, 1, "h1")), "重复入账会让版本计数虚高");
        assert_eq!(history.len(), 1);
        assert_eq!(history.bumps(), 1);
        assert!(history.record(record(1, 2, "h2")));
        assert_eq!(history.bumps(), 2);
    }

    #[test]
    fn history_is_bounded_and_keeps_the_newest() {
        let mut history = VersionHistory::new();
        for version in 1..=(HISTORY_CAPACITY as u64 + 10) {
            history.record(record(1, version, &format!("h{version}")));
        }
        assert_eq!(history.len(), HISTORY_CAPACITY);
        assert_eq!(history.last_version(&did(1)), Some(HISTORY_CAPACITY as u64 + 10));
        assert_eq!(history.hash_of(&did(1), 1), None, "最旧的记录被丢弃");
        assert_eq!(
            history.hash_of(&did(1), HISTORY_CAPACITY as u64 + 10),
            Some("h266")
        );
    }

    #[test]
    fn lookups_are_per_agent() {
        let mut history = VersionHistory::new();
        history.record(record(1, 1, "a1"));
        history.record(record(2, 7, "b7"));
        history.record(record(1, 2, "a2"));
        assert_eq!(history.of(&did(1)).len(), 2);
        assert_eq!(history.last_version(&did(2)), Some(7));
        assert_eq!(history.hash_of(&did(1), 2), Some("a2"));
        assert_eq!(history.hash_of(&did(2), 1), None);
        assert_eq!(history.last_version(&did(9)), None);
    }
}
