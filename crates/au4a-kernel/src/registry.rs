//! Agent 注册表（v1.0.1 宿主内核重构）。
//!
//! 为什么单独成模块：在此之前「谁注册了」「谁声明了哪个能力」只能靠遍历一张
//! `BTreeMap<Did, AgentCard>` 回答，而 `BTreeMap` 的序是**字节序**、不是**加入序**。
//! 自治网络需要内核自己回答「某个能力由哪些 Agent 声明」并且答案必须可复现：
//! 同一串注册动作 → 同一个指纹。否则重放验证会退化成一堆无法比对的日志。
//!
//! 因此本模块显式保留注册顺序（`order`），并为能力建倒排索引（`skills`）。
//! 索引是**派生数据**：它必须与名片集合保持一致，这一不变式由
//! `crate::audit::audit_kernel` 的 `registry.skills_indexed` 断言。

use std::collections::BTreeMap;

use au4a_core::{canonical_hash, CoreError, CoreResult, Credits, Did};
use serde::{Deserialize, Serialize};

use crate::AgentCard;

/// 注册顺序 + 能力倒排索引。
#[derive(Clone, Debug, Default)]
pub struct AgentRegistry {
    /// 加入顺序。指纹与能力查询都按这个序输出，保证可复现。
    order: Vec<Did>,
    /// 身份 → 名片。
    cards: BTreeMap<Did, AgentCard>,
    /// 能力 → 声明者（按加入序）。
    skills: BTreeMap<String, Vec<Did>>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn contains(&self, did: &Did) -> bool {
        self.cards.contains_key(did)
    }

    pub fn get(&self, did: &Did) -> Option<&AgentCard> {
        self.cards.get(did)
    }

    /// 可变取用（仅自测：审计需要构造「被篡改的注册表」）。
    #[cfg(test)]
    pub(crate) fn card_mut(&mut self, did: &Did) -> Option<&mut AgentCard> {
        self.cards.get_mut(did)
    }

    /// 破坏倒排索引，仅供审计自测使用。
    #[cfg(test)]
    pub(crate) fn break_index(&mut self) {
        self.skills.clear();
    }

    /// 加入一个名片。重复身份是调用错误（`DuplicateAgent`），不是竞争。
    pub fn insert(&mut self, card: AgentCard) -> CoreResult<()> {
        if self.cards.contains_key(&card.did) {
            return Err(CoreError::DuplicateAgent);
        }
        for skill in &card.skills {
            self.skills
                .entry(skill.clone())
                .or_default()
                .push(card.did.clone());
        }
        self.order.push(card.did.clone());
        self.cards.insert(card.did.clone(), card);
        Ok(())
    }

    /// 按加入序迭代名片。
    pub fn cards(&self) -> impl Iterator<Item = &AgentCard> {
        self.order.iter().filter_map(|did| self.cards.get(did))
    }

    /// 按加入序迭代身份。
    pub fn dids(&self) -> impl Iterator<Item = &Did> {
        self.order.iter()
    }

    /// 声明了某能力的 Agent（按加入序）。能力路由的第一跳就是它。
    pub fn agents_with_skill(&self, skill: &str) -> Vec<&AgentCard> {
        self.skills
            .get(skill)
            .map(|dids| dids.iter().filter_map(|d| self.cards.get(d)).collect())
            .unwrap_or_default()
    }

    /// 只读的能力倒排索引。
    pub fn skill_index(&self) -> &BTreeMap<String, Vec<Did>> {
        &self.skills
    }

    /// 自查：倒排索引是否是名片集合的忠实派生（audit 用它做不变式断言）。
    pub fn index_is_faithful(&self) -> bool {
        let mut expected: BTreeMap<String, Vec<Did>> = BTreeMap::new();
        for card in self.cards() {
            for skill in &card.skills {
                expected
                    .entry(skill.clone())
                    .or_default()
                    .push(card.did.clone());
            }
        }
        expected == self.skills
    }

    /// 同一张名片内重复声明的能力（会稀释能力图权重，属于准入瑕疵）。
    pub fn duplicate_skills(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for card in self.cards() {
            let mut seen: Vec<&String> = Vec::new();
            for skill in &card.skills {
                if seen.contains(&skill) {
                    out.push((card.did.as_str().to_string(), skill.clone()));
                } else {
                    seen.push(skill);
                }
            }
        }
        out
    }

    /// 已声明质押总额（整数运算，溢出即错误而不是回绕）。
    pub fn stake_total(&self) -> CoreResult<Credits> {
        self.cards()
            .try_fold(Credits::ZERO, |acc, card| acc.checked_add(card.stake))
    }

    /// 可复现快照（内容寻址输入）。
    pub fn snapshot(&self) -> RegistrySnapshot {
        RegistrySnapshot {
            dids: self.order.iter().map(|d| d.as_str().to_string()).collect(),
            skills: self
                .skills
                .iter()
                .map(|(skill, dids)| {
                    (
                        skill.clone(),
                        dids.iter().map(|d| d.as_str().to_string()).collect(),
                    )
                })
                .collect(),
        }
    }

    /// 注册表指纹：同一串注册动作 → 同一个值。
    pub fn fingerprint(&self) -> CoreResult<String> {
        self.snapshot().fingerprint()
    }
}

/// 注册表的只读投影。指纹建立在它上面，而不是建立在内部容器布局上。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawSnapshot", into = "RawSnapshot")]
pub struct RegistrySnapshot {
    /// 加入序的 DID。
    pub dids: Vec<String>,
    /// 能力 → 声明者（加入序）。
    pub skills: BTreeMap<String, Vec<String>>,
}

/// 线上形态（字段一一对应，JSON 形状不变）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawSnapshot {
    pub dids: Vec<String>,
    pub skills: BTreeMap<String, Vec<String>>,
}

/// v2.4.0 修复（P1）：**快照反序列化必须校验一致性**。
///
/// 修复前 `RegistrySnapshot` 只是 `derive(Deserialize)` + 全 `pub` 字段，
/// 于是 `dids` 与 `skills` 可以互相矛盾（索引里有 `dids` 未列的 DID、或某 DID 从未在索引里出现），
/// 而它**照样能算出指纹**——即"不忠实的状态也有合法指纹"。这与 v2.2.0（`Did`/`Refusal`）、
/// v2.3.0（`MsgKind`）是同一模式：*派生数据的入口绕过不变式*。
///
/// 忠实性无法只靠快照自身判定（需要名片集合），但**一致性**可以，而且这正是跨节点比对的前提：
/// * 每个 DID 都必须能解析（拒绝脏值，与 v2.2.0 的 `Did` 校验同一口径）；
/// * `dids` 不得重复（加入序是语义的一部分）；
/// * `skills` 里出现的每个 DID 都必须在 `dids` 里（否则索引指向不存在的人）。
impl TryFrom<RawSnapshot> for RegistrySnapshot {
    type Error = CoreError;

    fn try_from(raw: RawSnapshot) -> CoreResult<Self> {
        let mut seen: Vec<&str> = Vec::with_capacity(raw.dids.len());
        for did in &raw.dids {
            Did::parse(did)?;
            if seen.contains(&did.as_str()) {
                return Err(CoreError::DuplicateAgent);
            }
            seen.push(did);
        }
        for (skill, holders) in &raw.skills {
            if skill.is_empty() {
                return Err(CoreError::InvalidKind);
            }
            for holder in holders {
                if !seen.contains(&holder.as_str()) {
                    return Err(CoreError::UnknownAgent);
                }
            }
        }
        Ok(RegistrySnapshot {
            dids: raw.dids,
            skills: raw.skills,
        })
    }
}

impl From<RegistrySnapshot> for RawSnapshot {
    fn from(s: RegistrySnapshot) -> Self {
        RawSnapshot {
            dids: s.dids,
            skills: s.skills,
        }
    }
}

impl RegistrySnapshot {
    pub fn fingerprint(&self) -> CoreResult<String> {
        let value = serde_json::to_value(self).map_err(|_| CoreError::Encoding)?;
        canonical_hash(&value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use au4a_core::{AgentKeys, EvidenceGrade};

    fn card(seed: u8, display: &str, skills: &[&str], stake: i64) -> AgentCard {
        let keys = AgentKeys::from_seed(&[seed; 32]);
        AgentCard {
            did: keys.did(),
            display: display.to_string(),
            skills: skills.iter().map(|s| (*s).to_string()).collect(),
            stake: Credits(stake),
            evidence: EvidenceGrade::Verified,
        }
    }

    #[test]
    fn duplicate_identity_is_a_call_error() {
        let mut reg = AgentRegistry::new();
        reg.insert(card(1, "a", &["x"], 10)).unwrap();
        assert_eq!(
            reg.insert(card(1, "a", &["x"], 10)),
            Err(CoreError::DuplicateAgent)
        );
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn same_action_sequence_yields_the_same_fingerprint() {
        let mut a = AgentRegistry::new();
        let mut b = AgentRegistry::new();
        for (seed, skill) in [(1u8, "x"), (2, "y"), (3, "x")] {
            a.insert(card(seed, "n", &[skill], 10)).unwrap();
            b.insert(card(seed, "n", &[skill], 10)).unwrap();
        }
        assert_eq!(a.fingerprint().unwrap(), b.fingerprint().unwrap());
        // 加入序是语义的一部分：换序必须换指纹，否则重放无法发现顺序错乱。
        let mut c = AgentRegistry::new();
        for (seed, skill) in [(3u8, "x"), (2, "y"), (1, "x")] {
            c.insert(card(seed, "n", &[skill], 10)).unwrap();
        }
        assert_ne!(a.fingerprint().unwrap(), c.fingerprint().unwrap());
    }

    #[test]
    fn skill_index_follows_registration_order_and_stays_faithful() {
        let mut reg = AgentRegistry::new();
        reg.insert(card(1, "first", &["translate"], 10)).unwrap();
        reg.insert(card(2, "second", &["translate", "audit"], 10))
            .unwrap();
        let holders: Vec<&str> = reg
            .agents_with_skill("translate")
            .iter()
            .map(|c| c.display.as_str())
            .collect();
        assert_eq!(holders, vec!["first", "second"]);
        assert_eq!(reg.agents_with_skill("audit").len(), 1);
        assert!(reg.agents_with_skill("nonexistent").is_empty());
        assert!(reg.index_is_faithful());
    }

    #[test]
    fn duplicate_skill_declarations_are_reported() {
        let mut reg = AgentRegistry::new();
        reg.insert(card(1, "a", &["x", "x", "y"], 10)).unwrap();
        assert_eq!(reg.duplicate_skills().len(), 1);
        assert!(reg.duplicate_skills()[0].1 == "x");
        assert_eq!(reg.agents_with_skill("x").len(), 2, "索引如实记录重复声明");
    }

    #[test]
    fn stake_total_is_integer_and_order_independent() {
        let mut reg = AgentRegistry::new();
        reg.insert(card(1, "a", &[], 10)).unwrap();
        reg.insert(card(2, "b", &[], 32)).unwrap();
        assert_eq!(reg.stake_total().unwrap(), Credits(42));
    }

    // ── v2.4.0 回归测试 ──────────────────────────────────────────────────────

    #[test]
    fn inconsistent_snapshot_is_refused_by_serde() {
        // P1 回归：修复前 dids 与 skills 可以互相矛盾，而且照样能算出指纹。
        let d1 = AgentKeys::from_seed(&[1u8; 32]).did().as_str().to_string();
        let d2 = AgentKeys::from_seed(&[2u8; 32]).did().as_str().to_string();

        // 合法快照：往返且指纹稳定
        let ok = format!(r#"{{"dids":["{d1}","{d2}"],"skills":{{"x":["{d1}"]}}}}"#);
        let snap: RegistrySnapshot = serde_json::from_str(&ok).unwrap();
        assert_eq!(snap.dids.len(), 2);
        let json = serde_json::to_string(&snap).unwrap();
        assert_eq!(
            serde_json::from_str::<RegistrySnapshot>(&json).unwrap(),
            snap
        );

        // ① 索引指向不在册的 DID → 拒绝
        let outsider = format!(r#"{{"dids":["{d1}"],"skills":{{"x":["{d2}"]}}}}"#);
        assert!(serde_json::from_str::<RegistrySnapshot>(&outsider).is_err());

        // ② 重复 DID → 拒绝（加入序是语义的一部分）
        let dup = format!(r#"{{"dids":["{d1}","{d1}"],"skills":{{}}}}"#);
        assert!(serde_json::from_str::<RegistrySnapshot>(&dup).is_err());

        // ③ 脏 DID → 拒绝（与 v2.2.0 的 Did 校验同口径）
        let dirty = r#"{"dids":["did:au4a:zz"],"skills":{}}"#;
        assert!(serde_json::from_str::<RegistrySnapshot>(dirty).is_err());

        // ④ 空能力名 → 拒绝
        let empty_skill = format!(r#"{{"dids":["{d1}"],"skills":{{"":["{d1}"]}}}}"#);
        assert!(serde_json::from_str::<RegistrySnapshot>(&empty_skill).is_err());
    }
}
