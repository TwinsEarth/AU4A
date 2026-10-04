//! `governance_demo`——轨道 1.7 的可运行示例（v1.7.9）。
//!
//! 运行：
//!
//! ```powershell
//! $env:CARGO_TARGET_DIR = "E:\DS\_forangent\target\au4a-council"
//! cargo run -p au4a-council --example governance_demo
//! ```
//!
//! 它会做四件事并把结果打成 JSON：
//!
//! 1. 选出五类委员会（并让一批信誉为零的空壳 DID 试图刷票——全部被忽略）；
//! 2. 由委员提案、表决（其中一轮被双签作废、重开后通过）、执行成策略变更；
//! 3. 人类否决第二条动议——决议只能被阻断，执行被拒；
//! 4. 安全委员会走紧急通道即时下发策略，事后由治理确认；
//! 最后打印全部自检项与不变式状态。
//!
//! 示例不读文件、不开网络、不读墙钟：只依赖 `au4a-core` / `au4a-kernel` 的逻辑时钟。

use au4a_core::{all_passed, CoreError};
use au4a_kernel::{Kernel, KernelConfig};

fn main() -> Result<(), CoreError> {
    let mut kernel = Kernel::new(KernelConfig::default());

    let scenario = au4a_council::scenario(&mut kernel)?;
    println!("== scenario ==");
    println!(
        "{}",
        serde_json::to_string_pretty(&scenario).unwrap_or_default()
    );

    let checks = au4a_council::self_check();
    println!("== self_check ==");
    for check in &checks {
        println!(
            "[{}] {} — {}",
            if check.passed { "ok" } else { "FAIL" },
            check.name,
            check.detail
        );
    }
    println!("all_passed = {}", all_passed(&checks));

    let results = au4a_council::results_json()?;
    println!("== results ==");
    println!(
        "{}",
        serde_json::to_string_pretty(&results).unwrap_or_default()
    );

    println!("== claims ==");
    println!("{}", au4a_council::claims::summary());
    for claim in au4a_council::claims::CLAIMS {
        println!(
            "- {:<28} [{}] test={}",
            claim.id,
            claim.grade.as_str(),
            claim.test
        );
    }

    let report = au4a_council::invariants::replay(0x1707, 72)?;
    println!("== replay ==");
    println!(
        "seed={} steps={} proposals={} rounds={} voided={} executed={} blocked={} refusals={} violations={} digest={}",
        report.seed,
        report.steps,
        report.proposals,
        report.rounds,
        report.voided_rounds,
        report.executed,
        report.blocked,
        report.refusals,
        report.violations.len(),
        report.digest
    );
    Ok(())
}
