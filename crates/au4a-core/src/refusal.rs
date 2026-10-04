//! 类型化拒绝。
//!
//! 参考项目里最有价值的工程判断是：**不能因为一次拒绝就隔离一个 Agent**，
//! 因为「拒绝」既可能是竞争（对方还没同步、自己太忙、消息过期），也可能是恶意。
//! 把两者混在一起，规则就会在竞争上误伤。
//!
//! AU4A 因此把拒绝码分成两类：
//!
//! * [`RefusalCode::is_misconduct`] 为真的码：一次出现即构成恶意证据。
//!   只有两个——`malformed`（帧不可解析，这不可能是竞争，因为分帧是确定性的）
//!   和 `unauthorized`（签名或身份不成立，这要么是伪造要么是密钥失守）。
//! * 其余八个码都是「竞争/容量」语义，单次出现只记警告；
//!   只有**重复**触发（超过阈值）才升级，且升级理由是重复行为本身，而不是拒绝本身。

use serde::{Deserialize, Serialize};

/// 十种拒绝码。新增变体是协议变更，必须走新的中版本。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalCode {
    /// 帧或载荷不可解析/不合法。
    Malformed,
    /// 身份或签名不成立。
    Unauthorized,
    /// 策略引擎拒绝（可能是合理的）。
    PolicyDenied,
    /// 触发限流。
    RateLimited,
    /// 容量耗尽。
    ResourceExhausted,
    /// 对端落后于当前纪元。
    StaleEpoch,
    /// 能力存在但当前不支持该参数。
    Unsupported,
    /// 与本地状态冲突（并发写）。
    Conflict,
    /// 超时。
    Timeout,
    /// 服务降级中。
    Degraded,
}

impl RefusalCode {
    pub const ALL: [RefusalCode; 10] = [
        RefusalCode::Malformed,
        RefusalCode::Unauthorized,
        RefusalCode::PolicyDenied,
        RefusalCode::RateLimited,
        RefusalCode::ResourceExhausted,
        RefusalCode::StaleEpoch,
        RefusalCode::Unsupported,
        RefusalCode::Conflict,
        RefusalCode::Timeout,
        RefusalCode::Degraded,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RefusalCode::Malformed => "malformed",
            RefusalCode::Unauthorized => "unauthorized",
            RefusalCode::PolicyDenied => "policy_denied",
            RefusalCode::RateLimited => "rate_limited",
            RefusalCode::ResourceExhausted => "resource_exhausted",
            RefusalCode::StaleEpoch => "stale_epoch",
            RefusalCode::Unsupported => "unsupported",
            RefusalCode::Conflict => "conflict",
            RefusalCode::Timeout => "timeout",
            RefusalCode::Degraded => "degraded",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        RefusalCode::ALL.into_iter().find(|c| c.as_str() == s)
    }

    /// 单次拒绝是否构成恶意证据。
    pub fn is_misconduct(self) -> bool {
        matches!(self, RefusalCode::Malformed | RefusalCode::Unauthorized)
    }

    /// 是否可重试（对 Agent 自主重试策略有用）。
    pub fn retryable(self) -> bool {
        matches!(
            self,
            RefusalCode::RateLimited
                | RefusalCode::ResourceExhausted
                | RefusalCode::StaleEpoch
                | RefusalCode::Conflict
                | RefusalCode::Timeout
                | RefusalCode::Degraded
        )
    }
}

/// 拒绝的处置级别。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Escalation {
    /// 只记录。
    None,
    /// 记警告，不改变信誉。
    Warn,
    /// 建议隔离（需委员会确认）。
    Quarantine,
}

/// 重复次数的默认阈值：同一个非恶意码重复到这个次数才升级。
pub const REPEAT_THRESHOLD: u32 = 5;

/// 由拒绝码与重复计数决定处置。
pub fn escalate(code: RefusalCode, repeats: u32) -> Escalation {
    if code.is_misconduct() {
        return Escalation::Quarantine;
    }
    if repeats >= REPEAT_THRESHOLD {
        Escalation::Warn
    } else {
        Escalation::None
    }
}

/// 一条拒绝记录。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    pub code: RefusalCode,
    pub reason: String,
    pub retryable: bool,
}

impl Refusal {
    pub fn new(code: RefusalCode, reason: impl Into<String>) -> Self {
        Self {
            code,
            reason: reason.into(),
            retryable: code.retryable(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_two_codes_are_misconduct_in_one_shot() {
        let misconduct: Vec<&str> = RefusalCode::ALL
            .iter()
            .filter(|c| c.is_misconduct())
            .map(|c| c.as_str())
            .collect();
        assert_eq!(misconduct, vec!["malformed", "unauthorized"]);
    }

    #[test]
    fn a_race_does_not_quarantine() {
        for code in RefusalCode::ALL.iter().filter(|c| !c.is_misconduct()) {
            assert_eq!(escalate(*code, 1), Escalation::None, "{}", code.as_str());
            assert_ne!(escalate(*code, 1), Escalation::Quarantine);
        }
    }

    #[test]
    fn repetition_escalates_but_only_to_warn() {
        assert_eq!(
            escalate(RefusalCode::Timeout, REPEAT_THRESHOLD - 1),
            Escalation::None
        );
        assert_eq!(
            escalate(RefusalCode::Timeout, REPEAT_THRESHOLD),
            Escalation::Warn
        );
        assert_eq!(escalate(RefusalCode::Timeout, 10_000), Escalation::Warn);
    }

    #[test]
    fn codes_roundtrip() {
        assert_eq!(RefusalCode::ALL.len(), 10);
        for c in RefusalCode::ALL {
            assert_eq!(RefusalCode::parse(c.as_str()), Some(c));
        }
        assert_eq!(RefusalCode::parse("nonsense"), None);
    }

    #[test]
    fn retryability_matches_semantics() {
        assert!(RefusalCode::Timeout.retryable());
        assert!(!RefusalCode::Malformed.retryable());
        assert!(!RefusalCode::PolicyDenied.retryable());
    }
}
