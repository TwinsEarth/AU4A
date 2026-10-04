//! AU4A 轨道 1.5 — Safety API 安全 API（v1.5.1 → v1.5.10）。
//!
//! 安全接口是 **Agent 侧**的：Agent 自己查权限边界、自己举报（带可验证证据哈希）、
//! 自己被处罚后自己提交申诉证据、自己查询处罚记录、自己按案件订阅状态变更。
//! 这不是一个「人类控制台」——本 crate 里没有审批入口、没有管理员身份、没有人工复核阶段。
//!
//! 三条不可协商的不变式（每一版都在测试里被断言）：
//!
//! 1. **无罪不罚**：未被裁决确认的举报不改变任何 Agent 的余额与信誉。
//! 2. **证据可验证**：举报与申诉都必须携带可复算的证据哈希，缺失或伪造一律拒绝。
//! 3. **状态可审计**：每次状态变更写入哈希链事件（`prev` + 内容 → `hash`），断链可检测。
//!
//! 轨道内**串行**开发：每个小版本落地一个职责（v1.5.1 权限查询 → v1.5.10 审计），
//! 轨道间**零耦合**：只依赖 `au4a-core`（冻结基元）与 `au4a-kernel`（宿主内核）；
//! 与 `au4a-council` 的协作只走**数据契约**（可序列化 JSON），不互相依赖 crate。

pub mod config;
pub mod permission;
pub mod setup;

use au4a_core::{CoreResult, SelfCheck};
use au4a_kernel::Kernel;
use serde_json::{json, Value};

pub use config::SafetyConfig;
pub use permission::{
    query_permissions, stake_requirement, DenialReason, DeniedPermission, Permission,
    PermissionBoundary, PermissionQuery, StakeGate,
};
pub use setup::{ensure_agent, keys as role_keys, seed as role_seed};

/// 轨道号。
pub const TRACK: &str = "1.5";
/// 轨道标题。
pub const TITLE: &str = "Safety API 安全 API";
/// 版本区间。
pub const RANGE: &str = "v1.5.1 → v1.5.10";
/// 当前小版本（每个小版本落地时前移）。
pub const CURRENT: &str = "v1.5.1";

/// 编译期存在性标记：确保 crate 名与轨道号一致。
pub const CRATE: &str = "au4a_safety";

/// 轨道自检：节点 `verify` 聚合它，观察层「结果」面板展示它。
///
/// 每一项都是**跑出来的断言**（在私有内核上真跑一遍 `scenario` 后检查真实状态），
/// 不是占位文本。
pub fn self_check() -> Vec<SelfCheck> {
    let mut kernel = Kernel::new(au4a_kernel::KernelConfig::default());
    let mut checks = Vec::new();

    let run = scenario(&mut kernel);
    checks.push(match &run {
        Ok(_) => SelfCheck::pass(TRACK, "scenario.completed", "v1.5.1 端到端流程无错误返回"),
        Err(err) => SelfCheck::fail(TRACK, "scenario.completed", format!("scenario 失败: {err}")),
    });

    let config = demo_config();
    let reporter = role_keys(setup::ROLE_REPORTER).did();
    let arbiter = role_keys(setup::ROLE_ARBITER).did();
    let probe = role_keys(0xEE).did();

    let reporter_boundary = PermissionBoundary::of(&kernel, &config, &reporter);
    checks.push(if reporter_boundary.allows(Permission::ReportViolation) {
        SelfCheck::pass(
            TRACK,
            "permission.reporter_can_report",
            format!(
                "已注册 Agent 的 allowed={} denied={}，举报权可达",
                reporter_boundary.allowed.len(),
                reporter_boundary.denied.len()
            ),
        )
    } else {
        SelfCheck::fail(TRACK, "permission.reporter_can_report", "举报权不可达")
    });

    let probe_boundary = PermissionBoundary::of(&kernel, &config, &probe);
    checks.push(
        if !probe_boundary.registered
            && probe_boundary.allowed == vec![Permission::Register]
            && probe_boundary.denial(Permission::SendMessage)
                == Some(&DenialReason::NotRegistered)
        {
            SelfCheck::pass(
                TRACK,
                "permission.unregistered_is_structured",
                "未注册 DID 只能自助注册，其余权限带 not_registered 码被拒",
            )
        } else {
            SelfCheck::fail(TRACK, "permission.unregistered_is_structured", "未注册者的边界不结构化")
        },
    );

    let arbiter_boundary = PermissionBoundary::of(&kernel, &config, &arbiter);
    checks.push(if arbiter_boundary.allows(Permission::IssueVerdict) {
        SelfCheck::pass(
            TRACK,
            "permission.arbiter_only",
            "配置中的仲裁者可出具裁决，其余 Agent 得到 arbiter_only",
        )
    } else {
        SelfCheck::fail(TRACK, "permission.arbiter_only", "仲裁者权限不可达")
    });

    checks.push(match kernel.ledger().check_conservation() {
        Ok(()) => SelfCheck::pass(
            TRACK,
            "ledger.conservation",
            format!("Σ可用+Σ锁定+罚没 == 发行（minted={}）", kernel.ledger().minted()),
        ),
        Err(err) => SelfCheck::fail(TRACK, "ledger.conservation", err.to_string()),
    });

    checks
}

/// 轨道产物摘要（只读投影的一部分）。
pub fn results_json() -> CoreResult<Value> {
    let mut kernel = Kernel::new(au4a_kernel::KernelConfig::default());
    let scenario_value = scenario(&mut kernel)?;
    let checks = self_check();
    Ok(json!({
        "track": TRACK,
        "title": TITLE,
        "range": RANGE,
        "current": CURRENT,
        "checks": checks.len(),
        "checks_passed": checks.iter().filter(|c| c.passed).count(),
        "scenario": scenario_value,
    }))
}

/// 端到端自有流程：用共享内核跑一遍本轨道的能力，返回 JSON 摘要。
///
/// 契约（不可改）：不 panic、不读文件、不开网络、不读墙钟；同样的输入给同样的输出。
/// 每一版迭代都应该让这里多做一件真实的事，而不是多打印一行字。
pub fn scenario(kernel: &mut Kernel) -> CoreResult<Value> {
    let reporter = role_keys(setup::ROLE_REPORTER);
    let subject = role_keys(setup::ROLE_SUBJECT);
    let arbiter = role_keys(setup::ROLE_ARBITER);
    let service = role_keys(setup::ROLE_SERVICE);

    ensure_agent(kernel, &reporter, "reporter-agent", &["audit.report"], au4a_core::Credits(20))?;
    ensure_agent(kernel, &subject, "subject-agent", &["deliver.task"], au4a_core::Credits(20))?;
    ensure_agent(kernel, &arbiter, "arbiter-agent", &["arbitrate.case"], au4a_core::Credits(20))?;
    ensure_agent(kernel, &service, "safety-service", &["safety.api"], au4a_core::Credits(20))?;

    let config = SafetyConfig::single_arbiter(service.did(), arbiter.did());
    let reporter_boundary = PermissionBoundary::of(kernel, &config, &reporter.did());
    let arbiter_boundary = PermissionBoundary::of(kernel, &config, &arbiter.did());
    let probe_boundary = PermissionBoundary::of(kernel, &config, &role_keys(0xEE).did());

    kernel.emit(
        &format!("{TRACK}.scenario"),
        format!("{CURRENT} 权限边界查询：{} 个权限点", Permission::ALL.len()),
    );

    Ok(json!({
        "track": TRACK,
        "title": TITLE,
        "version": CURRENT,
        "agents": kernel.agent_count(),
        "permission_points": Permission::ALL.len(),
        "reporter": {
            "allowed": reporter_boundary.allowed.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
            "denied": reporter_boundary.denied.len(),
        },
        "arbiter": {
            "can_issue_verdict": arbiter_boundary.allows(Permission::IssueVerdict),
        },
        "probe_unregistered": {
            "registered": probe_boundary.registered,
            "allowed": probe_boundary.allowed.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
            "send_message_denial": probe_boundary.denial(Permission::SendMessage).map(|r| r.code()),
        },
        "ledger_conserved": kernel.ledger().check_conservation().is_ok(),
    }))
}

/// 演示用配置：服务身份 + 单一仲裁者，全部来自确定性种子。
pub fn demo_config() -> SafetyConfig {
    let arbiter = role_keys(setup::ROLE_ARBITER);
    let service = role_keys(setup::ROLE_SERVICE);
    SafetyConfig::single_arbiter(service.did(), arbiter.did())
}
