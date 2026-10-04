//! v1.6.5 模型更新（Model Update）+ 本地信誉台账（Reputation Ledger）。
//!
//! 行为调整（v1.6.3）给出的是**一步的意图**；模型更新负责把它变成**稳定的轨迹**：
//!
//! | 机制 | 作用 | 实现 |
//! |---|---|---|
//! | 学习率 | 缩放意图，避免一次反馈就把策略推到底 | `delta × learning_rate_bp / 10000` |
//! | 动量 | 指数平滑，抑制单轮噪声 | `m ← (mom×m + (10000−mom)×delta) / 10000` |
//! | 阻尼 | 方向反转时步长减半，抑制振荡 | 上一代方向与本次相反 → `delta / 2` |
//! | 遗忘 | 偏好按比例向 0 收缩，陈旧偏好自动退场 | `bias ← bias × (10000−forget) / 10000`，归零即移除 |
//! | 漂移钳制 | 单代总位移有上限 | `|Δ| ≤ max_drift_bp`，且始终落在 `PolicyBounds` 内 |
//! | 回滚 | 保留最近 `max_generations` 代，可逐字段还原 | `history: Vec<Generation>` |
//!
//! # 信誉为什么不可转让
//!
//! 参考项目里信誉可以被运营方授予或转移，于是「信誉」变成了一种可交易的筹码，而不是
//! 「这个协作者在我这里的实际表现」。AU4A 的做法：
//!
//! * 信誉**只能**由该协作者自己的可观测结果累加（交付成败、违规），没有任何转账/合并/授予接口。
//! * 每个 Agent 维护自己的本地台账（`BTreeMap<Did, i64>`），天然是「我的视角」，不声称全局一致。
//! * 取值被钳制在 `[REPUTATION_MIN, REPUTATION_MAX]`，单个协作者无法靠刷量把自己抬到无限高。

use std::collections::BTreeMap;

use au4a_core::{canonical_hash, CoreError, CoreResult, Did, SelfCheck};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::experience::{Experience, Outcome};
use crate::policy::{PolicyAdjustment, PolicyBounds, PolicyParams};

/// 交付成功带来的信誉变化。
pub const REPUTATION_SUCCESS: i64 = 20;
/// 部分成功带来的信誉变化。
pub const REPUTATION_PARTIAL: i64 = 5;
/// 交付失败带来的信誉变化。
pub const REPUTATION_FAILURE: i64 = -10;
/// 一次违规带来的信誉变化。
pub const REPUTATION_VIOLATION: i64 = -100;
/// 信誉下界。
pub const REPUTATION_MIN: i64 = -1_000;
/// 信誉上界。
pub const REPUTATION_MAX: i64 = 1_000;

/// 本地信誉台账。**不可转让**：只有 `apply_*` 这一条写入路径，且只影响被观察的那个协作者。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReputationLedger {
    scores: BTreeMap<Did, i64>,
    updates: u64,
    clamped: u64,
}

impl ReputationLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// 某协作者的信誉（未知 = 0，不是「不可信」也不是「可信」）。
    pub fn score_of(&self, peer: &Did) -> i64 {
        self.scores.get(peer).copied().unwrap_or(0)
    }

    pub fn len(&self) -> usize {
        self.scores.len()
    }

    pub fn is_empty(&self) -> bool {
        self.scores.is_empty()
    }

    pub fn updates(&self) -> u64 {
        self.updates
    }

    /// 有多少次更新被上下界钳制（观测用）。
    pub fn clamped(&self) -> u64 {
        self.clamped
    }

    /// 全部信誉之和（观察层看整体协作氛围；不含任何 DID）。
    pub fn total(&self) -> i64 {
        self.scores.values().copied().fold(0i64, |a, b| a.saturating_add(b))
    }

    pub fn peers(&self) -> Vec<Did> {
        self.scores.keys().cloned().collect()
    }

    /// 按交付结局累加信誉，返回**实际**施加的变化量（含钳制后的结果）。
    pub fn apply_outcome(&mut self, peer: &Did, outcome: Outcome) -> i64 {
        let delta = match outcome {
            Outcome::Success => REPUTATION_SUCCESS,
            Outcome::Partial => REPUTATION_PARTIAL,
            Outcome::Failure => REPUTATION_FAILURE,
        };
        self.apply_delta(peer, delta)
    }

    /// 违规：单独一条更强的负向更新（违规与低质量不是一回事）。
    pub fn apply_violation(&mut self, peer: &Did) -> i64 {
        self.apply_delta(peer, REPUTATION_VIOLATION)
    }

    /// 一条经验的信誉更新：对该经验的所有参与者各施加一次。
    pub fn apply_experience(&mut self, exp: &Experience) -> CoreResult<i64> {
        exp.validate()?;
        let mut applied = 0i64;
        for peer in &exp.peer_agents {
            applied = applied.saturating_add(self.apply_outcome(peer, exp.outcome));
        }
        Ok(applied)
    }

    /// 唯一的写入原语：只影响 `peer` 自己。
    ///
    /// 这里刻意**没有** `transfer(from, to, amount)`、`grant(...)`、`merge(...)` 之类的接口：
    /// 信誉是「我观察到的你的表现」，不是可以搬运的资产。
    fn apply_delta(&mut self, peer: &Did, delta: i64) -> i64 {
        let before = self.score_of(peer);
        let raw = before.saturating_add(delta);
        let after = raw.clamp(REPUTATION_MIN, REPUTATION_MAX);
        if after != raw {
            self.clamped = self.clamped.saturating_add(1);
        }
        self.scores.insert(peer.clone(), after);
        self.updates = self.updates.saturating_add(1);
        after.saturating_sub(before)
    }

    pub fn to_value(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn digest(&self) -> CoreResult<String> {
        canonical_hash(&self.to_value()?)
    }

    /// 公开投影：只有分布，没有任何 DID。
    pub fn public_json(&self) -> CoreResult<Value> {
        let values: Vec<i64> = self.scores.values().copied().collect();
        let count = values.len() as i64;
        let sum: i64 = values.iter().copied().sum();
        Ok(serde_json::json!({
            "peers": count,
            "min": values.iter().copied().min(),
            "max": values.iter().copied().max(),
            "mean": if count > 0 { sum / count } else { 0 },
            "negative": values.iter().filter(|v| **v < 0).count(),
            "updates": self.updates,
            "clamped": self.clamped,
            "transferable": false,
        }))
    }
}

/// 模型更新参数。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelConfig {
    /// 学习率（基点）：对传入调整量的缩放，10000 = 原样采用。
    pub learning_rate_bp: i64,
    /// 动量（基点）：0 = 无动量（直接用本步），越大越平滑。
    pub momentum_bp: i64,
    /// 遗忘（基点）：每一代偏好向 0 收缩的比例。
    pub forgetting_bp: i64,
    /// 单代漂移上限（基点）。
    pub max_drift_bp: i64,
    /// 保留多少代历史（回滚深度）。
    pub max_generations: u32,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            learning_rate_bp: 10_000,
            momentum_bp: 3_000,
            forgetting_bp: 1_000,
            max_drift_bp: 1_000,
            max_generations: 32,
        }
    }
}

impl ModelConfig {
    pub fn validate(&self) -> CoreResult<()> {
        if self.learning_rate_bp < 0
            || self.momentum_bp < 0
            || self.momentum_bp > 10_000
            || self.forgetting_bp < 0
            || self.forgetting_bp > 10_000
            || self.max_drift_bp < 0
            || self.max_generations == 0
        {
            return Err(CoreError::InvalidKind);
        }
        Ok(())
    }
}

/// 一代更新记录（可核对「这一步到底做了什么」）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateRecord {
    pub generation: u32,
    pub price_before_bp: i64,
    pub price_after_bp: i64,
    pub price_drift_bp: i64,
    /// 本代结束时的动量值。
    pub momentum_bp: i64,
    /// 是否因为方向反转而被阻尼。
    pub damped: bool,
    /// 是否有参数撞上漂移上限（含价格与偏好）。
    pub clamped: bool,
    /// 被遗忘清零并移除的偏好项数量。
    pub forgotten_entries: usize,
    /// 本代信誉实际变化合计。
    pub reputation_delta: i64,
}

/// 保留的一个代际快照（回滚用）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Generation {
    generation: u32,
    params: PolicyParams,
    momentum_price_bp: i64,
}

/// 个体学习模型：策略参数的轨迹 + 本地信誉台账。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearningModel {
    params: PolicyParams,
    generation: u32,
    history: Vec<Generation>,
    momentum_price_bp: i64,
    last_price_drift_bp: i64,
    reputation: ReputationLedger,
    bounds: PolicyBounds,
    config: ModelConfig,
}

impl LearningModel {
    /// 新模型：初始策略 = 未学习的基线（与对照组一致）。
    pub fn new(bounds: PolicyBounds) -> Self {
        Self::with_config(bounds, ModelConfig::default())
    }

    pub fn with_config(bounds: PolicyBounds, config: ModelConfig) -> Self {
        Self {
            params: PolicyParams::baseline(),
            generation: 0,
            history: Vec::new(),
            momentum_price_bp: 0,
            last_price_drift_bp: 0,
            reputation: ReputationLedger::new(),
            bounds,
            config,
        }
    }

    pub fn params(&self) -> &PolicyParams {
        &self.params
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }

    pub fn reputation(&self) -> &ReputationLedger {
        &self.reputation
    }

    /// 信誉台账的可变入口（只有本轨道内部的市场循环用它来记录观测到的交付结果）。
    pub fn reputation_mut(&mut self) -> &mut ReputationLedger {
        &mut self.reputation
    }

    pub fn config(&self) -> &ModelConfig {
        &self.config
    }

    pub fn bounds(&self) -> &PolicyBounds {
        &self.bounds
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// 应用一次行为调整：缩放 → 阻尼 → 动量 → 遗忘 → 漂移钳制 → 代际 +1。
    pub fn apply(&mut self, adjustment: &PolicyAdjustment) -> CoreResult<UpdateRecord> {
        self.config.validate()?;
        let price_before = self.params.price_bp;
        let reputation_before = self.reputation.total();

        // 1) 学习率
        let scaled = adjustment
            .price_moved_bp
            .saturating_mul(self.config.learning_rate_bp)
            / 10_000;
        // 2) 阻尼：与上一代位移方向相反 → 步长减半
        let reversed = self.last_price_drift_bp != 0
            && scaled != 0
            && scaled.signum() != self.last_price_drift_bp.signum();
        let damped = reversed;
        let step = if reversed { scaled / 2 } else { scaled };
        // 3) 动量：指数平滑
        let momentum = self
            .config
            .momentum_bp
            .saturating_mul(self.momentum_price_bp)
            .saturating_add(
                (10_000 - self.config.momentum_bp).saturating_mul(step),
            )
            / 10_000;
        // 4) 漂移钳制
        let drift_clamped = momentum.abs() > self.config.max_drift_bp;
        let drift = momentum.clamp(-self.config.max_drift_bp, self.config.max_drift_bp);

        self.history.push(Generation {
            generation: self.generation,
            params: self.params.clone(),
            momentum_price_bp: self.momentum_price_bp,
        });
        if self.history.len() > self.config.max_generations as usize {
            self.history.remove(0);
        }

        self.momentum_price_bp = drift;
        self.last_price_drift_bp = drift;
        self.params.price_bp = (self.params.price_bp + drift)
            .clamp(self.bounds.price_min_bp, self.bounds.price_max_bp);

        // 5) 遗忘：所有偏好按比例向 0 收缩（归零即移除，陈旧偏好自然退场）
        let mut forgotten = forget(&mut self.params.task_bias_bp, self.config.forgetting_bp);
        forgotten += forget(&mut self.params.peer_bias_bp, self.config.forgetting_bp);

        // 6) 偏好更新：学习率缩放 + 阻尼（同向继续、反向减半）+ 漂移钳制
        let mut clamped_biases = 0usize;
        for (task_type, delta) in &adjustment.task_bias_moved_bp {
            let before = self.params.bias_of_task(task_type);
            let applied = self.scale_bias(*delta, &mut clamped_biases);
            let after = (before + applied).clamp(self.bounds.bias_min_bp, self.bounds.bias_max_bp);
            self.params.task_bias_bp.insert(task_type.clone(), after);
        }
        for (peer, delta) in &adjustment.peer_bias_moved_bp {
            let before = self.params.bias_of_peer(peer);
            let applied = self.scale_bias(*delta, &mut clamped_biases);
            let after = (before + applied).clamp(self.bounds.bias_min_bp, self.bounds.bias_max_bp);
            self.params.peer_bias_bp.insert(peer.clone(), after);
        }
        // 偏好归零的条目也移除，避免「既不偏好也不反感」的僵尸项堆积
        self.params.task_bias_bp.retain(|_, v| *v != 0);
        self.params.peer_bias_bp.retain(|_, v| *v != 0);

        self.generation = self.generation.saturating_add(1);
        let reputation_delta = self.reputation.total().saturating_sub(reputation_before);

        Ok(UpdateRecord {
            generation: self.generation,
            price_before_bp: price_before,
            price_after_bp: self.params.price_bp,
            price_drift_bp: self.params.price_bp - price_before,
            momentum_bp: self.momentum_price_bp,
            damped,
            clamped: drift_clamped || clamped_biases > 0,
            forgotten_entries: forgotten,
            reputation_delta,
        })
    }

    /// 取一个偏好增量的实际施加上限（漂移钳制）。
    fn scale_bias(&self, delta: i64, clamped: &mut usize) -> i64 {
        let scaled = delta.saturating_mul(self.config.learning_rate_bp) / 10_000;
        if scaled.abs() > self.config.max_drift_bp {
            *clamped += 1;
            scaled.clamp(-self.config.max_drift_bp, self.config.max_drift_bp)
        } else {
            scaled
        }
    }

    /// 回滚一代：逐字段还原参数与动量，代际计数回退。
    pub fn rollback(&mut self) -> CoreResult<PolicyParams> {
        let Some(previous) = self.history.pop() else {
            return Err(CoreError::InvalidVersion);
        };
        self.params = previous.params;
        self.momentum_price_bp = previous.momentum_price_bp;
        self.last_price_drift_bp = 0; // 回滚后不再假设方向
        self.generation = previous.generation;
        Ok(self.params.clone())
    }

    pub fn to_value(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn digest(&self) -> CoreResult<String> {
        canonical_hash(&self.to_value()?)
    }

    /// 公开投影：策略参数（不含 DID）+ 信誉分布。
    pub fn public_json(&self) -> CoreResult<Value> {
        Ok(serde_json::json!({
            "generation": self.generation,
            "params": self.params.public_json()?,
            "reputation": self.reputation.public_json()?,
            "history_len": self.history.len(),
            "config": &self.config,
        }))
    }

    /// 真实断言：动量/阻尼/遗忘/钳制/回滚/信誉不可转让。
    pub fn self_check() -> Vec<SelfCheck> {
        let mut checks = Vec::new();
        let bounds = PolicyBounds::default();
        let config = ModelConfig::default();

        // 1) 回滚逐字段还原
        let mut model = LearningModel::with_config(bounds.clone(), config.clone());
        let adjustment = PolicyAdjustment {
            next: PolicyParams {
                price_bp: 11_500,
                task_bias_bp: BTreeMap::new(),
                peer_bias_bp: BTreeMap::new(),
            },
            changed: true,
            price_moved_bp: -500,
            task_bias_moved_bp: BTreeMap::new(),
            peer_bias_moved_bp: BTreeMap::new(),
            reasons: vec!["test".to_string()],
        };
        let before = model.params().clone();
        let applied = model.apply(&adjustment);
        let rolled = model.rollback();
        let rollback_ok = matches!((&applied, &rolled), (Ok(_), Ok(p)) if *p == before && model.generation() == 0 && model.history_len() == 0);
        checks.push(crate::check(
            "model.rollback_exact",
            rollback_ok,
            "应用一代后回滚：参数逐字段还原、代际回到 0、历史清空",
        ));

        // 2) 动量：连续同向步长的实际位移逐步逼近目标步长（平滑上升）
        let mut m2 = LearningModel::with_config(bounds.clone(), config.clone());
        let mut drifts = Vec::new();
        for _ in 0..6 {
            if let Ok(rec) = m2.apply(&adjustment) {
                drifts.push(rec.price_drift_bp);
            }
        }
        let momentum_ok = drifts.len() == 6
            && drifts[0].abs() > 0
            && drifts[0].abs() < drifts[1].abs()
            && drifts[1].abs() <= drifts[5].abs()
            && drifts.iter().all(|d| d.abs() <= config.max_drift_bp);
        checks.push(crate::check(
            "model.momentum_smooths",
            momentum_ok,
            format!("连续 6 代同向位移：{drifts:?}（幅度单调上升且不超过漂移上限）"),
        ));

        // 3) 阻尼：方向反转的一代步长减半
        let reverse = PolicyAdjustment {
            price_moved_bp: 500,
            ..adjustment.clone()
        };
        let mut m3 = LearningModel::with_config(
            bounds.clone(),
            ModelConfig {
                momentum_bp: 0,
                ..config.clone()
            },
        );
        let first = m3.apply(&adjustment).map(|r| r.price_drift_bp).unwrap_or(0);
        let second = m3.apply(&reverse);
        let damping_ok = match &second {
            Ok(r) => r.damped && r.price_drift_bp.abs() == first.abs() / 2,
            Err(_) => false,
        };
        checks.push(crate::check(
            "model.damping_halves",
            damping_ok,
            format!(
                "无动量时 +500 后反转 → 实际位移 {first} → {:?}（幅度减半）",
                second.map(|r| r.price_drift_bp)
            ),
        ));

        // 4) 遗忘：长期不动的偏好在若干代后归零并移除
        let mut m4 = LearningModel::with_config(bounds.clone(), config.clone());
        let with_bias = PolicyAdjustment {
            task_bias_moved_bp: {
                let mut m = BTreeMap::new();
                m.insert("translate.en-zh".to_string(), 100);
                m
            },
            price_moved_bp: 0,
            ..adjustment.clone()
        };
        let _ = m4.apply(&with_bias);
        let peak = m4.params().bias_of_task("translate.en-zh");
        for _ in 0..64 {
            let noop = PolicyAdjustment {
                next: m4.params().clone(),
                changed: false,
                price_moved_bp: 0,
                task_bias_moved_bp: BTreeMap::new(),
                peer_bias_moved_bp: BTreeMap::new(),
                reasons: vec!["noop".to_string()],
            };
            let _ = m4.apply(&noop);
        }
        let after = m4.params().bias_of_task("translate.en-zh");
        checks.push(crate::check(
            "model.forgetting_decays",
            peak > 0 && after == 0 && !m4.params().task_bias_bp.contains_key("translate.en-zh"),
            format!("偏好峰值 {peak}bp → 64 代无更新后 {after}bp（条目已移除）"),
        ));

        // 5) 信誉：钳制 + 不可转让（另一个协作者的经历不改变本协作者的信誉）
        let a = au4a_core::AgentKeys::from_seed(&[0x40; 32]).did();
        let b = au4a_core::AgentKeys::from_seed(&[0x41; 32]).did();
        let mut rep = ReputationLedger::new();
        for _ in 0..100 {
            rep.apply_violation(&a);
        }
        for _ in 0..5 {
            rep.apply_outcome(&b, Outcome::Success);
        }
        let reputation_ok = rep.score_of(&a) == REPUTATION_MIN
            && rep.score_of(&b) == 5 * REPUTATION_SUCCESS
            && rep.clamped() > 0
            && rep.public_json().map(|v| !v.to_string().contains("did:au4a:")).unwrap_or(false);
        checks.push(crate::check(
            "model.reputation_bounded_and_local",
            reputation_ok,
            format!(
                "100 次违规 → {}（下界 {}）；另一个协作者 5 次成功 → {}（不受影响）；公开投影无 DID",
                rep.score_of(&a),
                REPUTATION_MIN,
                rep.score_of(&b)
            ),
        ));
        checks
    }
}

/// 遗忘：把偏好像 0 收缩 `forget_bp/10000`，返回被移除（归零）的条目数。
fn forget<K: Ord + Clone>(map: &mut BTreeMap<K, i64>, forget_bp: i64) -> usize {
    if forget_bp <= 0 {
        return 0;
    }
    let keep = 10_000 - forget_bp;
    let mut removed = 0usize;
    let mut next: BTreeMap<K, i64> = BTreeMap::new();
    for (k, v) in map.iter() {
        let shrunk = v.saturating_mul(keep) / 10_000;
        if shrunk == 0 {
            removed += 1;
        } else {
            next.insert(k.clone(), shrunk);
        }
    }
    *map = next;
    removed
}

/// 模块级自检入口（`crate::self_check` 聚合它）。
pub fn self_check() -> Vec<SelfCheck> {
    LearningModel::self_check()
}
