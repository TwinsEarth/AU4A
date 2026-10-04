//! v1.6.2 反馈机制（Feedback）。
//!
//! 经验是**原始事实**，反馈是**可比较的统计**。学习循环的第二步是把经验聚合成
//! 按任务类型与按协作者两个维度的统计量，并显式给出「这批数据够不够支撑一次行为改变」。
//!
//! 两个刻意的选择：
//!
//! * 成功率 / 部分成功率 / 失败率用整数除法计算，且**失败率 = 10000 − 成功率 − 部分成功率**，
//!   让三个数在万分比上精确互补。若各自独立取整，三者和可能是 9999，长期累积会让
//!   「策略是否改善」的判断出现假信号。
//! * 每个统计都带 `confidence_bp` 与 `sufficient`：样本不足时**不学习**是正确行为，
//!   而不是「先学了再说」。`sufficient` 的唯一来源是 `sample >= MIN_SAMPLES`。

use std::collections::BTreeSet;

use au4a_core::{canonical_hash, CoreError, CoreResult, Credits, Did, SelfCheck};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::experience::{Experience, ExperienceStore, Outcome};

/// 反馈统计的样本下限：低于它时 `sufficient == false`，行为调整必须拒绝「学习」。
pub const MIN_SAMPLES: usize = 4;

/// 一个维度上的反馈统计（`scope` 是 `"overall"` 或任务类型名）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feedback {
    pub scope: String,
    pub sample: usize,
    pub success_bp: i64,
    pub partial_bp: i64,
    pub failure_bp: i64,
    /// 平均完成质量（含 Partial）：`Σ outcome.quality_bp() / sample`。
    pub quality_bp: i64,
    pub reward_total: Credits,
    /// 平均结算金额（整数向下取整）。
    pub mean_reward: Credits,
    pub confidence_bp: i64,
    pub sufficient: bool,
}

impl Feedback {
    /// 从一组经验计算统计。空集合给出全零且 `sufficient == false` 的报告（不是错误）。
    pub fn from_experiences(scope: &str, exps: &[&Experience]) -> CoreResult<Self> {
        let sample = exps.len();
        if sample == 0 {
            return Ok(Self {
                scope: scope.to_string(),
                sample: 0,
                success_bp: 0,
                partial_bp: 0,
                failure_bp: 0,
                quality_bp: 0,
                reward_total: Credits::ZERO,
                mean_reward: Credits::ZERO,
                confidence_bp: 0,
                sufficient: false,
            });
        }
        let mut successes = 0i64;
        let mut partials = 0i64;
        let mut quality_sum = 0i64;
        let mut reward_total = Credits::ZERO;
        for e in exps {
            match e.outcome {
                Outcome::Success => successes += 1,
                Outcome::Partial => partials += 1,
                Outcome::Failure => {}
            }
            quality_sum = quality_sum.checked_add(e.quality_bp()).ok_or(CoreError::Overflow)?;
            reward_total = reward_total.checked_add(e.reward)?;
        }
        let n = sample as i64;
        let success_bp = successes.saturating_mul(10_000) / n;
        let partial_bp = partials.saturating_mul(10_000) / n;
        // 三者和精确等于 10000：失败率由互补得到，避免各自取整引入的系统性缺口。
        let failure_bp = 10_000 - success_bp - partial_bp;
        Ok(Self {
            scope: scope.to_string(),
            sample,
            success_bp,
            partial_bp,
            failure_bp,
            quality_bp: quality_sum / n,
            reward_total,
            mean_reward: Credits(reward_total.get() / n),
            confidence_bp: confidence_of(sample),
            sufficient: sample >= MIN_SAMPLES,
        })
    }
}

/// 置信度（万分比）：`min(sample, MIN_SAMPLES) / MIN_SAMPLES`。
pub fn confidence_of(sample: usize) -> i64 {
    let capped = sample.min(MIN_SAMPLES) as i64;
    capped.saturating_mul(10_000) / MIN_SAMPLES as i64
}

/// 一个协作者的反馈统计（本地视图；对外发布时必须脱敏）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerFeedback {
    pub peer: Did,
    pub sample: usize,
    pub success_bp: i64,
    pub quality_bp: i64,
    pub mean_reward: Credits,
    pub confidence_bp: i64,
    pub sufficient: bool,
}

/// 反馈报告：整体 + 按任务类型 + 按协作者。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedbackReport {
    pub total: usize,
    pub overall: Feedback,
    pub per_type: Vec<Feedback>,
    pub per_peer: Vec<PeerFeedback>,
}

impl FeedbackReport {
    pub fn to_value(&self) -> CoreResult<Value> {
        serde_json::to_value(self).map_err(|_| CoreError::Encoding)
    }

    pub fn digest(&self) -> CoreResult<String> {
        canonical_hash(&self.to_value()?)
    }

    pub fn type_stats(&self, task_type: &str) -> Option<&Feedback> {
        self.per_type.iter().find(|f| f.scope == task_type)
    }

    pub fn peer_stats(&self, peer: &Did) -> Option<&PeerFeedback> {
        self.per_peer.iter().find(|p| &p.peer == peer)
    }

    /// 对外公开投影（v1.6.2 起）：**不含协作者 DID 明细、不含任务标识与上下文**，
    /// 只发布聚合量；协作者维度只发布计数与最佳/最差质量区间。
    ///
    /// 完整脱敏（含本地加密视图）在 v1.6.6 落地；这里先保证「默认发布路径不泄露明细」。
    pub fn public_json(&self) -> CoreResult<Value> {
        let best_quality = self.per_peer.iter().map(|p| p.quality_bp).max();
        let worst_quality = self.per_peer.iter().map(|p| p.quality_bp).min();
        Ok(serde_json::json!({
            "total": self.total,
            "overall": &self.overall,
            "per_type": &self.per_type,
            "peers_observed": self.per_peer.len(),
            "peers_sufficient": self.per_peer.iter().filter(|p| p.sufficient).count(),
            "peer_quality_best_bp": best_quality,
            "peer_quality_worst_bp": worst_quality,
        }))
    }

    /// 真实断言：整数统计精确、互补关系成立、样本不足不假装充分、公开投影不含 DID。
    pub fn self_check() -> Vec<SelfCheck> {
        let mut checks = Vec::new();

        // 1) 精确统计 + 三率和为 10000
        let peers: Vec<Did> = (0..2u8)
            .map(|s| au4a_core::AgentKeys::from_seed(&[0x70 + s; 32]).did())
            .collect();
        let mut store = match ExperienceStore::new(16) {
            Ok(s) => s,
            Err(_) => {
                return vec![crate::check(
                    "feedback.exact_statistics",
                    false,
                    "经验库构造失败",
                )]
            }
        };
        let rows = [
            ("a-1", Outcome::Success, 40i64),
            ("a-2", Outcome::Success, 40),
            ("a-3", Outcome::Partial, 20),
            ("a-4", Outcome::Failure, 0),
        ];
        let mut recorded = 0usize;
        for (id, outcome, reward) in rows {
            let context = format!("ctx-{id}");
            let built = Experience::new(
                id,
                "translate.en-zh",
                &context,
                "deliver",
                outcome,
                Credits(reward),
                1,
                &peers[..1],
            )
            .and_then(|e| store.record(e));
            if built.is_ok() {
                recorded += 1;
            }
        }
        let report = FeedbackAnalyser::analyse(&store);
        let (ok, detail) = match &report {
            Ok(r) => {
                let o = &r.overall;
                let exact = recorded == 4
                    && o.sample == 4
                    && o.success_bp == 5_000
                    && o.partial_bp == 2_500
                    && o.failure_bp == 2_500
                    && o.quality_bp == 6_250
                    && o.reward_total == Credits(100)
                    && o.mean_reward == Credits(25)
                    && o.confidence_bp == 10_000
                    && o.sufficient;
                (
                    exact,
                    format!(
                        "sample={} success={} partial={} failure={} quality={} mean_reward={}",
                        o.sample, o.success_bp, o.partial_bp, o.failure_bp, o.quality_bp, o.mean_reward
                    ),
                )
            }
            Err(e) => (false, format!("分析失败: {e:?}")),
        };
        checks.push(crate::check("feedback.exact_statistics", ok, detail));

        // 2) 样本不足 → 不充分；空库 → 零报告而不是错误
        let mut tiny = match ExperienceStore::new(8) {
            Ok(s) => s,
            Err(_) => return checks,
        };
        let context = "ctx-tiny".to_string();
        if let Ok(e) = Experience::new("t-1", "x", &context, "a", Outcome::Success, Credits(1), 1, &[]) {
            let _ = tiny.record(e);
        }
        let under = FeedbackAnalyser::analyse(&tiny)
            .map(|r| !r.overall.sufficient && r.overall.confidence_bp == 2_500)
            .unwrap_or(false);
        let empty_ok = ExperienceStore::new(4)
            .and_then(|s| FeedbackAnalyser::analyse(&s))
            .map(|r| r.total == 0 && r.per_type.is_empty() && !r.overall.sufficient)
            .unwrap_or(false);
        checks.push(crate::check(
            "feedback.evidence_threshold",
            under && empty_ok,
            format!("样本 1/4 时 sufficient=false 且 confidence=2500bp：{under}；空库给出零报告：{empty_ok}"),
        ));

        // 3) 公开投影不含 DID 明细
        let redacted = report
            .as_ref()
            .ok()
            .and_then(|r| r.public_json().ok())
            .map(|v| {
                let text = v.to_string();
                !text.contains("did:au4a:") && v.get("per_peer").is_none()
            })
            .unwrap_or(false);
        checks.push(crate::check(
            "feedback.public_projection_redacted",
            redacted,
            "公开投影不含 did:au4a: 明细，也不含 per_peer 字段",
        ));
        checks
    }
}

/// 反馈分析器：经验库 → 反馈报告。
///
/// 无状态、无 I/O：同样的经验库永远给出同样的报告（报告摘要可用来断言可复现性）。
pub struct FeedbackAnalyser;

impl FeedbackAnalyser {
    pub fn analyse(store: &ExperienceStore) -> CoreResult<FeedbackReport> {
        let all: Vec<&Experience> = store.entries().iter().collect();
        let overall = Feedback::from_experiences("overall", &all)?;

        let mut per_type = Vec::new();
        for task_type in store.task_types() {
            let subset: Vec<&Experience> = store.by_task_type(&task_type).collect();
            per_type.push(Feedback::from_experiences(&task_type, &subset)?);
        }

        let mut participants: BTreeSet<Did> = BTreeSet::new();
        for e in store.entries() {
            for p in &e.peer_agents {
                participants.insert(p.clone());
            }
        }
        let mut per_peer = Vec::new();
        for peer in participants {
            let subset: Vec<&Experience> = store
                .entries()
                .iter()
                .filter(|e| e.peer_agents.contains(&peer))
                .collect();
            let stats = Feedback::from_experiences("peer", &subset)?;
            per_peer.push(PeerFeedback {
                peer,
                sample: stats.sample,
                success_bp: stats.success_bp,
                quality_bp: stats.quality_bp,
                mean_reward: stats.mean_reward,
                confidence_bp: stats.confidence_bp,
                sufficient: stats.sufficient,
            });
        }

        Ok(FeedbackReport {
            total: all.len(),
            overall,
            per_type,
            per_peer,
        })
    }
}

/// 模块级自检入口（`crate::self_check` 聚合它）。
pub fn self_check() -> Vec<SelfCheck> {
    FeedbackReport::self_check()
}
