//! AU4A 轨道 1.6 — Individual Learning 个体学习（v1.6.1 → v1.6.10）。
//!
//! 个体学习的对象是**单个 Agent 自己的经验**，不是网络的全局模型：
//! Agent 记录自己做过什么、拿到了什么结果，然后**改变自己接下来的行为参数**
//! （定价、任务选择、协作对象选择），并在对照实验里量化「改变是否真的更好」。
//!
//! 本轨道刻意不做的事（做了就要如实降级证据等级）：
//!
//! * 不引入任何机器学习/线性代数依赖：更新规则是整数/万分比的加权线性更新。
//! * 不读墙钟、不做文件 I/O、不开网络、不起线程：时间来自 `LogicalClock`，
//!   落盘与传输由节点层负责，轨道本身是纯逻辑，因此可以被逐字节重放。
//! * 不声称「学会了」：每一次行为改变都必须由反馈数据推出，并有测试断言参数真的变了。
//!
//! # 错误映射约定
//!
//! `au4a-core` 的错误变体是冻结的，本轨道不新增错误类型，映射如下：
//!
//! | 情形 | 变体 |
//! |---|---|
//! | 字段缺失/为空/超长/非法枚举、容量非法、序列化里含重复条目 | [`CoreError::InvalidKind`] |
//! | JSON 解析/序列化失败 | [`CoreError::Encoding`] |
//! | 负数边界或负步长 | [`CoreError::NegativeAmount`] |
//! | 整数溢出 | [`CoreError::Overflow`] |
//! | 本地加密视图的密钥不正确（完整性标签不匹配） | [`CoreError::InvalidSignature`] |
//! | 回滚到一个不存在的代际（历史为空） | [`CoreError::InvalidVersion`] |
//!
//! 轨道内**串行**开发：每个小版本落地一个职责，并留下自检与证据。
//! 轨道间**零耦合**：只依赖 `au4a-core`（冻结基元）与 `au4a-kernel`（宿主内核）。

pub mod experience;
pub mod explain;
pub mod feedback;
pub mod model;
pub mod policy;
pub mod privacy;
pub mod rng;
pub mod scenario;
pub mod signal;
pub mod sim;
pub mod violation;

pub use experience::{Experience, ExperienceStore, Outcome, RecordOutcome, StoreStats};
pub use explain::{explain, explain_peer_tag, BiasExplanation, PolicyExplanation, PriceExplanation};
pub use feedback::{
    confidence_of, Feedback, FeedbackAnalyser, FeedbackReport, PeerFeedback, MIN_SAMPLES,
};
pub use model::{
    LearningModel, ModelConfig, ReputationLedger, UpdateRecord, REPUTATION_MAX, REPUTATION_MIN,
    REPUTATION_VIOLATION,
};
pub use policy::{
    adjust, bias_delta_bp, price_decision, price_direction, task_score_bp, PolicyAdjustment,
    PolicyBounds, PolicyParams, PolicyTargets, Signals,
};
pub use privacy::{open, peer_tag, publish, seal, PrivacyPolicy, PublicAggregate, PublicView, SealedBlob};
pub use rng::{hash64, SplitMix64};
pub use signal::{LearningSignal, SignalWeights, VIOLATION_UNIT_BP};
pub use sim::{
    ab_test, accept_rate_curve_bp, compare, effective_success_bp, peer_profiles, Comparison,
    MarketConfig, MarketRun, PeerProfile, RoundStats, TaskProfile, TASK_PROFILES,
};
pub use violation::{Violation, ViolationLog};

use au4a_core::{CoreResult, SelfCheck};
use serde_json::Value;

/// 轨道号。
pub const TRACK: &str = "1.6";
/// 轨道标题。
pub const TITLE: &str = "Individual Learning 个体学习";
/// 版本区间。
pub const RANGE: &str = "v1.6.1 → v1.6.10";
/// 已实现到的小版本（每落地一版就前移一格）。
pub const VERSION: &str = "v1.6.8";
/// 编译期存在性标记：确保 crate 名与轨道号一致。
pub const CRATE: &str = "au4a_learning";

/// 轨道自检：节点 `verify` 聚合它，观察层「结果」面板展示它。
///
/// 每一项都是**真实断言**：跑一遍本轨道的能力，比较结果。没有任何一项是「已接入」这种占位。
pub fn self_check() -> Vec<SelfCheck> {
    let mut checks = Vec::new();
    checks.extend(experience::self_check());
    checks.extend(feedback::self_check());
    checks.extend(violation::self_check());
    checks.extend(policy::self_check());
    checks.extend(signal::self_check());
    checks.extend(model::self_check());
    checks.extend(privacy::self_check());
    checks.extend(explain::self_check());
    checks.extend(sim::self_check());
    checks.extend(scenario::self_check());
    checks
}

/// 轨道产物摘要（只读投影的一部分）：在干净的默认内核上跑一次端到端流程并汇报。
pub fn results_json() -> CoreResult<Value> {
    let mut kernel = au4a_kernel::Kernel::new(au4a_kernel::KernelConfig::default());
    let summary = scenario::run(&mut kernel)?;
    let checks = self_check();
    let passed = checks.iter().filter(|c| c.passed).count();
    Ok(serde_json::json!({
        "track": TRACK,
        "title": TITLE,
        "range": RANGE,
        "version": VERSION,
        "checks": checks.len(),
        "checks_passed": passed,
        "all_passed": au4a_core::all_passed(&checks),
        "scenario": summary,
    }))
}

/// 端到端自有流程：用共享内核跑一遍本轨道的能力，返回 JSON 摘要。
///
/// 契约（不可改）：不 panic、不读文件、不开网络、不读墙钟；同样的输入给同样的输出。
/// 每一版迭代都让这里多做一件真实的事，而不是多打印一行字。
pub fn scenario(kernel: &mut au4a_kernel::Kernel) -> CoreResult<Value> {
    scenario::run(kernel)
}

/// 轨道内统一的 `CoreError → SelfCheck` 转换：失败也要给出可读证据，不允许 panic。
pub(crate) fn check(name: &str, passed: bool, detail: impl Into<String>) -> SelfCheck {
    if passed {
        SelfCheck::pass(TRACK, name, detail)
    } else {
        SelfCheck::fail(TRACK, name, detail)
    }
}
