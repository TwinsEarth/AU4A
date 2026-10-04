//! 时间、内容寻址与自检报告。
//!
//! 基元层**不读墙钟**：所有时间来自 [`LogicalClock`]。这让每一次运行都可以被确定性重放，
//! 也让「Agent 的行为是否可复现」成为一个可测试的性质，而不是一句愿望。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 内容哈希（SHA-256，小写 hex）。
pub fn content_hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// 短 id：内容哈希的前 16 位，用于日志与人类可读标识（不参与验签）。
pub fn short_id(full: &str) -> String {
    full.chars().take(16).collect()
}

/// 逻辑时钟。单调、确定、与真实时间无关。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalClock {
    t: u64,
}

impl LogicalClock {
    pub fn new() -> Self {
        Self { t: 0 }
    }

    /// 当前读数（不推进）。
    pub fn now(&self) -> u64 {
        self.t
    }

    /// 推进一格并返回新读数。
    pub fn tick(&mut self) -> u64 {
        self.t += 1;
        self.t
    }

    /// 合并外部观察到的时刻（保证单调：取 max+1）。
    pub fn observe(&mut self, remote: u64) -> u64 {
        self.t = self.t.max(remote) + 1;
        self.t
    }
}

/// 每条轨道、每个小版本都要产出的自检项。
///
/// 部署验证（`au4a-node verify`）聚合全部轨道的自检结果，观察层「结果」面板展示的就是它。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelfCheck {
    /// 轨道号，如 `"1.3"`。
    pub track: String,
    /// 检查名。
    pub name: String,
    pub passed: bool,
    /// 证据说明：具体断言了什么，而不是「应该没问题」。
    pub detail: String,
}

impl SelfCheck {
    pub fn pass(track: &str, name: &str, detail: impl Into<String>) -> Self {
        Self {
            track: track.to_string(),
            name: name.to_string(),
            passed: true,
            detail: detail.into(),
        }
    }

    pub fn fail(track: &str, name: &str, detail: impl Into<String>) -> Self {
        Self {
            track: track.to_string(),
            name: name.to_string(),
            passed: false,
            detail: detail.into(),
        }
    }
}

/// 全部通过才算通过。
pub fn all_passed(checks: &[SelfCheck]) -> bool {
    !checks.is_empty() && checks.iter().all(|c| c.passed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_is_monotonic_and_remote_aware() {
        let mut c = LogicalClock::new();
        assert_eq!(c.now(), 0);
        assert_eq!(c.tick(), 1);
        assert_eq!(c.tick(), 2);
        assert_eq!(c.observe(100), 101);
        assert_eq!(c.observe(3), 102);
    }

    #[test]
    fn content_hash_is_stable() {
        assert_eq!(content_hash(b"au4a"), content_hash(b"au4a"));
        assert_ne!(content_hash(b"au4a"), content_hash(b"au4b"));
    }

    #[test]
    fn empty_check_set_is_not_passing() {
        assert!(!all_passed(&[]));
        assert!(all_passed(&[SelfCheck::pass("1.0", "x", "d")]));
        assert!(!all_passed(&[
            SelfCheck::pass("1.0", "x", "d"),
            SelfCheck::fail("1.0", "y", "d")
        ]));
    }
}
