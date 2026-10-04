//! 轨道 1.6 示例：一个 Agent 的完整学习循环。
//!
//! 运行：
//!
//! ```powershell
//! $env:CARGO_TARGET_DIR = "E:\DS\_forangent\target\au4a-learning"
//! cd E:\DS\AU4A
//! cargo run -p au4a-learning --example learning_loop            # 默认种子
//! cargo run -p au4a-learning --example learning_loop -- 12345    # 指定种子
//! ```
//!
//! 输出是一份 JSON：经验收集 → 反馈分析 → 学习信号 → 行为调整 → 模型更新 → 效果评估，
//! 每段都带真实数字。示例本身不做任何计算，全部逻辑在 `au4a_learning::demo::demo_report`，
//! 因此测试断言的正是示例打印的内容。

fn main() {
    let seed = std::env::args()
        .nth(1)
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(au4a_learning::demo::DEMO_SEED);

    match au4a_learning::demo::demo_report(seed) {
        Ok(report) => match serde_json::to_string_pretty(&report) {
            Ok(text) => {
                println!("{text}");
                println!(
                    "\n[publishable: no did:au4a: in report = {}]",
                    au4a_learning::demo::demo_report_is_publishable(&report)
                );
            }
            Err(e) => {
                eprintln!("report 序列化失败: {e}");
                std::process::exit(1);
            }
        },
        Err(e) => {
            eprintln!("学习循环失败: {e}");
            std::process::exit(1);
        }
    }
}
