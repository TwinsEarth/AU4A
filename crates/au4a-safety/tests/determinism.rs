//! v1.5.8 确定性重放：同一输入必须得到同一串字节。
//!
//! 这不是「顺手测一下」，而是本轨道的可验证性基础：如果 `scenario` 两次结果不同，
//! 那么文档里的任何一个哈希、任何一条链头都无法被别人复核，「可审计」就是空话。
//!
//! 注意：这里刻意**不读墙钟、不读文件、不用随机数**——`AgentKeys::from_seed` 提供
//! 确定性身份，`Kernel` 的逻辑时钟提供确定性时间。

use au4a_kernel::{Kernel, KernelConfig};
use au4a_safety::{scenario, self_check, SafetyOffice};

#[test]
fn two_runs_of_the_scenario_on_fresh_kernels_are_byte_identical() {
    let mut first = Kernel::new(KernelConfig::default());
    let mut second = Kernel::new(KernelConfig::default());
    let a = scenario(&mut first).unwrap();
    let b = scenario(&mut second).unwrap();
    assert_eq!(
        a, b,
        "同样种子、同样逻辑时钟必须得到同样的 JSON（含案件 id、事件链哈希）"
    );
    assert_eq!(a["chain"]["head"], b["chain"]["head"]);
    assert_eq!(a["case"]["id"], b["case"]["id"]);
    assert_eq!(a["case"]["status"], b["case"]["status"]);
    assert_eq!(a["case"]["status"], serde_json::json!("arbitrated"));
}

#[test]
fn the_scenario_is_repeatable_on_the_same_kernel() {
    let mut kernel = Kernel::new(KernelConfig::default());
    let first = scenario(&mut kernel).unwrap();
    let second = scenario(&mut kernel).unwrap();
    // 每次调用都新建一个 SafetyOffice，所以链长度一致（同样的步骤序列）；
    // 但逻辑时钟已经前移，时间戳参与内容寻址，因此案件 id 不同。
    assert_eq!(
        first["chain"]["len"], second["chain"]["len"],
        "同一步骤序列必须产生同样多的事件"
    );
    assert_ne!(
        first["case"]["id"], second["case"]["id"],
        "同一内核重复调用时时间不同，案件 id 必须反映新的时间"
    );
    assert_eq!(second["chain"]["ok"], serde_json::json!(true));
    assert_eq!(second["ledger_conserved"], serde_json::json!(true));
    // 内核上的账本状态是持久的：第二轮又真实罚没了一次（不是幂等去重）。
    assert_eq!(kernel.ledger().slashed(), au4a_core::Credits(14));
}

#[test]
fn the_ledger_and_the_journal_agree_after_repeated_runs() {
    let mut kernel = Kernel::new(KernelConfig::default());
    scenario(&mut kernel).unwrap();
    scenario(&mut kernel).unwrap();
    // 重复运行不是「幂等去重」：两次都真实执行（5 + 2 罚没 × 2 轮）。
    // 守恒式在重复运行后仍然成立（没有重复扣款、没有凭空发行）。
    kernel.ledger().check_conservation().unwrap();
    assert_eq!(kernel.ledger().slashed(), au4a_core::Credits(14));
}

#[test]
fn self_check_and_results_json_are_deterministic() {
    let first = self_check();
    let second = self_check();
    assert_eq!(first.len(), second.len());
    for (a, b) in first.iter().zip(second.iter()) {
        assert_eq!(a, b, "自检项必须可复现：{}", a.name);
        assert!(a.passed, "自检项 {} 未通过：{}", a.name, a.detail);
    }
    assert!(au4a_core::all_passed(&first));
}

#[test]
fn two_offices_built_from_the_same_seeds_agree_on_the_chain_head() {
    // 独立构造两个服务实例，跑同样的步骤序列，链头必须一致。
    let head = || -> String {
        let mut kernel = Kernel::new(KernelConfig::default());
        let service = au4a_safety::role_keys(au4a_safety::setup::ROLE_SERVICE);
        let reporter = au4a_safety::role_keys(au4a_safety::setup::ROLE_REPORTER);
        let subject = au4a_safety::role_keys(au4a_safety::setup::ROLE_SUBJECT);
        let arbiter = au4a_safety::role_keys(au4a_safety::setup::ROLE_ARBITER);
        for (keys, display) in [
            (&service, "safety-service"),
            (&reporter, "reporter-agent"),
            (&subject, "subject-agent"),
        ] {
            au4a_safety::ensure_agent(
                &mut kernel,
                keys,
                display,
                &["x"],
                au4a_core::Credits(20),
            )
            .unwrap();
        }
        let config =
            au4a_safety::SafetyConfig::single_arbiter(service.did(), arbiter.did());
        let mut office = SafetyOffice::new(config, service).unwrap();
        let payload = serde_json::json!({"deterministic": true, "delivered": false});
        let reference =
            au4a_safety::EvidenceRef::commit(au4a_safety::EvidenceKind::Transcript, &payload)
                .unwrap();
        let report = office
            .report(
                &mut kernel,
                &reporter,
                &subject.did(),
                au4a_safety::ViolationKind::NonDelivery,
                reference,
                &payload,
            )
            .unwrap();
        let order = au4a_safety::PenaltyOrder::new(
            report.id.clone(),
            subject.did(),
            au4a_safety::SanctionKind::StakeSlash,
            au4a_core::Credits(4),
            arbiter.did(),
            kernel.now(),
        )
        .sign(&arbiter)
        .unwrap();
        office.apply_penalty_order(&mut kernel, order).unwrap();
        office.chain_head()
    };

    assert_eq!(head(), head());
}
