//! AU4A 节点库：把 10 条轨道聚合成一个可运行、可观察、可验证的整体。
//!
//! 这里体现项目的核心关系：**Agent 自主跑，人类只看**。
//! [`scenario`] 编排的是 Agent 自己的动作（注册、声明能力、协商、结算、学习、治理……），
//! [`observer`] 提供的是一组**只有 GET** 的只读投影。二者之间没有写通道。

pub mod identity;
pub mod observer;
pub mod scenario;

use au4a_core::{CoreError, CoreResult, SelfCheck};
use serde_json::{json, Value};

/// 节点运行时从 VERSION 文件读取真实版本号（crate 版本固定在系列基线，见根 Cargo.toml 注释）。
pub fn version() -> &'static str {
    include_str!("../../../VERSION").trim()
}

/// v2.6.4：**渲染快照 + 注入来源签名**的**唯一实现**。
///
/// 为什么必须提取：这段逻辑此前只存在于 `main.rs` 的一个闭包里，而集成测试
/// （`tests/provenance_e2e.rs`）只能**复刻**它——两者一旦漂移，测试就再也证明不了
/// "节点真正发出的面板是带签名的"。现在 `main.rs` 与测试调用同一份代码，漂移不可能发生。
///
/// 行为：
/// * `identity = None` → **匿名只读模式**：快照原样返回（与 v2.6.2 之前逐字节一致）；
/// * `identity = Some` → 用 v2.6.0 的口径签发报告（**只覆盖结果 + 收益**），
///   并把 `{node_did, sig, scheme, scope, report}` 写进快照的 `provenance`。
///   私钥只在这一刻被使用，快照里只出现 DID 与签名。
pub fn publish_snapshot(
    kernel: &au4a_kernel::Kernel,
    identity: Option<&identity::NodeIdentity>,
) -> String {
    let text = scenario::view_json(kernel);
    let Some(id) = identity else {
        return text;
    };
    let Ok(mut value) = serde_json::from_str::<Value>(&text) else {
        return text;
    };
    if let Ok(report) = au4a_kernel::Observer::report_signed(kernel, id.keys()) {
        value["provenance"] = json!({
            "node_did": report.node_did,
            "sig": report.sig,
            "scheme": "ed25519",
            "scope": au4a_kernel::ObserverReport::signed_routes(),
            // 把被签名的报告本体一并给出：验证方可直接对它跑 `verify_provenance()`，
            // 不必自己拼装签名载荷（拼装规则若漂移，"验签通过"就失去意义）。
            "report": report,
        });
    }
    au4a_core::canonicalize(&value).unwrap_or(text)
}

/// 轨道清单（轨道号、crate 名、标题、版本区间）。
pub fn track_table() -> Vec<String> {
    vec![
        row(
            au4a_kernel::TRACK,
            "au4a-kernel",
            au4a_kernel::TITLE,
            au4a_kernel::RANGE,
        ),
        row(
            au4a_capgraph::TRACK,
            "au4a-capgraph",
            au4a_capgraph::TITLE,
            au4a_capgraph::RANGE,
        ),
        row(
            au4a_negotiate::TRACK,
            "au4a-negotiate",
            au4a_negotiate::TITLE,
            au4a_negotiate::RANGE,
        ),
        row(
            au4a_state::TRACK,
            "au4a-state",
            au4a_state::TITLE,
            au4a_state::RANGE,
        ),
        row(
            au4a_economy::TRACK,
            "au4a-economy",
            au4a_economy::TITLE,
            au4a_economy::RANGE,
        ),
        row(
            au4a_safety::TRACK,
            "au4a-safety",
            au4a_safety::TITLE,
            au4a_safety::RANGE,
        ),
        row(
            au4a_learning::TRACK,
            "au4a-learning",
            au4a_learning::TITLE,
            au4a_learning::RANGE,
        ),
        row(
            au4a_council::TRACK,
            "au4a-council",
            au4a_council::TITLE,
            au4a_council::RANGE,
        ),
        row(
            au4a_chain::TRACK,
            "au4a-chain",
            au4a_chain::TITLE,
            au4a_chain::RANGE,
        ),
        row(
            au4a_scale::TRACK,
            "au4a-scale",
            au4a_scale::TITLE,
            au4a_scale::RANGE,
        ),
    ]
}

fn row(track: &str, krate: &str, title: &str, range: &str) -> String {
    format!("{track} {krate} {title} {range}")
}

/// 聚合全部 10 条轨道的自检项（`au4a-node verify` 与观察面板「结果」用的就是它）。
pub fn all_self_checks() -> Vec<SelfCheck> {
    let mut checks = au4a_kernel::self_check();
    checks.extend(au4a_capgraph::self_check());
    checks.extend(au4a_negotiate::self_check());
    checks.extend(au4a_state::self_check());
    checks.extend(au4a_economy::self_check());
    checks.extend(au4a_safety::self_check());
    checks.extend(au4a_learning::self_check());
    checks.extend(au4a_council::self_check());
    checks.extend(au4a_chain::self_check());
    checks.extend(au4a_scale::self_check());
    checks
}

/// 自检的 JSON 形式（可被工具消费）。
pub fn self_checks_json() -> Value {
    let checks = all_self_checks();
    let passed = checks.iter().filter(|c| c.passed).count();
    json!({
        "version": version(),
        "tracks": track_table().len(),
        "passed": passed,
        "total": checks.len(),
        "all_passed": au4a_core::all_passed(&checks),
        "checks": checks.iter().map(|c| json!({
            "track": c.track,
            "name": c.name,
            "passed": c.passed,
            "detail": c.detail,
        })).collect::<Vec<_>>(),
    })
}

/// 各轨道的只读产物摘要（观察层「结果」面板的输入之一）。
pub fn track_results() -> Value {
    let mut out = serde_json::Map::new();
    for (track, value) in [
        ("1.1", au4a_capgraph::results_json()),
        ("1.2", au4a_negotiate::results_json()),
        ("1.3", au4a_state::results_json()),
        ("1.4", au4a_economy::results_json()),
        ("1.5", au4a_safety::results_json()),
        ("1.6", au4a_learning::results_json()),
        ("1.7", au4a_council::results_json()),
        ("1.8", au4a_chain::results_json()),
        ("1.9", au4a_scale::results_json()),
    ] {
        match value {
            Ok(v) => {
                out.insert(track.into(), v);
            }
            Err(e) => {
                out.insert(track.into(), json!({ "error": e.to_string() }));
            }
        }
    }
    Value::Object(out)
}

/// 一条轨道的运行结果。
#[derive(Clone, Debug, serde::Serialize)]
pub struct TrackOutcome {
    pub track: String,
    pub title: String,
    pub ok: bool,
    pub detail: String,
}

/// 依次驱动 10 条轨道的 `scenario()`；每完成一条回调一次，观察面据此实时刷新「进度」。
pub fn run_tracks(
    kernel: &mut au4a_kernel::Kernel,
    mut on_step: impl FnMut(&au4a_kernel::Kernel),
) -> Vec<TrackOutcome> {
    let mut out: Vec<TrackOutcome> = Vec::new();
    let push = |kernel: &mut au4a_kernel::Kernel,
                on_step: &mut dyn FnMut(&au4a_kernel::Kernel),
                track: &str,
                title: &str,
                result: CoreResult<Value>,
                out: &mut Vec<TrackOutcome>| {
        let outcome = match result {
            Ok(v) => TrackOutcome {
                track: track.to_string(),
                title: title.to_string(),
                ok: true,
                detail: summarise(&v),
            },
            Err(e) => TrackOutcome {
                track: track.to_string(),
                title: title.to_string(),
                ok: false,
                detail: format!("scenario error: {e}"),
            },
        };
        kernel.emit(
            "track.scenario",
            format!(
                "{} {} -> {}",
                outcome.track,
                outcome.title,
                if outcome.ok { "ok" } else { "err" }
            ),
        );
        out.push(outcome);
        on_step(kernel);
    };

    let r = Ok(kernel.results_json());
    push(
        kernel,
        &mut on_step,
        au4a_kernel::TRACK,
        au4a_kernel::TITLE,
        r,
        &mut out,
    );
    let r = au4a_capgraph::scenario(kernel);
    push(
        kernel,
        &mut on_step,
        au4a_capgraph::TRACK,
        au4a_capgraph::TITLE,
        r,
        &mut out,
    );
    let r = au4a_negotiate::scenario(kernel);
    push(
        kernel,
        &mut on_step,
        au4a_negotiate::TRACK,
        au4a_negotiate::TITLE,
        r,
        &mut out,
    );
    let r = au4a_state::scenario(kernel);
    push(
        kernel,
        &mut on_step,
        au4a_state::TRACK,
        au4a_state::TITLE,
        r,
        &mut out,
    );
    let r = au4a_economy::scenario(kernel);
    push(
        kernel,
        &mut on_step,
        au4a_economy::TRACK,
        au4a_economy::TITLE,
        r,
        &mut out,
    );
    let r = au4a_safety::scenario(kernel);
    push(
        kernel,
        &mut on_step,
        au4a_safety::TRACK,
        au4a_safety::TITLE,
        r,
        &mut out,
    );
    let r = au4a_learning::scenario(kernel);
    push(
        kernel,
        &mut on_step,
        au4a_learning::TRACK,
        au4a_learning::TITLE,
        r,
        &mut out,
    );
    let r = au4a_council::scenario(kernel);
    push(
        kernel,
        &mut on_step,
        au4a_council::TRACK,
        au4a_council::TITLE,
        r,
        &mut out,
    );
    let r = au4a_chain::scenario(kernel);
    push(
        kernel,
        &mut on_step,
        au4a_chain::TRACK,
        au4a_chain::TITLE,
        r,
        &mut out,
    );
    let r = au4a_scale::scenario(kernel);
    push(
        kernel,
        &mut on_step,
        au4a_scale::TRACK,
        au4a_scale::TITLE,
        r,
        &mut out,
    );

    out
}

fn summarise(v: &Value) -> String {
    let s = au4a_core::canonicalize(v).unwrap_or_else(|_| "{}".to_string());
    if s.chars().count() <= 200 {
        s
    } else {
        format!("{}…", s.chars().take(200).collect::<String>())
    }
}

/// 观察层的完整投影（进度 + 结果 + 收益），HTTP 面板与 CLI 共用。
pub fn observe_all(kernel: &au4a_kernel::Kernel) -> Value {
    json!({
        "version": version(),
        "progress": kernel.observe_json(),
        "results": track_results(),
        "revenue": kernel.observe().ledger,
        "selfcheck": self_checks_json(),
        "tracks": track_table(),
    })
}

/// 供 CLI 使用的小工具：把错误转成可打印的一行。
pub fn describe(err: &CoreError) -> String {
    err.to_string()
}
