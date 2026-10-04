//! AU4A 节点二进制。
//!
//! 人能启动的只有这一个东西，能看到的也只有只读面板：
//! * `run` —— Agent 自主跑（注册 → 声明能力 → 10 条轨道依次运行），人类不参与；
//! * `observe` —— 打印只读投影（进度 / 结果 / 收益），一次性快照；
//! * `verify` —— 聚合 10 条轨道的自检并给出退出码（部署验证用）；
//! * `tracks` / `version` —— 清单与版本。

use std::process::exit;
use std::time::Duration;

use au4a_node::observer::{Observer, SharedView};
use au4a_node::scenario::{self, DEFAULT_AGENTS};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let flag = |name: &str| args.iter().any(|a| a == name);
    let value = |name: &str| -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };

    match cmd {
        "version" | "--version" | "-V" => println!("au4a-node {}", au4a_node::version()),

        "tracks" => {
            if flag("--json") {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&au4a_node::track_table()).unwrap()
                );
            } else {
                println!(
                    "au4a-node {} — 10 条轨道（10 个中版本）",
                    au4a_node::version()
                );
                for line in au4a_node::track_table() {
                    println!("  {line}");
                }
            }
        }

        "verify" => {
            let checks = au4a_node::all_self_checks();
            let passed = checks.iter().filter(|c| c.passed).count();
            let all = au4a_core::all_passed(&checks);
            if flag("--json") {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&au4a_node::self_checks_json()).unwrap()
                );
            } else {
                println!(
                    "au4a-node {} 自检：{passed}/{} 通过",
                    au4a_node::version(),
                    checks.len()
                );
                for c in &checks {
                    println!(
                        "  [{}] {:<6} {:<28} {}",
                        if c.passed { "ok" } else { "FAIL" },
                        c.track,
                        c.name,
                        c.detail
                    );
                }
            }
            exit(if all { 0 } else { 1 });
        }

        "observe" => {
            let agents = value("--agents")
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_AGENTS);
            let (kernel, outcomes, dids) = match scenario::run_full(agents, |_| {}) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("scenario failed: {e}");
                    exit(2);
                }
            };
            let summary = scenario::summary_json(&kernel, &outcomes, &dids)
                .unwrap_or(serde_json::Value::Null);
            if flag("--json") || flag("--once") {
                println!("{}", serde_json::to_string_pretty(&summary).unwrap());
            } else {
                println!("au4a-node {} 观察投影（只读）", au4a_node::version());
                println!(
                    "  Agent：{}　已投递消息：{}　拒绝：{}",
                    dids.len(),
                    kernel.observe().messages_delivered,
                    kernel.observe().refusal_count
                );
                println!(
                    "  收益：总量 {}　发行 {}　罚没 {}",
                    kernel.observe().ledger.total,
                    kernel.observe().ledger.minted,
                    kernel.observe().ledger.slashed
                );
                println!("  轨道结果：");
                for o in &outcomes {
                    println!(
                        "    [{}] {:<6} {}",
                        if o.ok { "ok" } else { "FAIL" },
                        o.track,
                        o.title
                    );
                }
            }
        }

        "run" => {
            let agents = value("--agents")
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_AGENTS);
            let observe = value("--observe");

            // 观察面先起来（人类可以立刻开始看），随后 Agent 才开跑。
            let placeholder = au4a_core::canonicalize(&serde_json::json!({
                "version": au4a_node::version(),
                "progress": { "agents": [], "messages_delivered": 0, "refusal_count": 0, "now": 0, "progress": [] },
                "results": { "tracks": {} },
                "revenue": { "accounts": {}, "minted": 0, "slashed": 0, "total": 0 },
                "selfcheck": au4a_node::self_checks_json(),
                "tracks": au4a_node::track_table(),
            }))
            .unwrap_or_else(|_| "{}".to_string());

            let view = SharedView::new(placeholder);
            let mut observer = match &observe {
                Some(addr) => match Observer::start(addr, view.clone()) {
                    Ok(o) => {
                        println!(
                            "只读观察面板：{}/（仅 GET；POST/PUT/PATCH/DELETE 一律 405）",
                            o.url()
                        );
                        Some(o)
                    }
                    Err(e) => {
                        eprintln!("cannot start observer on {addr}: {e}");
                        exit(2);
                    }
                },
                None => None,
            };

            let started = std::time::Instant::now();
            let (kernel, outcomes, dids) = match scenario::run_full(agents, |k| {
                if observe.is_some() {
                    view.update(scenario::view_json(k));
                }
            }) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("scenario failed: {e}");
                    exit(2);
                }
            };
            if observe.is_some() {
                view.update(scenario::view_json(&kernel));
            }

            let summary = scenario::summary_json(&kernel, &outcomes, &dids)
                .unwrap_or(serde_json::Value::Null);
            let ok = outcomes.iter().filter(|o| o.ok).count();
            if flag("--json") {
                println!("{}", serde_json::to_string_pretty(&summary).unwrap());
            } else {
                println!(
                    "au4a-node {} — {} 个 Agent 自主运行完成：{}/{} 条轨道 ok，耗时 {:?}",
                    au4a_node::version(),
                    dids.len(),
                    ok,
                    outcomes.len(),
                    started.elapsed()
                );
                for o in &outcomes {
                    println!(
                        "  [{}] {:<6} {:<34} {}",
                        if o.ok { "ok" } else { "FAIL" },
                        o.track,
                        o.title,
                        o.detail
                    );
                }
            }

            if let Some(o) = observer.as_mut() {
                println!("观察面板继续服务：{}/（Ctrl-C 退出）", o.url());
                loop {
                    std::thread::sleep(Duration::from_secs(3600));
                }
                // 不可达：保留 stop 语义的显式调用，避免 Drop 顺序歧义
                #[allow(unreachable_code)]
                o.stop();
            }
        }

        _ => {
            println!(
                "au4a-node {}\n用法:\n  au4a-node run [--agents N] [--observe 127.0.0.1:8787] [--json]\n  au4a-node observe [--once] [--json]\n  au4a-node verify [--json]\n  au4a-node tracks [--json]\n  au4a-node version",
                au4a_node::version()
            );
            exit(if cmd.is_empty() { 0 } else { 2 });
        }
    }
}
