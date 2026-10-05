use std::fmt;

use serde::{Deserialize, Serialize};

/// 基元层错误。刻意保持小而封闭：新增变体是接口变更。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreError {
    /// DID 不是 `did:au4a:<64 位小写 hex>`。
    InvalidDid,
    /// 签名缺失、格式错误或验证失败。
    InvalidSignature,
    /// 规范 JSON 拒绝浮点：金额与度量一律整数。
    FloatForbidden,
    /// 序列化/反序列化失败。
    Encoding,
    /// 金额为负。
    NegativeAmount,
    /// 金额为零（零额转账不是错误语义，是调用错误）。
    ZeroAmount,
    /// 整数溢出。
    Overflow,
    /// 余额不足。
    InsufficientFunds,
    /// 质押不足（低于网络 `min_stake`）。
    InsufficientStake,
    /// 未知 Agent。
    UnknownAgent,
    /// 重复注册。
    DuplicateAgent,
    /// 消息类型名不合法。
    InvalidKind,
    /// 帧超过 1 MiB 上限。
    FrameTooLarge,
    /// 帧长度前缀与实际负载不一致。
    FrameTruncated,
    /// 信封未签名。
    NotSealed,
    /// 版本号不合法。
    InvalidVersion,
    /// 守恒不变式被破坏：`Σavailable + Σlocked + slashed == minted` 不成立。
    ///
    /// v2.3.0 新增：这**不是溢出**（v2.3.0 之前 `check_conservation` 误报 `Overflow`），
    /// 也不是金额问题，而是账本内部状态不一致——必须能被独立识别与告警。
    ConservationViolated,
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            CoreError::InvalidDid => "invalid DID",
            CoreError::InvalidSignature => "invalid signature",
            CoreError::FloatForbidden => "floats are forbidden in canonical JSON",
            CoreError::Encoding => "encoding error",
            CoreError::NegativeAmount => "negative amount",
            CoreError::ZeroAmount => "zero amount",
            CoreError::Overflow => "integer overflow",
            CoreError::InsufficientFunds => "insufficient funds",
            CoreError::InsufficientStake => "insufficient stake",
            CoreError::UnknownAgent => "unknown agent",
            CoreError::DuplicateAgent => "duplicate agent",
            CoreError::InvalidKind => "invalid message kind",
            CoreError::FrameTooLarge => "frame too large",
            CoreError::FrameTruncated => "frame truncated",
            CoreError::NotSealed => "envelope not sealed",
            CoreError::InvalidVersion => "invalid version",
            CoreError::ConservationViolated => "conservation violated",
        };
        f.write_str(s)
    }
}

impl std::error::Error for CoreError {}

pub type CoreResult<T> = Result<T, CoreError>;
