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

// v2.1.0 修复（P0）：删掉 `PartialOrd, Ord` 派生——按声明顺序会得到
// `Verified < CpuProto < Unverified`，与"值越大越可信"完全相反，
// 任何 `max()`/排序都会选到**最不可信**的那一级。改为显式实现，秩：Unverified=0 < CpuProto=1 < Verified=2。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceGrade {
    Verified,
    CpuProto,
    Unverified,
}

impl EvidenceGrade {
    /// 可信度秩：**数值越大越可信**。`Ord` 与结算闸门都以它为准，二者不会再分叉。
    pub fn rank(self) -> u8 {
        match self {
            EvidenceGrade::Verified => 2,
            EvidenceGrade::CpuProto => 1,
            EvidenceGrade::Unverified => 0,
        }
    }

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
    /// * `Verified` 总是可结算（除负额）。
    /// * `CpuProto` 只能结算不超过阈值的小额——原型够用，但不足以承载大额信任。
    /// * `Unverified` 一律拒绝，无论金额多小。
    ///
    /// v2.1.0 修复（P1）：首行**拒绝负额**。`Credits` 虽是 `pub struct Credits(pub i64)`，
    /// 元组构造可以绕过 `Credits::new` 的非负校验；负数在下溢包装后会落进
    /// `amount <= threshold` 的"小额"分支被放行。负额在任何等级下都不可结算。
    pub fn settleable(self, amount: Credits, threshold: Credits) -> bool {
        if amount.0 < 0 {
            return false;
        }
        match self {
            EvidenceGrade::Verified => true,
            EvidenceGrade::CpuProto => amount <= threshold,
            EvidenceGrade::Unverified => false,
        }
    }
}

impl PartialOrd for EvidenceGrade {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for EvidenceGrade {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank().cmp(&other.rank())
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

    // ── v2.1.0 回归测试 ──────────────────────────────────────────────────────

    #[test]
    fn ord_direction_matches_trust_not_declaration_order() {
        // P0 回归：声明顺序是 Verified, CpuProto, Unverified。
        // 修复前 Ord 按声明顺序给出 Verified < Unverified，max() 会选出**最不可信**的一级。
        assert!(EvidenceGrade::Verified > EvidenceGrade::CpuProto);
        assert!(EvidenceGrade::CpuProto > EvidenceGrade::Unverified);
        assert!(EvidenceGrade::Verified > EvidenceGrade::Unverified);
        let best = [
            EvidenceGrade::Unverified,
            EvidenceGrade::Verified,
            EvidenceGrade::CpuProto,
        ]
        .into_iter()
        .max()
        .expect("数组非空");
        assert_eq!(best, EvidenceGrade::Verified, "max() 必须选到最可信的一级");
        assert_eq!(EvidenceGrade::Verified.rank(), 2);
        assert_eq!(EvidenceGrade::CpuProto.rank(), 1);
        assert_eq!(EvidenceGrade::Unverified.rank(), 0);
    }

    #[test]
    fn negative_amount_is_never_settleable() {
        // P1 回归：`Credits(pub i64)` 的元组构造可绕过 Credits::new 的非负校验，
        // 负额在下溢包装后会落进 CpuProto 的「小额」分支；闸门必须自己拒绝。
        let negative = Credits(-1);
        assert!(!EvidenceGrade::Unverified.settleable(negative, Credits(1_000_000)));
        assert!(!EvidenceGrade::CpuProto.settleable(negative, Credits(1_000_000)));
        assert!(!EvidenceGrade::Verified.settleable(negative, Credits(1_000_000)));
        assert!(!EvidenceGrade::CpuProto.settleable(Credits(i64::MIN), Credits(i64::MAX)));
        // 零额与正额行为不变，确认修复没有过度收紧
        assert!(EvidenceGrade::Verified.settleable(Credits(0), Credits(0)));
        assert!(EvidenceGrade::CpuProto.settleable(Credits(0), Credits(0)));
    }
}
