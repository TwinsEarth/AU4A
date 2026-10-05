//! 内核语义错误（v2.1.0 · 迁移 S1）。
//!
//! ## 为什么要有这一套
//!
//! `au4a_core::CoreError` 是**冻结基元层**的错误，只该表达编码 / 签名 / 账本这类与语义无关的失败。
//! 但"未知 Agent""重复注册""质押不足""证据闸门拒绝"都是**宿主内核的策略语义**，放在基元层有两个实际代价：
//!
//! 1. **污染**：内核要表达新的策略拒绝，就必须改冻结基元（违反"只增不改"的成本模型）；
//! 2. **语义重载**：`InsufficientFunds` 被复用为"证据闸门拒绝"，
//!    调用方**无法区分**"余额不够"与"证据等级不够"，日志、重试与告警都会做错。
//!
//! ## 迁移分四段（本文件是 S1）
//!
//! | 段 | 内容 | 状态 |
//! |---|---|---|
//! | **S1** | 新增 [`KernelError`]/[`KernelResult`]、兼容双向映射、`Kernel::settle_checked`（闸门拒绝有独立变体） | ✅ 本段 |
//! | S2 | `au4a-core` 的 3 个内核语义变体标 `#[deprecated]` 并写迁移说明 | ⏳ |
//! | S3 | 逐 crate 迁移 195 处调用点（同时做语义改判），一次一个 crate、一次一轮 workspace 测试 | ⏳ |
//! | S4 | 删除已弃用变体；`From<KernelError> for CoreError` 收紧，`EvidenceGateRefused` **不再**降级 | ⏳ |
//!
//! **S1 不改变任何既有公开签名**，因此 195 处旧调用点继续编译、继续通过测试；
//! 需要区分"证据不够"与"没钱"的调用方改用 [`crate::Kernel::settle_checked`] 即可。

use std::fmt;

use au4a_core::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};

/// 内核语义错误。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelError {
    /// 未知 Agent（内核注册表里没有这个 DID）。
    UnknownAgent,
    /// 重复注册。
    DuplicateAgent,
    /// 质押低于网络 `min_stake`。
    InsufficientStake,
    /// **证据闸门拒绝结算**：与"余额不足"是两件事，因此必须是独立变体。
    ///
    /// 触发条件：`EvidenceGrade::Unverified`（永不可结算），
    /// 或 `CpuProto` 且金额超过 `KernelConfig::cpu_proto_settle_cap`。
    EvidenceGateRefused,
    /// 来自冻结基元层的错误（编码 / 签名 / 账本 …）。
    Core(CoreError),
}

impl KernelError {
    /// 稳定短名：日志、观察面板与拒绝码映射都用它，改名即为接口变更。
    pub fn code(&self) -> &'static str {
        match self {
            KernelError::UnknownAgent => "unknown_agent",
            KernelError::DuplicateAgent => "duplicate_agent",
            KernelError::InsufficientStake => "insufficient_stake",
            KernelError::EvidenceGateRefused => "evidence_gate_refused",
            KernelError::Core(e) => core_code(e),
        }
    }

    /// 是否是**证据闸门**拒绝（迁移期给旧调用点做语义补判的唯一正确姿势）。
    pub fn is_evidence_gate(&self) -> bool {
        matches!(self, KernelError::EvidenceGateRefused)
    }

    /// 取出底层基元错误（若来自基元层）。
    pub fn as_core(&self) -> Option<&CoreError> {
        match self {
            KernelError::Core(e) => Some(e),
            _ => None,
        }
    }
}

/// 基元错误的稳定短名（与 `au4a-safety` 的连线口径保持一致）。
fn core_code(e: &CoreError) -> &'static str {
    match e {
        CoreError::InvalidDid => "invalid_did",
        CoreError::InvalidSignature => "invalid_signature",
        CoreError::FloatForbidden => "float_forbidden",
        CoreError::Encoding => "encoding",
        CoreError::NegativeAmount => "negative_amount",
        CoreError::ZeroAmount => "zero_amount",
        CoreError::Overflow => "overflow",
        CoreError::InsufficientFunds => "insufficient_funds",
        CoreError::InsufficientStake => "insufficient_stake",
        CoreError::UnknownAgent => "unknown_agent",
        CoreError::DuplicateAgent => "duplicate_agent",
        CoreError::InvalidKind => "invalid_kind",
        CoreError::FrameTooLarge => "frame_too_large",
        CoreError::FrameTruncated => "frame_truncated",
        CoreError::NotSealed => "not_sealed",
        CoreError::InvalidVersion => "invalid_version",
    }
}

impl fmt::Display for KernelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KernelError::UnknownAgent => f.write_str("unknown agent"),
            KernelError::DuplicateAgent => f.write_str("duplicate agent"),
            KernelError::InsufficientStake => f.write_str("insufficient stake"),
            KernelError::EvidenceGateRefused => f.write_str("evidence gate refused settlement"),
            KernelError::Core(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for KernelError {}

impl From<CoreError> for KernelError {
    fn from(e: CoreError) -> Self {
        KernelError::Core(e)
    }
}

/// **迁移期兼容边界**（S1–S3 期间存在，S4 收紧）。
///
/// 把内核错误降级回基元错误，好让 195 处旧调用点继续编译。注意这里
/// `EvidenceGateRefused` 被降级成 `InsufficientFunds` —— 这正是**语义重载本身**。
/// 需要区分二者时，请直接用 [`KernelError`] / [`crate::Kernel::settle_checked`]，
/// 不要依赖这条降级路径。
impl From<KernelError> for CoreError {
    fn from(e: KernelError) -> Self {
        match e {
            KernelError::UnknownAgent => CoreError::UnknownAgent,
            KernelError::DuplicateAgent => CoreError::DuplicateAgent,
            KernelError::InsufficientStake => CoreError::InsufficientStake,
            KernelError::EvidenceGateRefused => CoreError::InsufficientFunds,
            KernelError::Core(e) => e,
        }
    }
}

/// 内核层结果别名。
pub type KernelResult<T> = Result<T, KernelError>;

/// 把基元结果提升为内核结果（批量迁移时用）。
pub fn lift<T>(r: CoreResult<T>) -> KernelResult<T> {
    r.map_err(KernelError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_refusal_is_a_distinct_variant_not_insufficient_funds() {
        // S1 的核心断言：在新类型里，"证据不够"与"没钱"是**两个不同的值**。
        let gate = KernelError::EvidenceGateRefused;
        let broke = KernelError::from(CoreError::InsufficientFunds);
        assert_ne!(gate, broke, "证据闸门拒绝不得等于余额不足");
        assert!(gate.is_evidence_gate());
        assert!(!broke.is_evidence_gate());
        assert_eq!(gate.code(), "evidence_gate_refused");
        assert_eq!(broke.code(), "insufficient_funds");
        // 反向：来自基元的"没钱"不会被误判成闸门拒绝
        assert_eq!(broke.as_core(), Some(&CoreError::InsufficientFunds));
        assert_eq!(gate.as_core(), None);
    }

    #[test]
    fn legacy_boundary_downgrades_gate_to_insufficient_funds() {
        // 迁移期兼容边界的**已知行为**：降级后两者在基元层不可区分。
        // 这条测试把"已知缺陷"钉住：S4 收紧后它会失败，届时必须一起改（这是刻意的绊线）。
        let downgraded: CoreError = KernelError::EvidenceGateRefused.into();
        assert_eq!(downgraded, CoreError::InsufficientFunds);
        assert_eq!(
            CoreError::from(KernelError::UnknownAgent),
            CoreError::UnknownAgent
        );
        assert_eq!(
            CoreError::from(KernelError::InsufficientStake),
            CoreError::InsufficientStake
        );
        assert_eq!(
            CoreError::from(KernelError::DuplicateAgent),
            CoreError::DuplicateAgent
        );
    }

    #[test]
    fn every_variant_roundtrips_through_its_core_counterpart() {
        for e in [
            CoreError::InvalidDid,
            CoreError::InvalidSignature,
            CoreError::FloatForbidden,
            CoreError::Encoding,
            CoreError::NegativeAmount,
            CoreError::ZeroAmount,
            CoreError::Overflow,
            CoreError::InsufficientFunds,
            CoreError::InsufficientStake,
            CoreError::UnknownAgent,
            CoreError::DuplicateAgent,
            CoreError::InvalidKind,
            CoreError::FrameTooLarge,
            CoreError::FrameTruncated,
            CoreError::NotSealed,
            CoreError::InvalidVersion,
        ] {
            let k = KernelError::from(e.clone());
            assert_eq!(k.as_core(), Some(&e), "基元错误必须原样保留在 Core 变体里");
            assert!(!k.code().is_empty());
            let back: CoreError = k.into();
            assert_eq!(back, e, "Core 变体必须无损往返");
        }
    }

    #[test]
    fn code_names_are_stable_and_unique() {
        let codes = [
            KernelError::UnknownAgent.code(),
            KernelError::DuplicateAgent.code(),
            KernelError::InsufficientStake.code(),
            KernelError::EvidenceGateRefused.code(),
        ];
        assert_eq!(
            codes,
            [
                "unknown_agent",
                "duplicate_agent",
                "insufficient_stake",
                "evidence_gate_refused"
            ]
        );
        let mut sorted = codes.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), codes.len(), "短名必须互不重复");
    }

    #[test]
    fn lift_preserves_core_errors() {
        let ok: CoreResult<u8> = Ok(7);
        assert_eq!(lift(ok), Ok(7));
        let err: CoreResult<u8> = Err(CoreError::NotSealed);
        assert_eq!(lift(err), Err(KernelError::Core(CoreError::NotSealed)));
    }
}
