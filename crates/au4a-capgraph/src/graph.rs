//! v1.1.2 —— 能力图本体（声明入口）；v1.1.4 起邻居侧由缓存层托管。
//!
//! `AgentCapabilityGraph` 是**单个 Agent 视角**的图：
//! 它的「自己」是被签名的自有声明，「别人」是它缓存下来的邻居声明。
//! 没有全局注册表——每个 Agent 各持一份视图，视图之间靠广播（v1.1.3）收敛。
//! 这正是参考项目缺的那一块：能力数据是 Agent 可读写的，而不是运营方数据库里的表。
//!
//! 每个写入口都返回 [`DeclareOutcome`] 而不是把「拒绝」折叠进 `Err`：
//! 拒绝是有类型的（[`RefusalCode`]），可以被内核记录、被观察层只读展示、
//! 也可以被上层拿去升级判定（`Kernel::escalation_for`）。这是 `au4a-core::refusal` 的既有纪律。

use au4a_core::{CoreError, Did, RefusalCode};
use serde_json::{json, Value};

use crate::cache::{CacheStats, CapabilityCache};
use crate::capability::{Capability, SkillId};
use crate::declaration::SignedDeclaration;
use crate::index::{rank_and_truncate, CapKey, CapabilityIndex, CapabilityMatch, CapabilityQuery, QueryResult, QueryStats};

/// 能力图配置。人类可以设定容量与上限，但不能设定「谁有什么能力」。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapGraphConfig {
    /// 每个账户可声明的能力条数上限（防公告轰炸）。
    pub max_skills_per_agent: usize,
    /// 邻居视图容量上限（超出后按 LRU 淘汰，而不是拒绝新信息）。
    pub neighbor_capacity: usize,
    /// 邻居条目的存活逻辑刻数；`0` 表示不过期。
    pub cache_ttl_ticks: u64,
}

impl Default for CapGraphConfig {
    fn default() -> Self {
        Self {
            max_skills_per_agent: 64,
            neighbor_capacity: 128,
            cache_ttl_ticks: 64,
        }
    }
}

/// 邻居能力的已接受记录（缓存条目）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NeighborRecord {
    pub did: Did,
    pub epoch: u64,
    /// 接受时的逻辑时刻（TTL 的基准）。
    pub at: u64,
    /// 声明内容指纹（不含签名）。
    pub fingerprint: String,
    pub capabilities: Vec<Capability>,
}

impl NeighborRecord {
    pub fn skills(&self) -> Vec<&SkillId> {
        self.capabilities.iter().map(|c| &c.skill).collect()
    }
}

/// 一次声明应用的结果。拒绝不是异常，是一种有类型的、可被记录的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeclareOutcome {
    /// 声明被接受，图发生变化。
    Applied {
        agent: Did,
        epoch: u64,
        skills: usize,
        fingerprint: String,
    },
    /// 内容与已知状态完全一致，幂等丢弃（重放同一个通告不会污染版本）。
    Unchanged { agent: Did, epoch: u64 },
    /// 被拒绝，带类型化拒绝码。
    Rejected {
        agent: Did,
        code: RefusalCode,
        reason: String,
    },
}

impl DeclareOutcome {
    pub fn applied(agent: &Did, epoch: u64, skills: usize, fingerprint: String) -> Self {
        Self::Applied {
            agent: agent.clone(),
            epoch,
            skills,
            fingerprint,
        }
    }

    pub fn unchanged(agent: &Did, epoch: u64) -> Self {
        Self::Unchanged {
            agent: agent.clone(),
            epoch,
        }
    }

    pub fn rejected(agent: &Did, code: RefusalCode, reason: impl Into<String>) -> Self {
        Self::Rejected {
            agent: agent.clone(),
            code,
            reason: reason.into(),
        }
    }

    pub fn is_applied(&self) -> bool {
        matches!(self, Self::Applied { .. })
    }

    pub fn refusal(&self) -> Option<RefusalCode> {
        match self {
            Self::Rejected { code, .. } => Some(*code),
            _ => None,
        }
    }

    /// 只读投影（观察层用）。拒绝码以 `snake_case` 字符串出现。
    pub fn to_value(&self) -> Value {
        match self {
            Self::Applied {
                agent,
                epoch,
                skills,
                fingerprint,
            } => json!({
                "outcome": "applied",
                "agent": agent.as_str(),
                "epoch": epoch,
                "skills": skills,
                "fingerprint": fingerprint,
            }),
            Self::Unchanged { agent, epoch } => json!({
                "outcome": "unchanged",
                "agent": agent.as_str(),
                "epoch": epoch,
            }),
            Self::Rejected {
                agent,
                code,
                reason,
            } => json!({
                "outcome": "rejected",
                "agent": agent.as_str(),
                "code": code.as_str(),
                "reason": reason,
            }),
        }
    }
}

/// 单个 Agent 视角的能力图。
#[derive(Clone, Debug)]
pub struct AgentCapabilityGraph {
    owner: Did,
    config: CapGraphConfig,
    own: Vec<Capability>,
    own_epoch: u64,
    own_fingerprint: Option<String>,
    neighbors: CapabilityCache,
    index: CapabilityIndex,
}

impl AgentCapabilityGraph {
    pub fn new(owner: Did, config: CapGraphConfig) -> Self {
        let neighbors = CapabilityCache::new(config.neighbor_capacity, config.cache_ttl_ticks);
        Self {
            owner,
            config,
            own: Vec::new(),
            own_epoch: 0,
            own_fingerprint: None,
            neighbors,
            index: CapabilityIndex::new(),
        }
    }

    pub fn owner(&self) -> &Did {
        &self.owner
    }

    pub fn config(&self) -> &CapGraphConfig {
        &self.config
    }

    /// 自有能力的当前版本号（v1.1.7 起由声明自动递增）。
    pub fn own_epoch(&self) -> u64 {
        self.own_epoch
    }

    pub fn own_capabilities(&self) -> &[Capability] {
        &self.own
    }

    /// 应用一份已签名声明：自己的进 `own`，别人的进邻居缓存。
    ///
    /// 顺序是刻意的：**先验签，再校验内容，最后才比较版本**。
    /// 如果先看版本，一个伪造的高版本声明就能把状态推到「有更新」，
    /// 即便它随后被拒也会在日志里留下错误的因果。
    pub fn apply(&mut self, signed: &SignedDeclaration, now: u64) -> DeclareOutcome {
        let agent = signed.agent().clone();
        if let Err(err) = signed.verify() {
            return DeclareOutcome::rejected(
                &agent,
                RefusalCode::Unauthorized,
                format!("declaration signature rejected: {err}"),
            );
        }
        if let Err(err) = signed.declaration.validate_against(self.config.max_skills_per_agent) {
            let code = match err {
                CoreError::InvalidKind => RefusalCode::PolicyDenied,
                _ => RefusalCode::Malformed,
            };
            return DeclareOutcome::rejected(&agent, code, format!("declaration rejected: {err}"));
        }
        let fingerprint = match signed.content_fingerprint() {
            Ok(fp) => fp,
            Err(err) => {
                return DeclareOutcome::rejected(
                    &agent,
                    RefusalCode::Malformed,
                    format!("declaration not canonicalisable: {err}"),
                )
            }
        };

        if agent == self.owner {
            return self.apply_own(signed, fingerprint);
        }
        self.apply_neighbor(signed, fingerprint, now)
    }

    fn apply_own(&mut self, signed: &SignedDeclaration, fingerprint: String) -> DeclareOutcome {
        let agent = self.owner.clone();
        let epoch = signed.epoch();
        if self.own_fingerprint.as_deref() == Some(fingerprint.as_str()) {
            return DeclareOutcome::unchanged(&agent, self.own_epoch);
        }
        if epoch < self.own_epoch {
            return DeclareOutcome::rejected(
                &agent,
                RefusalCode::StaleEpoch,
                format!("epoch {epoch} < current {}", self.own_epoch),
            );
        }
        if epoch == self.own_epoch && self.own_epoch != 0 {
            // 同一个版本号下内容不同：要么是发送方没有递增版本，要么是被篡改。
            // 两种情况都不能接受——接受会让「版本号 → 内容」不再是一个函数。
            return DeclareOutcome::rejected(
                &agent,
                RefusalCode::Conflict,
                format!("epoch {epoch} reused with different content"),
            );
        }
        self.own = signed.capabilities().to_vec();
        self.own_epoch = epoch;
        self.own_fingerprint = Some(fingerprint.clone());
        // 索引与数据同步：索引只记「哪里有」，因此每个写入口都必须更新它。
        self.index.insert_agent(&agent, &self.own);
        DeclareOutcome::applied(&agent, epoch, self.own.len(), fingerprint)
    }

    fn apply_neighbor(
        &mut self,
        signed: &SignedDeclaration,
        fingerprint: String,
        now: u64,
    ) -> DeclareOutcome {
        let agent = signed.agent().clone();
        let epoch = signed.epoch();
        // 只有「活着」的记录才算已知：过期记录等价于没听过，否则旧世界会永久驻留。
        let known = self
            .neighbors
            .peek(&agent)
            .filter(|_| self.neighbors.is_live(&agent, now))
            .map(|record| (record.epoch, record.fingerprint.clone()));
        if let Some((known_epoch, known_fingerprint)) = known {
            if known_fingerprint == fingerprint {
                return DeclareOutcome::unchanged(&agent, known_epoch);
            }
            if epoch < known_epoch {
                return DeclareOutcome::rejected(
                    &agent,
                    RefusalCode::StaleEpoch,
                    format!("epoch {epoch} < known {known_epoch}"),
                );
            }
            if epoch == known_epoch {
                return DeclareOutcome::rejected(
                    &agent,
                    RefusalCode::Conflict,
                    format!("epoch {epoch} reused with different content"),
                );
            }
        }
        let record = NeighborRecord {
            did: agent.clone(),
            epoch,
            at: now,
            fingerprint: fingerprint.clone(),
            capabilities: signed.capabilities().to_vec(),
        };
        let insert = self.neighbors.insert(record, now);
        if !insert.accepted {
            // 唯一会「拒绝新信息」的情形：缓存容量被配成 0（不缓存）。这是配置问题，
            // 不是竞争问题——因此用容量语义的拒绝码，而不是把它算成对端作恶。
            return DeclareOutcome::rejected(
                &agent,
                RefusalCode::ResourceExhausted,
                "neighbor cache capacity is 0",
            );
        }
        // 缓存淘汰/过期的邻居必须同时从索引里摘掉，否则查询会撞上「索引有键、图里没数据」。
        for gone in insert.evicted.iter().chain(insert.expired.iter()) {
            self.index.remove_agent(gone);
        }
        self.index.insert_agent(&agent, signed.capabilities());
        DeclareOutcome::applied(&agent, epoch, signed.capabilities().len(), fingerprint)
    }

    /// 某个 Agent 的能力（自己或邻居）。
    pub fn capabilities_of(&self, did: &Did) -> Option<&[Capability]> {
        if did == &self.owner {
            return Some(&self.own);
        }
        self.neighbors.peek(did).map(|r| r.capabilities.as_slice())
    }

    /// 邻居记录（不做过期判定、不触碰 LRU）。命中式读取用 [`Self::get_neighbor`]。
    pub fn neighbor(&self, did: &Did) -> Option<&NeighborRecord> {
        self.neighbors.peek(did)
    }

    /// 命中式读取：清理过期条目、推进 LRU、计入缓存统计。
    pub fn get_neighbor(&mut self, did: &Did, now: u64) -> Option<&NeighborRecord> {
        self.neighbors.get(did, now)
    }

    pub fn neighbors(&self) -> impl Iterator<Item = &NeighborRecord> {
        self.neighbors.iter()
    }

    pub fn neighbor_count(&self) -> usize {
        self.neighbors.len()
    }

    /// 未过期的邻居数。
    pub fn live_neighbor_count(&self, now: u64) -> usize {
        self.neighbors.live_len(now)
    }

    /// 清理过期邻居，返回被清理的 DID（字典序）。
    pub fn expire_neighbors(&mut self, now: u64) -> Vec<Did> {
        let expired = self.neighbors.expire(now);
        for did in &expired {
            self.index.remove_agent(did);
        }
        expired
    }

    /// 显式失效一个邻居。
    pub fn invalidate_neighbor(&mut self, did: &Did) -> bool {
        let removed = self.neighbors.invalidate(did);
        if removed {
            self.index.remove_agent(did);
        }
        removed
    }

    pub fn cache_stats(&self) -> CacheStats {
        self.neighbors.stats()
    }

    pub fn cache_capacity(&self) -> usize {
        self.neighbors.capacity()
    }

    /// LRU 顺序（最久未用在前）。
    pub fn lru_order(&self) -> Vec<Did> {
        self.neighbors.lru_order()
    }

    /// 把索引键解析回能力（自己走 `own`，邻居走缓存）。
    pub fn capability_at(&self, key: &CapKey) -> Option<&Capability> {
        let (did, slot) = key;
        if did == &self.owner {
            return self.own.get(*slot);
        }
        self.neighbors
            .peek(did)
            .and_then(|record| record.capabilities.get(*slot))
    }

    /// 查询：**只走倒排索引**，扫描条数由 [`QueryStats`] 如实报告。
    ///
    /// 查询前先清理过期邻居：过期数据不该出现在结果里，也不该留在索引里。
    pub fn query(&mut self, query: &CapabilityQuery, now: u64) -> QueryResult {
        let _ = self.expire_neighbors(now);
        let nodes_total = self.capability_count();
        let candidates = self
            .index
            .candidates_for(&query.skill, query.input_format.as_ref());
        let mut matches = Vec::new();
        let mut scanned = 0usize;
        for key in &candidates {
            if let Some(cap) = self.capability_at(key) {
                scanned += 1;
                if query.matches(cap) {
                    matches.push(CapabilityMatch {
                        did: key.0.clone(),
                        slot: key.1,
                        score: i64::from(cap.effective_reliability_bp()),
                        capability: cap.clone(),
                    });
                }
            }
        }
        let matched = matches.len();
        let matches = rank_and_truncate(matches, query.limit);
        QueryResult {
            matches,
            stats: QueryStats {
                candidates: candidates.len(),
                scanned,
                matched,
                nodes_total,
            },
        }
    }

    /// 「谁最适合做 X」：等价于 `limit = 1` 的查询，但意图更明确。
    pub fn best_for(&mut self, skill: &SkillId, now: u64) -> QueryResult {
        self.query(&CapabilityQuery::new(skill.clone()).with_limit(1), now)
    }

    /// 提供某技能的 Agent（升序、去重）。这也走索引。
    pub fn providers_of(&self, skill: &SkillId) -> Vec<Did> {
        let mut providers: Vec<Did> = Vec::new();
        for key in self.index.candidates_for_skill(skill) {
            if !providers.contains(&key.0) {
                providers.push(key.0);
            }
        }
        providers
    }

    /// 索引不变式：索引条目数 == 图内能力数，且每个键都能解析回**同技能**的能力。
    ///
    /// 这条断言是「索引不是猜测」的机器可检查形式：任何漏更新都会让它变成 `false`。
    pub fn index_consistent(&self) -> bool {
        if self.index.indexed_entries() != self.capability_count() {
            return false;
        }
        self.index.skill_entries().iter().all(|(skill, key)| {
            self.capability_at(key)
                .map(|cap| &cap.skill == skill)
                .unwrap_or(false)
        })
    }

    pub fn index_summary(&self) -> Value {
        self.index.to_value()
    }

    pub fn index_writes(&self) -> u64 {
        self.index.writes()
    }

    /// 路径规划（v1.1.6）：候选来自 v1.1.5 的索引，搜索是多准则标签设定。
    ///
    /// 图只负责「有哪些能力」；「怎么串起来最划算」由 [`crate::planner`] 回答。
    pub fn plan(
        &mut self,
        request: &crate::planner::PipelineRequest,
        cost: &crate::planner::PlanCost,
        now: u64,
    ) -> crate::planner::PlanOutcome {
        crate::planner::plan(self, request, cost, now)
    }

    /// 视图里的 Agent 数量（含自己）。
    pub fn known_agents(&self) -> usize {
        self.neighbors.len() + 1
    }

    /// 视图里的能力总条数（含自己）。
    pub fn capability_count(&self) -> usize {
        self.own.len() + self.neighbors.iter().map(|r| r.capabilities.len()).sum::<usize>()
    }

    /// 视图里出现过的全部技能（升序、去重）。
    pub fn skills(&self) -> Vec<&SkillId> {
        let mut skills: Vec<&SkillId> = Vec::new();
        for cap in self.own.iter().chain(self.neighbors.iter().flat_map(|r| r.capabilities.iter())) {
            if !skills.contains(&&cap.skill) {
                skills.push(&cap.skill);
            }
        }
        skills.sort();
        skills
    }

    /// 只读投影：节点 `observe` 与文档证据都用它。
    pub fn to_value(&self) -> Value {
        let neighbors: Vec<Value> = self
            .neighbors
            .iter()
            .map(|r| {
                json!({
                    "did": r.did.as_str(),
                    "epoch": r.epoch,
                    "at": r.at,
                    "skills": r.capabilities.iter().map(|c| c.skill.as_str()).collect::<Vec<_>>(),
                })
            })
            .collect();
        json!({
            "owner": self.owner.as_str(),
            "own_epoch": self.own_epoch,
            "own_skills": self.own.iter().map(|c| c.skill.as_str()).collect::<Vec<_>>(),
            "neighbors": neighbors,
            "neighbor_capacity": self.neighbors.capacity(),
            "capability_count": self.capability_count(),
            "cache": self.neighbors.stats().to_value(),
            "index": self.index.to_value(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declaration::Declaration;
    use au4a_core::{AgentKeys, Credits};

    fn keys(seed: u8) -> AgentKeys {
        AgentKeys::from_seed(&[seed; 32])
    }

    fn cap(name: &str) -> Capability {
        Capability::new(SkillId::new(name).expect("valid skill"), Credits(3))
    }

    fn signed(keys: &AgentKeys, epoch: u64, skills: &[&str]) -> SignedDeclaration {
        let caps: Vec<Capability> = skills.iter().map(|s| cap(s)).collect();
        Declaration::new(keys.did(), epoch, epoch * 10, caps)
            .expect("coherent")
            .sign(keys)
            .expect("self-signed")
    }

    #[test]
    fn own_declaration_is_applied_and_redeclaration_is_idempotent() {
        let a = keys(1);
        let mut g = AgentCapabilityGraph::new(a.did(), CapGraphConfig::default());
        let d = signed(&a, 1, &["translate.en-zh"]);
        assert!(g.apply(&d, 1).is_applied());
        assert_eq!(g.capability_count(), 1);
        assert_eq!(g.own_epoch(), 1);
        assert!(matches!(g.apply(&d, 2), DeclareOutcome::Unchanged { .. }));
        assert_eq!(g.own_epoch(), 1, "幂等重放不推进版本");
    }

    #[test]
    fn a_neighbor_declaration_lands_in_the_neighbor_view() {
        let a = keys(1);
        let b = keys(2);
        let mut g = AgentCapabilityGraph::new(a.did(), CapGraphConfig::default());
        assert!(g.apply(&signed(&b, 1, &["sentiment.analyze"]), 5).is_applied());
        assert_eq!(g.neighbor_count(), 1);
        assert_eq!(g.known_agents(), 2);
        assert!(g.capabilities_of(&b.did()).is_some());
        assert_eq!(g.neighbor(&b.did()).map(|r| r.at), Some(5));
    }

    #[test]
    fn tampering_is_rejected_as_unauthorized() {
        let a = keys(1);
        let b = keys(2);
        let mut g = AgentCapabilityGraph::new(a.did(), CapGraphConfig::default());
        let good = signed(&b, 1, &["x"]);
        let mut value = good.to_value().expect("serialisable");
        value["declaration"]["capabilities"][0]["reliability_bp"] = serde_json::json!(10_000);
        let tampered = SignedDeclaration::from_value(&value).expect("parses");
        let outcome = g.apply(&tampered, 1);
        assert_eq!(outcome.refusal(), Some(RefusalCode::Unauthorized));
        assert!(outcome.refusal().expect("code").is_misconduct());
        assert_eq!(g.neighbor_count(), 0, "被拒的声明绝不改变图");
    }

    #[test]
    fn stale_and_conflicting_epochs_are_refused_with_typed_codes() {
        let a = keys(1);
        let b = keys(2);
        let mut g = AgentCapabilityGraph::new(a.did(), CapGraphConfig::default());
        g.apply(&signed(&b, 5, &["x"]), 1);
        assert_eq!(
            g.apply(&signed(&b, 4, &["x"]), 2).refusal(),
            Some(RefusalCode::StaleEpoch)
        );
        assert_eq!(
            g.apply(&signed(&b, 5, &["y"]), 3).refusal(),
            Some(RefusalCode::Conflict)
        );
        assert_eq!(g.neighbor(&b.did()).map(|r| r.epoch), Some(5));
    }

    #[test]
    fn a_full_neighbor_view_evicts_instead_of_refusing() {
        let a = keys(1);
        let config = CapGraphConfig {
            neighbor_capacity: 2,
            ..CapGraphConfig::default()
        };
        let mut g = AgentCapabilityGraph::new(a.did(), config);
        assert!(g.apply(&signed(&keys(2), 1, &["x"]), 1).is_applied());
        assert!(g.apply(&signed(&keys(3), 1, &["x"]), 1).is_applied());
        let third = g.apply(&signed(&keys(4), 1, &["x"]), 1);
        assert!(third.is_applied(), "满了要淘汰旧条目，而不是拒绝新信息");
        assert_eq!(g.neighbor_count(), 2, "容量上限是硬约束");
        assert_eq!(g.cache_stats().evictions, 1);
    }

    #[test]
    fn an_expired_neighbor_is_treated_as_unknown() {
        let a = keys(1);
        let b = keys(2);
        let config = CapGraphConfig {
            cache_ttl_ticks: 10,
            ..CapGraphConfig::default()
        };
        let mut g = AgentCapabilityGraph::new(a.did(), config);
        assert!(g.apply(&signed(&b, 5, &["x"]), 100).is_applied());
        assert_eq!(g.live_neighbor_count(110), 1);
        // 过期后同 epoch 异内容不再是 conflict（旧世界已经不存在了），而是重新接受。
        let after = g.apply(&signed(&b, 5, &["y"]), 111);
        assert!(after.is_applied(), "{:?}", after.to_value());
        assert_eq!(g.cache_stats().expirations, 1);
    }

    #[test]
    fn skills_lists_every_provider_once() {
        let a = keys(1);
        let b = keys(2);
        let mut g = AgentCapabilityGraph::new(a.did(), CapGraphConfig::default());
        g.apply(&signed(&a, 1, &["translate.en-zh", "sentiment.analyze"]), 1);
        g.apply(&signed(&b, 1, &["translate.en-zh"]), 1);
        let skills: Vec<&str> = g.skills().iter().map(|s| s.as_str()).collect();
        assert_eq!(skills, vec!["sentiment.analyze", "translate.en-zh"]);
    }
}
