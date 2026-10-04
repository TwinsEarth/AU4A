//! v1.6.8 可解释性（Explain）：每个策略参数**为什么**是这个值。
//!
//! 文档如果只是人写的散文，就会慢慢和代码分叉。本模块的做法是：解释由**与行为相同的函数**产出
//! （[`crate::policy::price_decision`] / [`crate::policy::bias_delta_bp`] / [`crate::policy::task_score_bp`]），
//! 并把「解释所蕴含的下一步动作」也发布出来（`implied_*`）。测试逐项断言
//! `implied_price_move_bp` 与 `adjust()` 实际给出的移动量一致——解释与行为不可能各说各话。
//!
//! 解释文本里的协作者用**加盐摘要前缀**标识（同 [`crate::policy`]），所以整份解释可以直接对外发布。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use au4a_core::{canonical_hash, content_hash, short_id, CoreError, CoreResult, Credits, Did, SelfCheck};

use crate::feedback::FeedbackReport;
use crate::policy::{bias_delta_bp, price_decision, task_score_bp, PolicyBounds, PolicyParams, PolicyTargets, Signals};
use crate::violation::ViolationLog;

/// 定价解释（含「解释所蕴含的动作」）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceExplanation {
    pub current_price_bp: i64,
    pub accept_rate_bp: Option<i64>,
    pub target_bp: i64,
    pub deadband_bp: i64,
    pub smoothed_reward: Credits,
    pub prev_smoothed_reward: Credits,
    pub reputation_delta: i64,
    /// 解释所蕴含的定价方向（-1 / 0 / +1）。
    pub implied_direction: i64,
    /// 解释所蕴含的移动量（基点，已按 bounds 钳制）。
    pub implied_move_bp: i64,
    pub reason: String,
}

/// 一个偏好项的解释。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BiasExplanation {
    /// 任务类型名，或协作者的摘要前缀（`peer:<tag>`）。
    pub target: String,
    pub kind: String,
    pub current_bias_bp: i64,
    /// 任务：综合评分；协作者：完成质量。
    pub score_bp: i64,
    /// 对应的整体值。
    pub overall_bp: i64,
    pub violations: u32,
    /// 解释所蕴含的增量（与 `adjust` 使用同一公式）。
    pub implied_delta_bp: i64,
    pub reason: String,
}

/// 一份完整的策略解释。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyExplanation {
    pub price: PriceExplanation,
    pub task_biases: Vec<BiasExplanation>,
    pub peer_biases: Vec<BiasExplanation>,
    pub evidence: String,
}

impl PolicyExplanation {
    pub fn to_value(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn digest(&self) -> CoreResult<String> {
        canonical_hash(&self.to_value()?)
    }

    /// 人类可读摘要（观察层「结果」面板可以直接显示；不含 DID）。
    pub fn summary(&self) -> String {
        let mut lines = vec![format!("定价 {}bp：{}", self.price.current_price_bp, self.price.reason)];
        for b in &self.task_biases {
            lines.push(format!("任务偏好 {}={}bp：{}", b.target, b.current_bias_bp, b.reason));
        }
        for b in &self.peer_biases {
            lines.push(format!("协作偏好 {}={}bp：{}", b.target, b.current_bias_bp, b.reason));
        }
        lines.push(self.evidence.clone());
        lines.join("\n")
    }
}

/// 协作者标识：DID 的加盐摘要前缀，可发布、不含 DID。
///
/// 与 `policy` 模块里 `peer_tag` 的用途一致（把解释拿去发布时不泄露身份），
/// 这里额外带上 `peer:` 前缀，让读者一眼看出这是脱敏标签而不是任务类型名。
pub fn explain_peer_tag(peer: &Did) -> String {
    format!("peer:{}", short_id(&content_hash(peer.as_str().as_bytes())))
}

/// 生成策略解释。
pub fn explain(
    params: &PolicyParams,
    report: &FeedbackReport,
    violations: &ViolationLog,
    signals: &Signals,
    bounds: &PolicyBounds,
    targets: &PolicyTargets,
) -> CoreResult<PolicyExplanation> {
    // ---- 定价 ----
    let (dir, reason) = price_decision(signals, targets);
    let implied_move = if dir == 0 {
        0
    } else {
        let step = bounds.price_step_bp * dir.signum();
        let after = (params.price_bp + step).clamp(bounds.price_min_bp, bounds.price_max_bp);
        after - params.price_bp
    };
    let price = PriceExplanation {
        current_price_bp: params.price_bp,
        accept_rate_bp: signals.accept_rate_bp,
        target_bp: targets.accept_rate_target_bp,
        deadband_bp: targets.accept_deadband_bp,
        smoothed_reward: signals.mean_reward,
        prev_smoothed_reward: signals.prev_mean_reward,
        reputation_delta: signals.reputation_delta,
        implied_direction: dir,
        implied_move_bp: implied_move,
        reason,
    };

    // ---- 任务选择 ----
    let overall_score = task_score_bp(&report.overall, targets.reward_ref);
    let mut task_biases = Vec::new();
    for f in &report.per_type {
        let score = task_score_bp(f, targets.reward_ref);
        let sufficient = f.sufficient;
        let delta = if sufficient {
            bias_delta_bp(score, overall_score, bounds)
        } else {
            0
        };
        let reason = if sufficient {
            format!(
                "样本 {} 条（充足）：质量 {}bp、平均收益 {}（归一 {}bp）→ 评分 {}bp，比整体 {}bp {}；偏好增量为 {delta}bp",
                f.sample,
                f.quality_bp,
                f.mean_reward,
                reward_bp(f.mean_reward, targets.reward_ref),
                score,
                overall_score,
                if score >= overall_score { "更好" } else { "更差" }
            )
        } else {
            format!(
                "样本 {} 条（不足 {} 条）→ 不改变偏好（宁可不学，也不拿噪声当信号）",
                f.sample, crate::feedback::MIN_SAMPLES
            )
        };
        task_biases.push(BiasExplanation {
            target: f.scope.clone(),
            kind: "task".to_string(),
            current_bias_bp: params.bias_of_task(&f.scope),
            score_bp: score,
            overall_bp: overall_score,
            violations: 0,
            implied_delta_bp: delta,
            reason,
        });
    }

    // ---- 协作对象选择 ----
    let mut peers: Vec<Did> = report.per_peer.iter().map(|p| p.peer.clone()).collect();
    for p in violations.peers() {
        if !peers.contains(&p) {
            peers.push(p);
        }
    }
    let mut peer_biases = Vec::new();
    for peer in peers {
        let quality = report
            .peer_stats(&peer)
            .map(|s| s.quality_bp)
            .unwrap_or(0);
        let sample = report.peer_stats(&peer).map(|s| s.sample).unwrap_or(0);
        let mut delta = if sample > 0 {
            bias_delta_bp(quality, report.overall.quality_bp, bounds)
        } else {
            0
        };
        let count = violations.count_of(&peer);
        let reason = if sample == 0 && count > 0 {
            format!(
                "没有合作样本，但有 {} 条违规记录 → 直接扣 {}bp（违规不需要再观察一次）",
                count, targets.violation_penalty_bp
            )
        } else if count > 0 {
            format!(
                "合作 {} 次、质量 {}bp（整体 {}bp）；另有 {} 条违规 → 质量增量 {}bp 再扣 {}bp",
                sample, quality, report.overall.quality_bp, count, delta, targets.violation_penalty_bp
            )
        } else {
            format!(
                "合作 {} 次、质量 {}bp（整体 {}bp）→ 偏好增量 {}bp",
                sample, quality, report.overall.quality_bp, delta
            )
        };
        if count > 0 {
            delta = delta.saturating_sub(targets.violation_penalty_bp);
        }
        peer_biases.push(BiasExplanation {
            target: explain_peer_tag(&peer),
            kind: "peer".to_string(),
            current_bias_bp: params.bias_of_peer(&peer),
            score_bp: quality,
            overall_bp: report.overall.quality_bp,
            violations: count,
            implied_delta_bp: delta,
            reason,
        });
    }

    let evidence = format!(
        "依据：本轮样本 {} 条（置信度 {}bp，门槛 {}bp）、综合信号窗口 {} 条经验、违规台账 {} 条",
        signals.sample,
        signals.confidence_bp,
        targets.min_confidence_bp,
        report.total,
        violations.total()
    );

    Ok(PolicyExplanation {
        price,
        task_biases,
        peer_biases,
        evidence,
    })
}

fn reward_bp(mean_reward: Credits, reward_ref: Credits) -> i64 {
    if reward_ref.get() <= 0 {
        return 0;
    }
    mean_reward
        .get()
        .saturating_mul(10_000)
        .checked_div(reward_ref.get())
        .unwrap_or(0)
        .clamp(0, 10_000)
}

/// 真实断言：解释蕴含的动作与 `adjust` 的实际动作一致、文本不含 DID、字段由数据推出。
pub fn self_check() -> Vec<SelfCheck> {
    use crate::experience::{Experience, ExperienceStore, Outcome};
    use crate::feedback::FeedbackAnalyser;
    use crate::policy::adjust;

    let mut checks = Vec::new();
    let peers: Vec<Did> = (0..2u8)
        .map(|s| au4a_core::AgentKeys::from_seed(&[0x40 + s; 32]).did())
        .collect();
    let mut store = match ExperienceStore::new(32) {
        Ok(s) => s,
        Err(_) => return vec![crate::check("explain.consistent", false, "经验库构造失败")],
    };
    for i in 0..8u64 {
        let (task_type, outcome, reward, peer) = if i % 2 == 0 {
            ("good.type", Outcome::Success, 80i64, peers[0].clone())
        } else {
            ("bad.type", Outcome::Failure, 0i64, peers[1].clone())
        };
        let context = format!("ctx-{i}");
        if let Ok(e) = Experience::new(
            &format!("t-{i}"),
            task_type,
            &context,
            "deliver",
            outcome,
            Credits(reward),
            i,
            &[peer],
        ) {
            let _ = store.record(e);
        }
    }
    let report = match FeedbackAnalyser::analyse(&store) {
        Ok(r) => r,
        Err(_) => return vec![crate::check("explain.consistent", false, "反馈分析失败")],
    };
    let bounds = PolicyBounds::default();
    let targets = PolicyTargets::default();
    let base = Signals {
        sample: 8,
        confidence_bp: 10_000,
        accept_rate_bp: Some(3_000),
        success_bp: 5_000,
        mean_reward: Credits(10),
        prev_mean_reward: Credits::ZERO,
        prev_price_dir: 0,
        violations: 0,
        reputation_delta: 0,
    };
    let params = PolicyParams::baseline();

    // 1) 解释蕴含的定价动作 == adjust 的实际动作（四种输入各测一遍）
    let cases = [
        ("accept=3000 应降价", Signals { accept_rate_bp: Some(3_000), ..base.clone() }),
        ("accept=9800 应提价", Signals { accept_rate_bp: Some(9_800), ..base.clone() }),
        ("死区停手", Signals { accept_rate_bp: Some(8_600), ..base.clone() }),
        (
            "信誉下降冻结涨价",
            Signals { accept_rate_bp: Some(9_800), reputation_delta: -40, ..base.clone() },
        ),
        (
            "收益下滑反向",
            Signals {
                accept_rate_bp: Some(3_000),
                prev_mean_reward: Credits(50),
                mean_reward: Credits(10),
                ..base.clone()
            },
        ),
    ];
    let mut consistent = true;
    let mut detail = Vec::new();
    for (label, signals) in cases {
        let explained = explain(&params, &report, &ViolationLog::new(), &signals, &bounds, &targets);
        let applied = adjust(
            &params,
            &report,
            &ViolationLog::new(),
            &signals,
            &bounds,
            &targets,
        );
        match (explained, applied) {
            (Ok(e), Ok(a)) => {
                let same = e.price.implied_move_bp == a.price_moved_bp;
                consistent &= same;
                detail.push(format!(
                    "{label}: 解释 {:?} vs 实际 {:?}",
                    e.price.implied_move_bp, a.price_moved_bp
                ));
                // 任务偏好增量也必须与 adjust 的移动量一致
                for b in &e.task_biases {
                    let moved = a.task_bias_moved_bp.get(&b.target).copied().unwrap_or(0);
                    let before = params.bias_of_task(&b.target);
                    let after = (before + b.implied_delta_bp)
                        .clamp(bounds.bias_min_bp, bounds.bias_max_bp);
                    consistent &= (after - before) == moved;
                }
            }
            _ => consistent = false,
        }
    }
    checks.push(crate::check(
        "explain.matches_behaviour",
        consistent,
        detail.join("；"),
    ));

    // 2) 解释可发布：不含 DID；文本由数据推出（含样本数与阈值）
    let signals = Signals {
        accept_rate_bp: Some(3_000),
        ..base.clone()
    };
    let explanation = explain(&params, &report, &ViolationLog::new(), &signals, &bounds, &targets);
    let (publishable, text_ok) = match &explanation {
        Ok(e) => {
            let text = e.summary() + &e.to_value().map(|v| v.to_string()).unwrap_or_default();
            (
                !text.contains("did:au4a:")
                    && e.peer_biases.iter().all(|b| b.target.starts_with("peer:")),
                e.task_biases.iter().all(|b| b.reason.contains("样本"))
                    && e.evidence.contains("门槛")
                    && e.to_value().is_ok(),
            )
        }
        Err(_) => (false, false),
    };
    checks.push(crate::check(
        "explain.publishable_and_derived",
        publishable && text_ok,
        format!("不含 DID={publishable}；每条理由都引用样本与阈值={text_ok}"),
    ));
    checks
}
