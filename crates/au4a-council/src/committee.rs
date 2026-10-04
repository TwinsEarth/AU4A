//! 委员会与席位（v1.7.1）。
//!
//! 五类委员会是**封闭枚举**而不是字符串：新增一类委员会会改变治理拓扑，属于协议变更。
//! 每类委员会的职责边界写在 [`CommitteeKind::mandate`] 里，于是「安全委员会能不能替资源
//! 委员会定价」这类问题在类型层面就有答案，而不是靠运行时的君子协定。
//!
//! 容错数学（BFT-lite）也放在这里：`f = ⌊(n-1)/3⌋`，法定人数 `quorum = n - f`。
//! 对 `n = 3f+1` 的规模，`quorum = 2f+1`；对更小的规模，`f = 0`，即要求全员一致。

use au4a_core::Did;
use serde::{Deserialize, Serialize};

/// 委员会数量。
pub const COMMITTEE_COUNT: usize = 5;

/// 五类委员会。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitteeKind {
    /// 资源委员会：算力、额度与质押参数。
    Resource,
    /// 任务委员会：任务准入与结算规则。
    Task,
    /// 仲裁委员会：争议裁决与罚没建议。
    Arbitration,
    /// 进化委员会：能力演进与模型更新。
    Evolution,
    /// 安全委员会：唯一持有紧急通道的委员会。
    Security,
}

impl CommitteeKind {
    /// 全部五类，顺序固定（确定性遍历的基础）。
    pub const ALL: [CommitteeKind; COMMITTEE_COUNT] = [
        CommitteeKind::Resource,
        CommitteeKind::Task,
        CommitteeKind::Arbitration,
        CommitteeKind::Evolution,
        CommitteeKind::Security,
    ];

    /// 稳定字符串（进签名载荷与审计日志，不可随意改）。
    pub fn as_str(self) -> &'static str {
        match self {
            CommitteeKind::Resource => "resource",
            CommitteeKind::Task => "task",
            CommitteeKind::Arbitration => "arbitration",
            CommitteeKind::Evolution => "evolution",
            CommitteeKind::Security => "security",
        }
    }

    /// 解析稳定字符串。
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// 中文名（人类只读投影里直接使用）。
    pub fn title(self) -> &'static str {
        match self {
            CommitteeKind::Resource => "资源委员会",
            CommitteeKind::Task => "任务委员会",
            CommitteeKind::Arbitration => "仲裁委员会",
            CommitteeKind::Evolution => "进化委员会",
            CommitteeKind::Security => "安全委员会",
        }
    }

    /// 职责边界：这个委员会**只能**做什么。
    pub fn mandate(self) -> &'static str {
        match self {
            CommitteeKind::Resource => "算力/额度/质押参数",
            CommitteeKind::Task => "任务准入与结算规则",
            CommitteeKind::Arbitration => "争议裁决",
            CommitteeKind::Evolution => "能力与模型演进",
            CommitteeKind::Security => "安全底线与紧急策略",
        }
    }

    /// 是否持有紧急通道——只有安全委员会有。
    pub fn has_emergency_channel(self) -> bool {
        matches!(self, CommitteeKind::Security)
    }
}

/// 一名当选成员。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    /// 成员身份。
    pub did: Did,
    /// 加权票排名（从 1 开始）。
    pub rank: u16,
    /// 加权票得分（Σ 合格选民权重）。
    pub score: u64,
    /// 当选时的信誉（万分比）。
    pub reputation_bp: u32,
    /// 当选时的在线时长（逻辑刻度）。
    pub uptime: u64,
    /// 任期结束的逻辑时刻。
    pub term_ends_at: u64,
}

/// 一届委员会。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Committee {
    /// 类别。
    pub kind: CommitteeKind,
    /// 届数（同类委员会每改选一次 +1）。
    pub epoch: u64,
    /// 席位上限。
    pub seats: usize,
    /// 改选发生的逻辑时刻。
    pub elected_at: u64,
    /// 产生这届委员会的选举内容地址。
    pub election_id: String,
    /// 成员，按 rank 升序。
    pub members: Vec<Member>,
}

impl Committee {
    /// 实际到任人数。
    pub fn size(&self) -> usize {
        self.members.len()
    }

    /// 无成员。
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// 按 DID 取成员。
    pub fn member(&self, did: &Did) -> Option<&Member> {
        self.members.iter().find(|m| &m.did == did)
    }

    /// 是否为在任成员。**所有表决准入都走这里**。
    pub fn has_member(&self, did: &Did) -> bool {
        self.member(did).is_some()
    }

    /// BFT-lite 容错上限 `f = ⌊(n-1)/3⌋`，满足 `n ≥ 3f+1`。
    ///
    /// 空委员会 `f = 0`，但这不代表「零容错」而是「无表决资格」，见 [`Committee::is_bft_consistent`]。
    pub fn fault_bound(&self) -> usize {
        self.size().saturating_sub(1) / 3
    }

    /// 法定人数 `n - f`：对 `n = 3f+1` 即 `2f+1`。
    pub fn quorum(&self) -> usize {
        self.size().saturating_sub(self.fault_bound())
    }

    /// 是否有表决资格：`n ≥ 3f+1` 且 `quorum = n - f`。
    ///
    /// **空委员会刻意判为不合格**：否则 `quorum = 0` 会让「0 票 ≥ 0」的决议自动通过——
    /// 这是治理系统里最容易出现、后果最严重的一类静默漏洞。
    pub fn is_bft_consistent(&self) -> bool {
        let n = self.size();
        let f = self.fault_bound();
        n > 3 * f && n > 0 && self.quorum() == n - f
    }

    /// 在任成员 DID 列表（rank 升序，确定性）。
    pub fn member_dids(&self) -> Vec<Did> {
        self.members.iter().map(|m| m.did.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use au4a_core::AgentKeys;

    fn member(seed: u8, rank: u16) -> Member {
        Member {
            did: AgentKeys::from_seed(&[seed; 32]).did(),
            rank,
            score: u64::from(rank),
            reputation_bp: 5_000,
            uptime: 100,
            term_ends_at: 1_000,
        }
    }

    #[test]
    fn five_kinds_roundtrip_through_stable_names() {
        assert_eq!(CommitteeKind::ALL.len(), COMMITTEE_COUNT);
        for k in CommitteeKind::ALL {
            assert_eq!(CommitteeKind::parse(k.as_str()), Some(k));
            assert!(!k.title().is_empty());
            assert!(!k.mandate().is_empty());
        }
        assert_eq!(CommitteeKind::parse("treasury"), None);
    }

    #[test]
    fn only_security_has_the_emergency_channel() {
        let holders: Vec<&str> = CommitteeKind::ALL
            .iter()
            .filter(|k| k.has_emergency_channel())
            .map(|k| k.as_str())
            .collect();
        assert_eq!(holders, vec!["security"]);
    }

    #[test]
    fn quorum_math_holds_for_bft_sizes() {
        for n in [1usize, 2, 3, 4, 5, 7, 10, 13] {
            let c = Committee {
                kind: CommitteeKind::Task,
                epoch: 1,
                seats: n,
                elected_at: 0,
                election_id: String::from("x"),
                members: (0..n as u8).map(|i| member(i, i as u16 + 1)).collect(),
            };
            assert!(c.is_bft_consistent(), "n={n}");
            let f = c.fault_bound();
            assert!(n > 3 * f, "n={n} f={f}");
            assert_eq!(c.quorum(), n - f);
            if n == 3 * f + 1 {
                assert_eq!(c.quorum(), 2 * f + 1, "n={n}");
            }
        }
    }

    #[test]
    fn an_empty_committee_has_no_voting_capacity() {
        let c = Committee {
            kind: CommitteeKind::Task,
            epoch: 1,
            seats: 5,
            elected_at: 0,
            election_id: String::from("x"),
            members: Vec::new(),
        };
        assert_eq!(c.quorum(), 0);
        // quorum = 0 会让「0 票 ≥ 0」自动通过，因此空委员会必须判为不合格。
        assert!(!c.is_bft_consistent());
    }

    #[test]
    fn membership_lookup_is_by_did() {
        let c = Committee {
            kind: CommitteeKind::Arbitration,
            epoch: 3,
            seats: 5,
            elected_at: 7,
            election_id: String::from("e"),
            members: vec![member(1, 1), member(2, 2)],
        };
        assert!(c.has_member(&member(1, 1).did));
        assert!(!c.has_member(&member(9, 9).did));
        assert_eq!(c.member_dids().len(), 2);
        assert_eq!(c.fault_bound(), 0);
        assert_eq!(c.quorum(), 2);
    }
}
