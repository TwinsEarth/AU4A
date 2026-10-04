//! Agent 生命周期状态机（v1.0.7）。
//!
//! 状态与事件的合法迁移是**白名单**，不是「默认可做」：
//!
//! ```text
//! Provisional --Admitted--> Active <--WorkFinished-- Busy
//!                             |  ^                     ^
//!                    WorkStarted  Recovered            |
//!                             v  |                     |
//!                      Busy / Degraded ---------------+
//!                             |
//!        CouncilQuarantine / Refused(恶意 2 码)
//!                             v
//!                        Quarantined --CouncilReprieve--> Active
//!                             |
//!                    Retired（终态，不可逆）
//! ```
//!
//! 三条硬不变式（都有穷举测试）：
//!
//! 1. **竞争永不隔离**：`Refused(code)` 只有在 `code.is_misconduct()` 时才进入 `Quarantined`；
//!    其余 8 个码只降到 `Degraded`，且 `Degraded` 是可恢复状态。
//! 2. **隔离必须由 Agent 侧证据或委员会决定**：`CouncilQuarantine` 是唯一另一条入口，
//!    而委员会动议本身只对恶意码开放（见 `council.rs`）。
//! 3. **`Retired` 是终态**：任何事件都不能把它复活；对已退役 Agent 的操作返回 `stale_epoch`
//!    （竞争语义：对端已经不在网里了，不该被当成恶意）。
//!
//! 状态机是**事件溯源**的：`apply` 只追加历史，`replay` 可以从事件流重建同一状态。

use std::collections::BTreeMap;

use au4a_core::{canonical_hash, CoreError, CoreResult, Did, RefusalCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Agent 生命周期状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// 已注册但尚未通过准入确认。
    Provisional,
    /// 正常在网，可收发、可结算。
    Active,
    /// 正在处理工作（不改变权限，只是可见性）。
    Busy,
    /// 降级：发生过竞争性失败或收到警告，但**没有**被隔离。
    Degraded,
    /// 隔离：只由恶意证据或委员会决定进入。
    Quarantined,
    /// 退役：终态。
    Retired,
}

impl AgentState {
    pub const ALL: [AgentState; 6] = [
        AgentState::Provisional,
        AgentState::Active,
        AgentState::Busy,
        AgentState::Degraded,
        AgentState::Quarantined,
        AgentState::Retired,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AgentState::Provisional => "provisional",
            AgentState::Active => "active",
            AgentState::Busy => "busy",
            AgentState::Degraded => "degraded",
            AgentState::Quarantined => "quarantined",
            AgentState::Retired => "retired",
        }
    }

    /// 是否还能参与网络（可收发/可结算）。
    pub fn is_participating(self) -> bool {
        matches!(self, AgentState::Active | AgentState::Busy)
    }

    /// 是否被限制。
    pub fn is_restricted(self) -> bool {
        matches!(self, AgentState::Quarantined | AgentState::Retired)
    }
}

/// 生命周期事件。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "event", content = "code")]
pub enum LifecycleEvent {
    /// 注册/准入完成。
    Admitted,
    /// 开始工作。
    WorkStarted,
    /// 工作结束。
    WorkFinished,
    /// 被拒绝（分类决定去向：恶意 → 隔离，竞争 → 降级）。
    Refused(RefusalCode),
    /// 从降级中恢复。
    Recovered,
    /// 委员会通过的隔离决定。
    CouncilQuarantine,
    /// 委员会通过的解除隔离决定。
    CouncilReprieve,
    /// 主动退役。
    Retired,
}

impl LifecycleEvent {
    /// 穷举用的事件样本（`Refused` 覆盖恶意与竞争各一个）。
    pub const ALL_SAMPLES: [LifecycleEvent; 10] = [
        LifecycleEvent::Admitted,
        LifecycleEvent::WorkStarted,
        LifecycleEvent::WorkFinished,
        LifecycleEvent::Refused(RefusalCode::Malformed),
        LifecycleEvent::Refused(RefusalCode::Unauthorized),
        LifecycleEvent::Refused(RefusalCode::Timeout),
        LifecycleEvent::Refused(RefusalCode::PolicyDenied),
        LifecycleEvent::Recovered,
        LifecycleEvent::CouncilQuarantine,
        LifecycleEvent::CouncilReprieve,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            LifecycleEvent::Admitted => "admitted",
            LifecycleEvent::WorkStarted => "work_started",
            LifecycleEvent::WorkFinished => "work_finished",
            LifecycleEvent::Refused(_) => "refused",
            LifecycleEvent::Recovered => "recovered",
            LifecycleEvent::CouncilQuarantine => "council_quarantine",
            LifecycleEvent::CouncilReprieve => "council_reprieve",
            LifecycleEvent::Retired => "retired",
        }
    }

    /// 该事件是否携带拒绝码。
    pub fn code(self) -> Option<RefusalCode> {
        match self {
            LifecycleEvent::Refused(code) => Some(code),
            _ => None,
        }
    }
}

/// 一次合法迁移的记录。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition {
    pub from: AgentState,
    pub to: AgentState,
    pub event: LifecycleEvent,
    pub at: u64,
}

/// 纯迁移函数：白名单。`Err` 里的码说明为什么这次事件不合法。
pub fn next_state(state: AgentState, event: LifecycleEvent) -> Result<AgentState, RefusalCode> {
    use AgentState as S;
    use LifecycleEvent as E;
    // 终态：退役之后一切都被当作对端过期（竞争），而不是恶意。
    if state == S::Retired {
        return match event {
            E::Retired => Ok(S::Retired),
            _ => Err(RefusalCode::StaleEpoch),
        };
    }
    match event {
        E::Admitted => match state {
            S::Provisional => Ok(S::Active),
            _ => Err(RefusalCode::Conflict),
        },
        E::WorkStarted => match state {
            S::Active => Ok(S::Busy),
            _ => Err(RefusalCode::Conflict),
        },
        E::WorkFinished => match state {
            S::Busy => Ok(S::Active),
            _ => Err(RefusalCode::Conflict),
        },
        E::Refused(code) => {
            if code.is_misconduct() {
                Ok(S::Quarantined)
            } else {
                match state {
                    S::Quarantined => Ok(S::Quarantined), // 已被隔离：竞争失败不改变状态
                    S::Degraded => Ok(S::Degraded),
                    _ => Ok(S::Degraded),
                }
            }
        }
        E::Recovered => match state {
            S::Degraded => Ok(S::Active),
            S::Active | S::Busy => Ok(state),
            _ => Err(RefusalCode::Conflict),
        },
        E::CouncilQuarantine => match state {
            S::Quarantined => Ok(S::Quarantined),
            _ => Ok(S::Quarantined),
        },
        E::CouncilReprieve => match state {
            S::Quarantined => Ok(S::Active),
            _ => Err(RefusalCode::Conflict),
        },
        E::Retired => Ok(S::Retired),
    }
}

/// 一次「尝试施加事件」的结果：不抛错，而是把拒绝码放在值里，便于审计与观察。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleOutcome {
    pub did: String,
    pub from: AgentState,
    pub to: AgentState,
    pub applied: bool,
    pub code: Option<RefusalCode>,
}

impl LifecycleOutcome {
    pub fn to_json(&self) -> Value {
        json!({
            "did": self.did,
            "from": self.from.as_str(),
            "to": self.to.as_str(),
            "applied": self.applied,
            "code": self.code.map(|c| c.as_str()),
        })
    }
}

/// 单个 Agent 的生命周期（事件溯源）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lifecycle {
    did: String,
    state: AgentState,
    history: Vec<Transition>,
}

impl Lifecycle {
    /// 新 Agent：`Provisional` 起步。
    pub fn new(did: &Did) -> Self {
        Self {
            did: did.as_str().to_string(),
            state: AgentState::Provisional,
            history: Vec::new(),
        }
    }

    pub fn did(&self) -> &str {
        &self.did
    }

    pub fn state(&self) -> AgentState {
        self.state
    }

    pub fn history(&self) -> &[Transition] {
        &self.history
    }

    /// 白名单预判：与 `apply` 的成功/失败完全一致（有穷举测试）。
    pub fn can(&self, event: LifecycleEvent) -> bool {
        next_state(self.state, event).is_ok()
    }

    /// 当前状态下所有合法事件。
    pub fn allowed_events(&self) -> Vec<LifecycleEvent> {
        LifecycleEvent::ALL_SAMPLES
            .iter()
            .copied()
            .filter(|event| self.can(*event))
            .collect()
    }

    /// 应用一个事件：合法则迁移并记入历史，非法则**不改状态**并返回类型化拒绝码。
    pub fn apply(&mut self, event: LifecycleEvent, at: u64) -> Result<Transition, RefusalCode> {
        let to = next_state(self.state, event)?;
        let transition = Transition {
            from: self.state,
            to,
            event,
            at,
        };
        self.state = to;
        self.history.push(transition);
        Ok(transition)
    }

    /// 从事件流重建（重放）：同一事件流 → 同一状态与同一历史。
    pub fn replay(did: &Did, events: &[(LifecycleEvent, u64)]) -> Result<Self, RefusalCode> {
        let mut life = Lifecycle::new(did);
        for (event, at) in events {
            life.apply(*event, *at)?;
        }
        Ok(life)
    }

    /// 施加事件但不返回 `Result`：把拒绝码放进 [`LifecycleOutcome`]（状态不变）。
    pub fn attempt(&mut self, event: LifecycleEvent, at: u64) -> LifecycleOutcome {
        let from = self.state;
        match self.apply(event, at) {
            Ok(transition) => LifecycleOutcome {
                did: self.did.clone(),
                from,
                to: transition.to,
                applied: true,
                code: None,
            },
            Err(code) => LifecycleOutcome {
                did: self.did.clone(),
                from,
                to: from,
                applied: false,
                code: Some(code),
            },
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "did": self.did,
            "state": self.state.as_str(),
            "participating": self.state.is_participating(),
            "restricted": self.state.is_restricted(),
            "transitions": self.history.len(),
            "history": self.history,
        })
    }

    /// 历史指纹：可用来比对两台节点的重放结果。
    pub fn fingerprint(&self) -> CoreResult<String> {
        let value = serde_json::to_value(&self.history).map_err(|_| CoreError::Encoding)?;
        canonical_hash(&value)
    }
}

/// 全网络的 Agent 生命周期台账。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LifecycleBook {
    lives: BTreeMap<String, Lifecycle>,
}

impl LifecycleBook {
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一个新 Agent（`Provisional`）。
    pub fn enroll(&mut self, did: &Did) -> &Lifecycle {
        self.lives
            .entry(did.as_str().to_string())
            .or_insert_with(|| Lifecycle::new(did))
    }

    pub fn get(&self, did: &Did) -> Option<&Lifecycle> {
        self.lives.get(did.as_str())
    }

    pub fn len(&self) -> usize {
        self.lives.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lives.is_empty()
    }

    /// 对某个 Agent 施加事件；未登记则返回 `UnknownAgent`。
    pub fn apply(
        &mut self,
        did: &Did,
        event: LifecycleEvent,
        at: u64,
    ) -> CoreResult<Transition> {
        let life = self
            .lives
            .get_mut(did.as_str())
            .ok_or(CoreError::UnknownAgent)?;
        life.apply(event, at).map_err(|_| CoreError::InvalidKind)
    }

    /// 不关心拒绝码的调用方：返回是否发生了状态变化。
    pub fn try_apply(&mut self, did: &Did, event: LifecycleEvent, at: u64) -> Option<Transition> {
        self.lives
            .get_mut(did.as_str())
            .and_then(|life| life.apply(event, at).ok())
    }

    /// 施加事件并保留拒绝码（供内核 API 使用）。
    pub fn attempt(
        &mut self,
        did: &Did,
        event: LifecycleEvent,
        at: u64,
    ) -> Option<LifecycleOutcome> {
        self.lives
            .get_mut(did.as_str())
            .map(|life| life.attempt(event, at))
    }

    pub fn states(&self) -> BTreeMap<String, AgentState> {
        self.lives
            .iter()
            .map(|(did, life)| (did.clone(), life.state()))
            .collect()
    }

    pub fn count(&self, state: AgentState) -> usize {
        self.lives.values().filter(|l| l.state() == state).count()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "agents": self.lives.len(),
            "by_state": AgentState::ALL
                .iter()
                .map(|s| (s.as_str().to_string(), json!(self.count(*s))))
                .collect::<serde_json::Map<String, Value>>(),
            "states": self.states(),
        })
    }

    pub fn fingerprint(&self) -> CoreResult<String> {
        let value = serde_json::to_value(self.states()).map_err(|_| CoreError::Encoding)?;
        canonical_hash(&value)
    }
}

/// 编译期证据：迁移函数是纯函数（不接触内核、不读时钟）。
const _: fn(AgentState, LifecycleEvent) -> Result<AgentState, RefusalCode> = next_state;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Kernel, KernelConfig};
    use au4a_core::AgentKeys;

    fn keys(tag: u8) -> AgentKeys {
        AgentKeys::from_seed(&[tag; 32])
    }

    #[test]
    fn the_happy_path_is_admitted_then_work_then_idle() {
        let did = keys(1).did();
        let mut life = Lifecycle::new(&did);
        assert_eq!(life.state(), AgentState::Provisional);
        assert!(!life.state().is_restricted());
        assert!(!life.state().is_participating());

        life.apply(LifecycleEvent::Admitted, 1).unwrap();
        assert_eq!(life.state(), AgentState::Active);
        life.apply(LifecycleEvent::WorkStarted, 2).unwrap();
        assert_eq!(life.state(), AgentState::Busy);
        assert!(life.state().is_participating());
        life.apply(LifecycleEvent::WorkFinished, 3).unwrap();
        assert_eq!(life.state(), AgentState::Active);
        assert_eq!(life.history().len(), 3);
        assert!(life.allowed_events().contains(&LifecycleEvent::WorkStarted));
        assert!(!life.allowed_events().contains(&LifecycleEvent::CouncilReprieve));
    }

    #[test]
    fn illegal_transitions_are_refused_with_typed_codes_and_do_not_mutate() {
        let did = keys(2).did();
        let mut life = Lifecycle::new(&did);
        assert_eq!(
            life.apply(LifecycleEvent::WorkStarted, 1),
            Err(RefusalCode::Conflict)
        );
        assert_eq!(life.state(), AgentState::Provisional, "非法事件不得改变状态");
        assert!(life.history().is_empty());
        life.apply(LifecycleEvent::Admitted, 2).unwrap();
        assert_eq!(life.apply(LifecycleEvent::Admitted, 3), Err(RefusalCode::Conflict));
        assert_eq!(
            life.apply(LifecycleEvent::CouncilReprieve, 4),
            Err(RefusalCode::Conflict),
            "没被隔离就谈不上解除"
        );
        assert_eq!(life.state(), AgentState::Active);
    }

    #[test]
    fn competition_never_quarantines_exhaustively_over_all_codes() {
        for code in RefusalCode::ALL {
            let did = keys(3).did();
            let mut life = Lifecycle::new(&did);
            life.apply(LifecycleEvent::Admitted, 1).unwrap();
            let result = life.apply(LifecycleEvent::Refused(code), 2).unwrap();
            if code.is_misconduct() {
                assert_eq!(result.to, AgentState::Quarantined, "{}", code.as_str());
                assert!(life.state().is_restricted());
            } else {
                assert_eq!(result.to, AgentState::Degraded, "{}", code.as_str());
                assert_ne!(life.state(), AgentState::Quarantined);
                // 降级可以恢复，且恢复不需要任何外部批准。
                life.apply(LifecycleEvent::Recovered, 3).unwrap();
                assert_eq!(life.state(), AgentState::Active);
            }
        }
    }

    #[test]
    fn repeated_competition_stays_degraded_and_recoverable() {
        let did = keys(4).did();
        let mut life = Lifecycle::new(&did);
        life.apply(LifecycleEvent::Admitted, 1).unwrap();
        for i in 0..50u64 {
            life.apply(LifecycleEvent::Refused(RefusalCode::Timeout), i + 2)
                .unwrap();
            assert_eq!(life.state(), AgentState::Degraded);
        }
        life.apply(LifecycleEvent::Recovered, 100).unwrap();
        assert_eq!(life.state(), AgentState::Active);
        assert_eq!(life.history().len(), 52);
    }

    #[test]
    fn retired_is_terminal() {
        let did = keys(5).did();
        let mut life = Lifecycle::new(&did);
        life.apply(LifecycleEvent::Admitted, 1).unwrap();
        life.apply(LifecycleEvent::Retired, 2).unwrap();
        assert_eq!(life.state(), AgentState::Retired);
        for event in LifecycleEvent::ALL_SAMPLES {
            let before = life.state();
            let result = life.apply(event, 3);
            match event {
                LifecycleEvent::Retired => assert!(result.is_ok()),
                _ => assert_eq!(result, Err(RefusalCode::StaleEpoch), "{:?}", event),
            }
            assert_eq!(life.state(), before);
        }
        assert_eq!(life.apply(LifecycleEvent::Retired, 4).unwrap().to, AgentState::Retired);
    }

    #[test]
    fn can_and_apply_agree_and_council_events_are_agent_side() {
        let did = keys(6).did();
        // 穷举：所有状态 × 所有事件样本，can() 与 apply() 的成功/失败必须一致。
        let paths: Vec<Vec<LifecycleEvent>> = vec![
            vec![],
            vec![LifecycleEvent::Admitted],
            vec![LifecycleEvent::Admitted, LifecycleEvent::WorkStarted],
            vec![LifecycleEvent::Admitted, LifecycleEvent::Refused(RefusalCode::Timeout)],
            vec![LifecycleEvent::Admitted, LifecycleEvent::Refused(RefusalCode::Malformed)],
            vec![LifecycleEvent::Admitted, LifecycleEvent::Retired],
        ];
        for path in paths {
            let mut probe = Lifecycle::new(&did);
            for event in &path {
                probe.apply(*event, 1).unwrap();
            }
            for event in LifecycleEvent::ALL_SAMPLES {
                let mut fresh = Lifecycle::new(&did);
                for e in &path {
                    fresh.apply(*e, 1).unwrap();
                }
                // allowed_events 必须恰好等于 can 为真的集合（在施加事件**之前**枚举）。
                let allowed_before = fresh.allowed_events();
                let can = fresh.can(event);
                assert_eq!(allowed_before.contains(&event), can);
                let applied = fresh.apply(event, 2);
                assert_eq!(can, applied.is_ok(), "state={:?} event={:?}", probe.state(), event);
            }
        }
        // 委员会隔离 → 解除隔离 → 恢复 Active。
        let mut life = Lifecycle::new(&did);
        life.apply(LifecycleEvent::Admitted, 1).unwrap();
        life.apply(LifecycleEvent::CouncilQuarantine, 2).unwrap();
        assert_eq!(life.state(), AgentState::Quarantined);
        life.apply(LifecycleEvent::CouncilReprieve, 3).unwrap();
        assert_eq!(life.state(), AgentState::Active);
    }

    #[test]
    fn replay_rebuilds_the_identical_lifecycle() {
        let did = keys(7).did();
        let events = [
            (LifecycleEvent::Admitted, 1),
            (LifecycleEvent::WorkStarted, 2),
            (LifecycleEvent::Refused(RefusalCode::Conflict), 3),
            (LifecycleEvent::Recovered, 4),
            (LifecycleEvent::Refused(RefusalCode::Unauthorized), 5),
            (LifecycleEvent::CouncilReprieve, 6),
        ];
        let a = Lifecycle::replay(&did, &events).unwrap();
        let b = Lifecycle::replay(&did, &events).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.fingerprint().unwrap(), b.fingerprint().unwrap());
        assert_eq!(a.state(), AgentState::Active);
        // 中途非法的事件流会被拒绝并指出原因。
        let bad = [
            (LifecycleEvent::WorkStarted, 1),
            (LifecycleEvent::Admitted, 2),
        ];
        assert_eq!(Lifecycle::replay(&did, &bad), Err(RefusalCode::Conflict));
    }

    #[test]
    fn the_book_tracks_every_agent_and_stays_deterministic() {
        let mut k = Kernel::new(KernelConfig::default());
        let mut book = LifecycleBook::new();
        for i in 0..3u8 {
            let keys = keys(30 + i);
            k.register(&keys, format!("agent-{i}"), &["x"], au4a_core::Credits(20))
                .unwrap();
            book.enroll(&keys.did());
            book.apply(&keys.did(), LifecycleEvent::Admitted, i as u64).unwrap();
        }
        assert_eq!(book.len(), 3);
        assert_eq!(book.count(AgentState::Active), 3);
        assert!(book.get(&keys(30).did()).is_some());
        assert!(book.apply(&keys(99).did(), LifecycleEvent::Admitted, 1).is_err());

        // 竞争失败 10 次 + 一次恶意：只有恶意那次进入隔离。
        let victim = keys(30).did();
        for i in 0..10 {
            book.try_apply(&victim, LifecycleEvent::Refused(RefusalCode::RateLimited), i);
        }
        assert_eq!(book.get(&victim).unwrap().state(), AgentState::Degraded);
        book.try_apply(&victim, LifecycleEvent::Refused(RefusalCode::Unauthorized), 100);
        assert_eq!(book.get(&victim).unwrap().state(), AgentState::Quarantined);
        assert_eq!(book.count(AgentState::Quarantined), 1);

        let a = book.to_json();
        let b = book.to_json();
        assert_eq!(a, b);
        assert_eq!(a["by_state"]["quarantined"], 1);
        assert_eq!(book.fingerprint().unwrap(), book.fingerprint().unwrap());
    }
}
