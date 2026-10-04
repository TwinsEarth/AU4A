//! v1.8.7 集成测试（二）：端到端场景契约与「不伪造链上成功」的检查。
//!
//! 这些测试把场景当成对外契约来查：section 是否齐全、等级是否恒为 `cpu-proto`、
//! `real_network` 是否为 false、同一内核是否逐字节可复现、自检是否接线。

use au4a_chain::testnet::{ChainId, ChainTx, Testnet, ONCHAIN_GRADE};
use au4a_chain::{results_json, scenario, self_check, TITLE, TRACK};
use au4a_core::{all_passed, canonicalize};
use au4a_kernel::{Kernel, KernelConfig};
use serde_json::{json, Value};

fn run() -> (Kernel, Value) {
    let mut kernel = Kernel::new(KernelConfig::default());
    match scenario(&mut kernel) {
        Ok(value) => (kernel, value),
        Err(err) => panic!("场景必须成功，实际 {err}：{:?}", kernel.refusals()),
    }
}

#[test]
fn the_scenario_contract_has_every_section() {
    let (kernel, value) = run();
    for section in [
        "track",
        "title",
        "range",
        "scenario",
        "agents",
        "bridge",
        "rgb",
        "taproot",
        "erc8004",
        "x402",
        "routing",
        "reputation",
        "audit",
        "refusals",
        "testnet",
        "grade",
        "real_network",
        "conservation",
        "events",
    ] {
        assert!(
            value.get(section).is_some(),
            "场景 JSON 缺少 section: {section}"
        );
    }
    assert_eq!(value["track"], json!("1.8"));
    assert_eq!(value["grade"], json!("cpu-proto"));
    assert_eq!(value["real_network"], json!(false));
    assert_eq!(value["testnet"]["real_network"], json!(false));
    assert_eq!(value["testnet"]["grade"], json!("cpu-proto"));
    assert_eq!(value["bridge"]["consistent"], json!(true));
    assert!(value["refusals"].as_array().map(Vec::len).unwrap_or(0) >= 3);
    kernel.ledger().check_conservation().unwrap();
}

#[test]
fn the_scenario_is_byte_for_byte_reproducible() {
    let (_, a) = run();
    let (_, b) = run();
    assert_eq!(canonicalize(&a).unwrap(), canonicalize(&b).unwrap());
    assert_ne!(canonicalize(&a).unwrap(), canonicalize(&json!({})).unwrap());
}

#[test]
fn no_section_ever_claims_a_real_chain_or_a_non_cpu_proto_grade() {
    let (_, value) = run();
    // 递归检查：任何 grade 字段都只能是 cpu-proto，任何 real_network 都必须是 false。
    fn walk(value: &Value) {
        match value {
            Value::Object(map) => {
                if let Some(grade) = map.get("grade") {
                    assert_eq!(
                        grade,
                        &json!("cpu-proto"),
                        "出现了非 cpu-proto 的等级：{value}"
                    );
                }
                if let Some(real) = map.get("real_network") {
                    assert_eq!(real, &json!(false), "出现了声称真实网络的字段：{value}");
                }
                for child in map.values() {
                    walk(child);
                }
            }
            Value::Array(items) => items.iter().for_each(walk),
            _ => {}
        }
    }
    walk(&value);
    assert_eq!(ONCHAIN_GRADE.as_str(), "cpu-proto");
}

#[test]
fn the_pinned_numbers_of_the_end_to_end_flow_hold() {
    let (_, value) = run();
    // RGB：发行 400 → 250/150，流通量不变，最终化 1 个包。
    assert_eq!(value["rgb"]["supply"], json!(400));
    assert_eq!(value["rgb"]["circulating"], json!(400));
    assert_eq!(value["rgb"]["finalized"], json!(1));
    // Taproot：锚定 400，Merkle 验证通过。
    assert_eq!(value["taproot"]["amount_total"], json!(400));
    assert_eq!(value["taproot"]["verified_count"], json!(1));
    // ERC-8004：2 个身份、1 条反馈 8500bp → 权重 1700bp。
    assert_eq!(value["erc8004"]["identities"], json!(2));
    assert_eq!(value["erc8004"]["average_bp"], json!(8_500));
    assert_eq!(value["erc8004"]["weight_bp"], json!(1_700));
    // x402：发票 100 结清，托管先 +100 再回到 400。
    assert_eq!(value["x402"]["settled"], json!(true));
    assert_eq!(value["x402"]["escrow_after_claim"], json!(400));
    // 路由：400 → x402，费用 2、到账 398；不一致时缓办。
    assert_eq!(value["routing"]["decision"]["rail"], json!("x402"));
    assert_eq!(value["routing"]["decision"]["fee"], json!(2));
    assert_eq!(value["routing"]["decision"]["net"], json!(398));
    assert_eq!(
        value["routing"]["blocked_decision"]["reason"],
        json!("reconciliation_failed")
    );
    // 信誉：两个事件 → 6600/7000/5000/6600，综合 6300。
    assert_eq!(value["reputation"]["applied_events"], json!(2));
    assert_eq!(value["reputation"]["alice"]["overall_bp"], json!(6_300));
    // 双轨对账：托管 == 链上表示 == 400。
    assert_eq!(value["bridge"]["escrowed"], json!(400));
    assert_eq!(value["bridge"]["chain_supply"], json!(400));
    // 安全审计：风险如实登记（含未防护项），攻击全部被拦住。
    assert_eq!(value["audit"]["all_attacks_blocked"], json!(true));
    assert_eq!(value["audit"]["grade"], json!("cpu-proto"));
    assert!(
        value["audit"]["risks"]
            .as_array()
            .map(Vec::len)
            .unwrap_or(0)
            >= 8,
        "风险登记表过短"
    );
    assert!(
        value["audit"]["unresolved_count"].as_u64().unwrap_or(0) >= 3,
        "未防护/部分防护的风险必须如实保留"
    );
    assert!(
        value["audit"]["attacks"]
            .as_array()
            .map(|a| a.iter().all(|x| x["blocked"] == json!(true)))
            .unwrap_or(false),
        "攻击用例必须全部被拦住"
    );
}

#[test]
fn the_observable_layer_sees_the_named_refusals() {
    let mut kernel = Kernel::new(KernelConfig::default());
    scenario(&mut kernel).unwrap();
    // 场景里刻意留下的拒绝：RGB 未最终化最终化、不支持的原子交换、自评、无头寸/信誉转让等。
    let codes: Vec<&str> = kernel
        .refusals()
        .iter()
        .map(|(_, r)| r.code.as_str())
        .collect();
    assert!(
        codes.contains(&"timeout"),
        "应有未达最终性的拒绝：{codes:?}"
    );
    assert!(
        codes.contains(&"unsupported"),
        "应有不支持操作的拒绝：{codes:?}"
    );
    assert!(codes.contains(&"policy_denied"), "应有策略拒绝：{codes:?}");
    // 每条拒绝都带原因，没有空 detail。
    for (_, refusal) in kernel.refusals() {
        assert!(!refusal.reason.is_empty());
    }
}

#[test]
fn self_check_and_results_are_wired_and_all_pass() {
    let checks = self_check();
    assert!(all_passed(&checks), "{checks:?}");
    assert!(checks.len() >= 6, "自检项应覆盖各模块：{}", checks.len());
    for check in &checks {
        assert_eq!(check.track, TRACK);
        assert!(!check.detail.is_empty(), "{} 缺少细节", check.name);
    }
    let results = results_json().unwrap();
    assert_eq!(results["track"], json!(TRACK));
    assert_eq!(results["all_passed"], json!(true));
    assert_eq!(results["real_network"], json!(false));
    assert_eq!(results["grade"], json!("cpu-proto"));
    assert!(results["modules"].as_array().map(Vec::len).unwrap_or(0) >= 5);
    assert!(TITLE.contains("Cross-Chain"));
}

#[test]
fn a_tampered_chain_tx_is_refused_before_it_reaches_any_adapter() {
    let mut net = Testnet::new(ChainId::BtcRegtest, 1);
    let mut tx = ChainTx::new(
        ChainId::BtcRegtest,
        "rgb.issue",
        &au4a_core::AgentKeys::from_seed(&[1; 32]).did(),
        1,
        json!({ "supply": 10 }),
    )
    .unwrap();
    tx.payload = json!({ "supply": 1_000_000 });
    let err = net.accept(&tx).unwrap_err();
    assert_eq!(err.op, "rgb.issue");
    assert_eq!(err.code, au4a_core::RefusalCode::Malformed);
    assert_eq!(net.refusals().len(), 1);
}
