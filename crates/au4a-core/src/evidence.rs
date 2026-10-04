//! 证据分级——结算闸门。
//!
//! 参考项目的失败模式不是「没有功能」，而是「文档声称的能力在代码里不可达」，于是
//! 结算建立在无法验证的声明上。AU4A 的规则很简单：**每一条能力必须自报证据等级，
//! 而 `unverified` 永远不可结算**。
//!
//! * [`EvidenceGrade::Verified`]：本机真实执行并留下可复现证据（测试/实测日志）。
//! * [`EvidenceGrade::CpuProto`]：纯 CPU 语义原型，语义正确但没有真实网络/链。
//! * [`EvidenceGrade::Unverified`]：只有文档或设计，不可用于任何结算或对外承诺。

use serde::{Deserialize, Serialize};

use crate::ledger::Credits;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceGrade {
    Verified,
    CpuProto,
    Unverified,
}

impl EvidenceGrade {
    pub fn as_str(self) -> &'static str {
        match self {
            EvidenceGrade::Verified => "verified",
            EvidenceGrade::CpuProto => "cpu-proto",
            EvidenceGrade::Unverified => "unverified",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "verified" => Some(EvidenceGrade::Verified),
            "cpu-proto" => Some(EvidenceGrade::CpuProto),
            "unverified" => Some(EvidenceGrade::Unverified),
            _ => None,
        }
    }

    /// 结算闸门：值越大越可信。`threshold` 是「原型允许结算的上限」。
    ///
    /// * `Verified` 总是可结算。
    /// * `CpuProto` 只能结算不超过阈值的小额——原型够用，但不足以承载大额信任。
    /// * `Unverified` 一律拒绝，无论金额多小。
    pub fn settleable(self, amount: Credits, threshold: Credits) -> bool {
        match self {
            EvidenceGrade::Verified => true,
            EvidenceGrade::CpuProto => amount <= threshold,
            EvidenceGrade::Unverified => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unverified_never_settles() {
        let g = EvidenceGrade::Unverified;
        assert!(!g.settleable(Credits(0), Credits(1_000_000)));
        assert!(!g.settleable(Credits(1), Credits(1_000_000)));
    }

    #[test]
    fn cpu_proto_is_capped() {
        let g = EvidenceGrade::CpuProto;
        assert!(g.settleable(Credits(100), Credits(100)));
        assert!(!g.settleable(Credits(101), Credits(100)));
    }

    #[test]
    fn verified_has_no_cap() {
        assert!(EvidenceGrade::Verified.settleable(Credits(i64::MAX), Credits(1)));
    }

    #[test]
    fn grades_roundtrip_through_strings() {
        for g in [
            EvidenceGrade::Verified,
            EvidenceGrade::CpuProto,
            EvidenceGrade::Unverified,
        ] {
            assert_eq!(EvidenceGrade::parse(g.as_str()), Some(g));
        }
        assert_eq!(EvidenceGrade::parse("probably-fine"), None);
    }
}
