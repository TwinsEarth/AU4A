//! 确定性测试/仿真框架（v1.0.9）。
//!
//! 十条轨道共用同一个内核，验收又要求「同样的种子给同样的结果」。本模块把这个要求
//! 变成**可复用的工具**，而不是散落在各个测试里的重复代码：
//!
//! * [`Harness`]：用确定性种子驱动一个真实内核（注册 → 通告 → 报价 → 结算 → 自治回合 →
//!   生命周期事件 → 伪造尝试），每一步都写进 [`Journal`]；
//! * [`Harness::replay`]：把日志**重放**到一个全新内核上，用注册表指纹 / 账本 / 拒绝记录 /
//!   生命周期台账四处比对，证明「同一串动作 → 同一个世界」；
//! * [`invariant_suite`]：宿主审计 + 本轨道特有不变式（隔离必须有恶意证据或委员会决定、
//!   台账无孤儿、注册与生命周期一一对应）。
//!
//! 框架本身不读墙钟、不做 I/O：它只是把「Agent 的动作」按顺序喂给内核。

use au4a_core::{
    canonical_hash, AgentKeys, CoreError, CoreResult, Credits, Did, Envelope, EvidenceGrade,
    RefusalCode, SelfCheck,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::autonomy::{AutonomyLayer, AutonomyPolicy};
use crate::lifecycle::{AgentState, LifecycleEvent};
use crate::pmb::{self, kinds_ext};
use crate::Kernel;
use crate::KernelConfig;

/// 日志里的一步。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "step", content = "args")]
pub enum JournalStep {
    Register {
        display: String,
        skills: Vec<String>,
        stake: i64,
    },
    Announce,
    Offer { to: usize, skill: String, price: i64 },
    Settle { to: usize, amount: i64, grade: String },
    /// 伪造一个信封（期望被内核拒绝）。
    Forgery,
    /// 一次自治回合（决策由策略决定，可重放）。
    AutonomyTurn { planned: Vec<String> },
    /// 生命周期事件（用字符串表达，便于日志可读）。
    Lifecycle { event: String },
}

impl JournalStep {
    pub fn as_str(&self) -> &'static str {
        match self {
            JournalStep::Register { .. } => "register",
            JournalStep::Announce => "announce",
            JournalStep::Offer { .. } => "offer",
            JournalStep::Settle { .. } => "settle",
            JournalStep::Forgery => "forgery",
            JournalStep::AutonomyTurn { .. } => "autonomy_turn",
            JournalStep::Lifecycle { .. } => "lifecycle",
        }
    }
}

/// 日志条目：谁、在什么逻辑时刻、做了什么、是否成功。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub at: u64,
    pub actor: usize,
    pub step: JournalStep,
    pub ok: bool,
}

/// 可重放的确定性日志。
pub type Journal = Vec<JournalEntry>;

/// 一次重放的结果（与原世界比对用的四个面）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayOutcome {
    pub registry_fingerprint: String,
    pub lifecycle_fingerprint: String,
    pub minted: i64,
    pub slashed: i64,
    pub refusals: usize,
    pub journal_len: usize,
}

/// 宿主审计 + 本轨道特有不变式。
pub fn invariant_suite(kernel: &Kernel) -> Vec<SelfCheck> {
    let track = crate::TRACK;
    let mut checks = kernel.audit().to_self_checks(track);

    // 隔离必须有据：要么有恶意拒绝记录，要么生命周期历史里有委员会隔离。
    let mut unjustified: Vec<String> = Vec::new();
    for (did, state) in kernel.lifecycles().states() {
        if state != AgentState::Quarantined {
            continue;
        }
        let parsed = au4a_core::Did::parse(&did).ok();
        let has_misconduct = parsed
            .as_ref()
            .map(|did| {
                kernel
                    .refusals()
                    .iter()
                    .any(|(who, refusal)| who == did && refusal.code.is_misconduct())
            })
            .unwrap_or(false);
        let council_ordered = parsed
            .as_ref()
            .and_then(|did| kernel.lifecycle_of(did))
            .map(|life| {
                life.history()
                    .iter()
                    .any(|t| t.event == LifecycleEvent::CouncilQuarantine)
            })
            .unwrap_or(false);
        if !has_misconduct && !council_ordered {
            unjustified.push(did);
        }
    }
    checks.push(if unjustified.is_empty() {
        SelfCheck::pass(
            track,
            "harness.quarantine_justified",
            "每个被隔离的 Agent 都有恶意证据或委员会决定",
        )
    } else {
        SelfCheck::fail(
            track,
            "harness.quarantine_justified",
            format!("无据隔离：{unjustified:?}"),
        )
    });

    // 注册 ↔ 生命周期一一对应。
    let registry_count = kernel.agent_count();
    let lifecycle_count = kernel.lifecycles().len();
    let orphans = kernel.lifecycles().states().keys().any(|did| {
        au4a_core::Did::parse(did)
            .map(|parsed| !kernel.is_registered(&parsed))
            .unwrap_or(true)
    });
    checks.push(if registry_count == lifecycle_count && !orphans {
        SelfCheck::pass(
            track,
            "harness.registry_lifecycle_bijection",
            format!("{registry_count} 个在册 Agent 与 {lifecycle_count} 条生命周期一一对应"),
        )
    } else {
        SelfCheck::fail(
            track,
            "harness.registry_lifecycle_bijection",
            format!("在册 {registry_count} vs 台账 {lifecycle_count}，孤儿={orphans}"),
        )
    });

    // 队列里的信封必须都验签通过（内核收下的都必须是真签名）。
    let unsigned = kernel
        .queued_envelopes()
        .iter()
        .filter(|env| env.verify().is_err())
        .count();
    checks.push(if unsigned == 0 {
        SelfCheck::pass(
            track,
            "harness.queue_sealed",
            format!("{} 个待投递信封全部验签通过", kernel.queued_envelopes().len()),
        )
    } else {
        SelfCheck::fail(track, "harness.queue_sealed", format!("{unsigned} 个信封验签失败"))
    });

    checks
}

/// 从种子表里取第 `index` 个种子（重放时按日志顺序重建身份）。
fn seed_at(seeds: &[[u8; 32]], index: usize) -> CoreResult<&[u8; 32]> {
    seeds.get(index).ok_or(CoreError::UnknownAgent)
}

/// 确定性仿真台：一个内核 + 一串种子 + 一本日志。
pub struct Harness {
    config: KernelConfig,
    base_tag: u8,
    kernel: Kernel,
    seeds: Vec<[u8; 32]>,
    journal: Journal,
}

impl Harness {
    /// 种子由 `base_tag` 派生：同 tag → 同身份 → 同结果。
    pub fn new(config: KernelConfig, base_tag: u8) -> Self {
        Self {
            kernel: Kernel::new(config.clone()),
            config,
            base_tag,
            seeds: Vec::new(),
            journal: Vec::new(),
        }
    }

    pub fn kernel(&self) -> &Kernel {
        &self.kernel
    }

    pub fn kernel_mut(&mut self) -> &mut Kernel {
        &mut self.kernel
    }

    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    pub fn config(&self) -> &KernelConfig {
        &self.config
    }

    pub fn base_tag(&self) -> u8 {
        self.base_tag
    }

    pub fn agent_count(&self) -> usize {
        self.seeds.len()
    }

    fn seed_for(&self, index: usize) -> CoreResult<[u8; 32]> {
        self.seeds.get(index).copied().ok_or(CoreError::UnknownAgent)
    }

    fn keys_for(&self, index: usize) -> CoreResult<AgentKeys> {
        Ok(AgentKeys::from_seed(&self.seed_for(index)?))
    }

    pub fn did_of(&self, index: usize) -> CoreResult<Did> {
        Ok(self.keys_for(index)?.did())
    }

    /// 自主注册 n 个 Agent，全部写进日志。
    pub fn add_agents(&mut self, n: usize, skills: &[&str], stake: Credits) -> CoreResult<Vec<Did>> {
        let mut dids = Vec::new();
        for _ in 0..n {
            let mut seed = [0xC0u8; 32];
            seed[0] = self.base_tag;
            seed[1] = self.seeds.len() as u8;
            // 每个 Agent 的种子由 (base_tag, 序号) 唯一决定，且与其它轨道不共享前缀。
            seed[2] = 0x10 ^ (self.seeds.len() as u8);
            let keys = AgentKeys::from_seed(&seed);
            let display = format!("harness-{}-{}", self.base_tag, self.seeds.len());
            self.seeds.push(seed);
            let at = self.kernel.tick();
            let result = self.kernel.register(&keys, display.clone(), skills, stake);
            let ok = result.is_ok();
            self.journal.push(JournalEntry {
                at,
                actor: self.seeds.len() - 1,
                step: JournalStep::Register {
                    display,
                    skills: skills.iter().map(|s| (*s).to_string()).collect(),
                    stake: stake.get(),
                },
                ok,
            });
            let card = result?;
            dids.push(card.did);
        }
        Ok(dids)
    }

    /// 广播能力通告。
    pub fn announce(&mut self, actor: usize) -> CoreResult<String> {
        let keys = self.keys_for(actor)?;
        let card = self
            .kernel
            .card(&keys.did())
            .cloned()
            .ok_or(CoreError::UnknownAgent)?;
        let at = self.kernel.tick();
        let env = pmb::announce_card(&keys, &card, self.kernel.now() + 1)?;
        let result = self.kernel.send(&env);
        let ok = result.is_ok();
        let id = env.id.clone();
        self.journal.push(JournalEntry {
            at,
            actor,
            step: JournalStep::Announce,
            ok,
        });
        result?;
        Ok(id)
    }

    /// 点对点报价。
    pub fn offer(
        &mut self,
        actor: usize,
        to: usize,
        skill: &str,
        price: Credits,
    ) -> CoreResult<String> {
        let from = self.keys_for(actor)?;
        let to_did = self.did_of(to)?;
        let at = self.kernel.tick();
        let env = Envelope::new(
            from.did(),
            Some(to_did),
            kinds_ext::NEGOTIATE_OFFER,
            self.kernel.now() + 1,
            None,
            json!({"skill": skill, "price": price, "evidence": "cpu-proto"}),
        )?
        .seal(&from)?;
        let result = self.kernel.send(&env);
        let ok = result.is_ok();
        let id = env.id.clone();
        self.journal.push(JournalEntry {
            at,
            actor,
            step: JournalStep::Offer {
                to,
                skill: skill.to_string(),
                price: price.get(),
            },
            ok,
        });
        result?;
        Ok(id)
    }

    /// 结算（经证据闸门；失败也如实记进日志）。
    pub fn settle(
        &mut self,
        actor: usize,
        to: usize,
        amount: Credits,
        grade: EvidenceGrade,
    ) -> CoreResult<bool> {
        let from = self.did_of(actor)?;
        let to_did = self.did_of(to)?;
        let at = self.kernel.tick();
        let ok = self.kernel.settle(&from, &to_did, amount, grade).is_ok();
        self.journal.push(JournalEntry {
            at,
            actor,
            step: JournalStep::Settle {
                to,
                amount: amount.get(),
                grade: grade.as_str().to_string(),
            },
            ok,
        });
        Ok(ok)
    }

    /// 伪造一个信封：期望内核按恶意码拒绝。
    pub fn attempt_forgery(&mut self, actor: usize) -> CoreResult<RefusalCode> {
        let keys = self.keys_for(actor)?;
        let at = self.kernel.tick();
        let mut env = Envelope::new(
            keys.did(),
            None,
            kinds_ext::PROGRESS_EVENT,
            self.kernel.now() + 1,
            None,
            json!({"honest": true}),
        )?
        .seal(&keys)?;
        env.body = json!({"tampered": true});
        let ok = self.kernel.send(&env).is_ok();
        let code = self
            .kernel
            .refusals()
            .last()
            .map(|(_, refusal)| refusal.code)
            .unwrap_or(RefusalCode::Malformed);
        self.journal.push(JournalEntry {
            at,
            actor,
            step: JournalStep::Forgery,
            ok,
        });
        Ok(code)
    }

    /// 让某个 Agent 跑一次自治回合（策略可传入；决策与日志都可重放）。
    pub fn autonomy_turn(
        &mut self,
        actor: usize,
        policy: AutonomyPolicy,
    ) -> CoreResult<crate::AutonomyTurn> {
        let keys = self.keys_for(actor)?;
        let mut layer = AutonomyLayer::new(keys, policy);
        let at = self.kernel.tick();
        let turn = layer.turn(&mut self.kernel)?;
        let planned: Vec<String> = turn.planned.iter().map(|k| k.as_str().to_string()).collect();
        self.journal.push(JournalEntry {
            at,
            actor,
            step: JournalStep::AutonomyTurn { planned },
            ok: true,
        });
        Ok(turn)
    }

    /// 施加生命周期事件。
    pub fn lifecycle(&mut self, actor: usize, event: LifecycleEvent) -> CoreResult<bool> {
        let did = self.did_of(actor)?;
        let at = self.kernel.tick();
        let outcome = self.kernel.apply_lifecycle(&did, event)?;
        self.journal.push(JournalEntry {
            at,
            actor,
            step: JournalStep::Lifecycle {
                event: format!("{event:?}"),
            },
            ok: outcome.applied,
        });
        Ok(outcome.applied)
    }

    /// 日志指纹：可用于跨节点/跨运行比对。
    pub fn journal_fingerprint(&self) -> CoreResult<String> {
        let value = serde_json::to_value(&self.journal).map_err(|_| CoreError::Encoding)?;
        canonical_hash(&value)
    }

    pub fn journal_json(&self) -> Value {
        serde_json::to_value(&self.journal).unwrap_or(Value::Null)
    }

    /// 一次重放：用同样的种子与配置，把日志逐条重放到全新内核上。
    ///
    /// 重放会如实重现「哪一步成功、哪一步失败」——失败也要一致，否则就不是确定性。
    pub fn replay(&self) -> CoreResult<Kernel> {
        let mut kernel = Kernel::new(self.config.clone());
        let mut seeds: Vec<[u8; 32]> = Vec::new();
        for entry in &self.journal {
            // 原世界里每一步之前都 `tick()` 过一次；重放必须复现同一个逻辑时钟，
            // 否则「心跳是否到期」这类判断会漂移。
            kernel.tick();
            match &entry.step {
                JournalStep::Register {
                    display,
                    skills,
                    stake,
                } => {
                    let mut seed = [0xC0u8; 32];
                    seed[0] = self.base_tag;
                    seed[1] = seeds.len() as u8;
                    seed[2] = 0x10 ^ (seeds.len() as u8);
                    let keys = AgentKeys::from_seed(&seed);
                    seeds.push(seed);
                    let skill_refs: Vec<&str> = skills.iter().map(|s| s.as_str()).collect();
                    let result =
                        kernel.register(&keys, display.clone(), &skill_refs, Credits(*stake));
                    if result.is_ok() != entry.ok {
                        return Err(CoreError::DuplicateAgent);
                    }
                }
                JournalStep::Announce => {
                    let keys = AgentKeys::from_seed(seed_at(&seeds, entry.actor)?);
                    let card = kernel
                        .card(&keys.did())
                        .cloned()
                        .ok_or(CoreError::UnknownAgent)?;
                    let env = pmb::announce_card(&keys, &card, kernel.now() + 1)?;
                    if kernel.send(&env).is_ok() != entry.ok {
                        return Err(CoreError::InvalidKind);
                    }
                }
                JournalStep::Offer { to, skill, price } => {
                    let keys = AgentKeys::from_seed(seed_at(&seeds, entry.actor)?);
                    let to_did = AgentKeys::from_seed(seed_at(&seeds, *to)?).did();
                    let env = Envelope::new(
                        keys.did(),
                        Some(to_did),
                        kinds_ext::NEGOTIATE_OFFER,
                        kernel.now() + 1,
                        None,
                        json!({"skill": skill, "price": price, "evidence": "cpu-proto"}),
                    )?
                    .seal(&keys)?;
                    if kernel.send(&env).is_ok() != entry.ok {
                        return Err(CoreError::InvalidKind);
                    }
                }
                JournalStep::Settle { to, amount, grade } => {
                    let from = AgentKeys::from_seed(seed_at(&seeds, entry.actor)?).did();
                    let to_did = AgentKeys::from_seed(seed_at(&seeds, *to)?).did();
                    let grade = EvidenceGrade::parse(grade).ok_or(CoreError::InvalidKind)?;
                    let ok = kernel
                        .settle(&from, &to_did, Credits(*amount), grade)
                        .is_ok();
                    if ok != entry.ok {
                        return Err(CoreError::InsufficientFunds);
                    }
                }
                JournalStep::Forgery => {
                    let keys = AgentKeys::from_seed(seed_at(&seeds, entry.actor)?);
                    let mut env = Envelope::new(
                        keys.did(),
                        None,
                        kinds_ext::PROGRESS_EVENT,
                        kernel.now() + 1,
                        None,
                        json!({"honest": true}),
                    )?
                    .seal(&keys)?;
                    env.body = json!({"tampered": true});
                    if kernel.send(&env).is_ok() != entry.ok {
                        return Err(CoreError::InvalidKind);
                    }
                }
                JournalStep::AutonomyTurn { planned } => {
                    let keys = AgentKeys::from_seed(seed_at(&seeds, entry.actor)?);
                    let mut layer = AutonomyLayer::new(keys, AutonomyPolicy::default());
                    let turn = layer.turn(&mut kernel)?;
                    let replayed: Vec<String> =
                        turn.planned.iter().map(|k| k.as_str().to_string()).collect();
                    if &replayed != planned {
                        return Err(CoreError::InvalidKind);
                    }
                }
                JournalStep::Lifecycle { event } => {
                    let did = AgentKeys::from_seed(seed_at(&seeds, entry.actor)?).did();
                    let ev = match event.as_str() {
                        s if s.contains("WorkStarted") => LifecycleEvent::WorkStarted,
                        s if s.contains("WorkFinished") => LifecycleEvent::WorkFinished,
                        s if s.contains("Recovered") => LifecycleEvent::Recovered,
                        s if s.contains("CouncilQuarantine") => LifecycleEvent::CouncilQuarantine,
                        s if s.contains("CouncilReprieve") => LifecycleEvent::CouncilReprieve,
                        s if s.contains("Retired") => LifecycleEvent::Retired,
                        s if s.contains("Unauthorized") => {
                            LifecycleEvent::Refused(RefusalCode::Unauthorized)
                        }
                        s if s.contains("Malformed") => {
                            LifecycleEvent::Refused(RefusalCode::Malformed)
                        }
                        _ => LifecycleEvent::Refused(RefusalCode::Timeout),
                    };
                    let outcome = kernel.apply_lifecycle(&did, ev)?;
                    if outcome.applied != entry.ok {
                        return Err(CoreError::InvalidKind);
                    }
                }
            }
        }
        Ok(kernel)
    }

    fn outcome_of(kernel: &Kernel) -> CoreResult<ReplayOutcome> {
        Ok(ReplayOutcome {
            registry_fingerprint: kernel.registry_fingerprint()?,
            lifecycle_fingerprint: kernel.lifecycles().fingerprint()?,
            minted: kernel.ledger().minted().get(),
            slashed: kernel.ledger().slashed().get(),
            refusals: kernel.refusals().len(),
            journal_len: 0,
        })
    }

    /// 当前世界的四个面（用于与重放结果比对）。
    pub fn outcome(&self) -> CoreResult<ReplayOutcome> {
        let mut outcome = Self::outcome_of(&self.kernel)?;
        outcome.journal_len = self.journal.len();
        Ok(outcome)
    }

    /// 重放并与当前世界比对；不一致就返回 `Err`（fail-closed）。
    pub fn verify_replay(&self) -> CoreResult<ReplayOutcome> {
        let replayed = self.replay()?;
        let expected = self.outcome()?;
        let mut actual = Self::outcome_of(&replayed)?;
        actual.journal_len = expected.journal_len;
        if actual != expected {
            return Err(CoreError::InvalidSignature);
        }
        Ok(actual)
    }
}

/// 编译期证据：不变式套件只接受共享借用（测试框架不能顺手改状态）。
const _: fn(&Kernel) -> Vec<SelfCheck> = invariant_suite;

#[cfg(test)]
mod tests {
    use super::*;

    fn script() -> Harness {
        let mut h = Harness::new(KernelConfig::default(), 7);
        h.add_agents(3, &["skill.a"], Credits(20)).unwrap();
        h.announce(0).unwrap();
        h.offer(0, 1, "skill.a", Credits(5)).unwrap();
        assert!(h.settle(0, 1, Credits(5), EvidenceGrade::Verified).unwrap());
        assert!(!h.settle(0, 1, Credits(5), EvidenceGrade::Unverified).unwrap());
        h.autonomy_turn(0, AutonomyPolicy::default()).unwrap();
        h.lifecycle(1, LifecycleEvent::WorkStarted).unwrap();
        h.lifecycle(1, LifecycleEvent::Refused(RefusalCode::Timeout))
            .unwrap();
        h.lifecycle(1, LifecycleEvent::Recovered).unwrap();
        let code = h.attempt_forgery(2).unwrap();
        assert!(code.is_misconduct());
        h
    }

    #[test]
    fn the_same_script_produces_the_same_journal_and_world() {
        let a = script();
        let b = script();
        assert_eq!(a.journal_fingerprint().unwrap(), b.journal_fingerprint().unwrap());
        assert_eq!(a.outcome().unwrap(), b.outcome().unwrap());
        assert_eq!(a.journal().len(), 12, "3 注册 + 通告 + 报价 + 2 结算 + 回合 + 3 生命周期 + 伪造");
        assert_eq!(a.journal_json().as_array().map(|j| j.len()), Some(12));
    }

    #[test]
    fn replaying_the_journal_rebuilds_the_identical_world() {
        let h = script();
        let outcome = h.verify_replay().unwrap();
        assert_eq!(outcome.registry_fingerprint, h.outcome().unwrap().registry_fingerprint);
        assert_eq!(outcome.refusals, h.kernel().refusals().len());
        assert!(outcome.minted > 0);
        assert_eq!(outcome.slashed, 0);
        // 重放出来的世界同样满足全部不变式。
        let replayed = h.replay().unwrap();
        let checks = invariant_suite(&replayed);
        assert!(au4a_core::all_passed(&checks), "{checks:?}");
    }

    #[test]
    fn the_invariant_suite_passes_on_a_live_harness_and_catches_tampering() {
        let h = script();
        let checks = invariant_suite(h.kernel());
        assert!(au4a_core::all_passed(&checks), "{:?}", checks);
        let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
        for expected in [
            "harness.quarantine_justified",
            "harness.registry_lifecycle_bijection",
            "harness.queue_sealed",
            "ledger.conservation",
            "lifecycle.tracked",
        ] {
            assert!(names.contains(&expected), "缺少 {expected}");
        }

        // 制造一次「无据隔离」：直接把状态改成 Quarantined，套件必须发现。
        let mut broken = Harness::new(KernelConfig::default(), 8);
        broken.add_agents(1, &["x"], Credits(20)).unwrap();
        let did = broken.did_of(0).unwrap();
        broken
            .kernel_mut()
            .force_quarantine_for_test(&did);
        let checks = invariant_suite(broken.kernel());
        assert!(!au4a_core::all_passed(&checks));
        assert!(checks
            .iter()
            .any(|c| c.name == "harness.quarantine_justified" && !c.passed));
    }

    #[test]
    fn quarantining_through_misconduct_or_a_council_decision_is_justified() {
        let mut h = Harness::new(KernelConfig::default(), 9);
        h.add_agents(2, &["x"], Credits(20)).unwrap();
        let code = h.attempt_forgery(0).unwrap();
        assert!(code.is_misconduct());
        h.lifecycle(0, LifecycleEvent::Refused(code)).unwrap();
        assert_eq!(
            h.kernel().lifecycle_of(&h.did_of(0).unwrap()).unwrap().state(),
            AgentState::Quarantined
        );
        assert!(au4a_core::all_passed(&invariant_suite(h.kernel())));

        // 委员会路径：没有恶意拒绝记录，但历史里有 CouncilQuarantine。
        let mut h2 = Harness::new(KernelConfig::default(), 10);
        h2.add_agents(1, &["x"], Credits(20)).unwrap();
        h2.lifecycle(0, LifecycleEvent::CouncilQuarantine).unwrap();
        assert!(au4a_core::all_passed(&invariant_suite(h2.kernel())));
    }

    #[test]
    fn register_failures_are_journaled_truthfully() {
        let mut h = Harness::new(KernelConfig::default(), 11);
        h.add_agents(1, &["x"], Credits(20)).unwrap();
        // 质押低于准入下限：注册失败，日志里如实记录 ok=false。
        assert!(h.add_agents(1, &["x"], Credits(1)).is_err());
        assert_eq!(h.journal().last().map(|e| e.ok), Some(false));
        assert_eq!(h.journal().len(), 2);
        // 失败也必须被重放出来（失败也是一致性的一部分）。
        assert!(h.verify_replay().is_ok());
    }
}
