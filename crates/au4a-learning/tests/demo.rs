//! v1.6.9 示例测试：报告结构完整、五阶段全覆盖、可复现、可发布、数字真实。

use au4a_learning::demo::{
    demo_report, demo_report_is_publishable, DEMO_EXPERIENCES, DEMO_GENERATIONS, DEMO_SEED,
};

#[test]
fn demo_runs_the_whole_loop_with_real_numbers() {
    let report = demo_report(DEMO_SEED).unwrap();
    assert_eq!(report["seed"], DEMO_SEED);
    let steps = report["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 6, "六个阶段都要出现");

    let names: Vec<&str> = steps.iter().map(|s| s["step"].as_str().unwrap()).collect();
    assert!(names[0].contains("经验收集"));
    assert!(names[1].contains("反馈分析"));
    assert!(names[2].contains("学习信号"));
    assert!(names[3].contains("行为调整"));
    assert!(names[4].contains("模型更新"));
    assert!(names[5].contains("效果评估"));
    // 每一步都必须有非空证据
    assert!(steps
        .iter()
        .all(|s| !s["evidence"].as_str().unwrap().trim().is_empty()));

    // 1) 经验收集
    assert_eq!(report["steps"][0]["experiences"], DEMO_EXPERIENCES);
    assert!(report["steps"][0]["success_bp"].as_i64().unwrap() > 0);
    // 2) 反馈分析
    assert!(report["steps"][1]["by_type"].as_i64().unwrap() >= 2);
    assert!(report["steps"][1]["by_peer"].as_i64().unwrap() >= 1);
    // 3) 学习信号
    let composite = report["steps"][2]["composite_bp"].as_i64().unwrap();
    assert!((-10_000..=10_000).contains(&composite));
    assert!(composite > 0, "示例经验整体是成功的 → 综合信号应为正");
    // 4) 行为调整：真的动了（报价历史 → 接受率 41% 远低于目标 85% → 降价一步）
    assert_eq!(report["steps"][3]["changed"], serde_json::json!(true));
    assert_eq!(
        report["steps"][3]["price_moved_bp"],
        serde_json::json!(-500)
    );
    let quotes = &report["steps"][3]["quote_history"];
    assert_eq!(quotes["quoted"], au4a_learning::demo::DEMO_QUOTES);
    assert!(quotes["accepted"].as_i64().unwrap() > 0);
    assert!(quotes["accept_rate_bp"].as_i64().unwrap() < 8_500);
    // 5) 模型更新：动量让第一步小于满步长，之后逐步逼近
    let timeline = report["steps"][4]["timeline"].as_array().unwrap();
    assert_eq!(timeline.len(), DEMO_GENERATIONS as usize);
    let d0 = timeline[0]["price_drift_bp"].as_i64().unwrap();
    let d2 = timeline[2]["price_drift_bp"].as_i64().unwrap();
    assert!(
        d0.abs() < d2.abs(),
        "动量必须让第一步小于后续步长：{d0} vs {d2}"
    );
    assert_eq!(timeline[0]["generation"], 1);
    assert_eq!(timeline[2]["generation"], DEMO_GENERATIONS);
    // 6) 效果评估：真实改善
    assert_eq!(report["steps"][5]["improved"], serde_json::json!(true));
    let comparison = &report["steps"][5]["comparison"];
    assert!(comparison["success_lift_bp"].as_i64().unwrap() > 0);
    assert!(comparison["revenue_lift_credits"].as_i64().unwrap() > 0);
    assert!(comparison["params_differ"].as_bool().unwrap());
}

#[test]
fn demo_is_reproducible_and_seed_sensitive() {
    let a = demo_report(DEMO_SEED).unwrap();
    let b = demo_report(DEMO_SEED).unwrap();
    assert_eq!(a, b);
    assert_eq!(
        au4a_core::canonicalize(&a).unwrap(),
        au4a_core::canonicalize(&b).unwrap()
    );
    let other = demo_report(DEMO_SEED + 1).unwrap();
    assert_ne!(a, other, "换种子必须换一个市场（否则是伪复现）");
    // 每一个种子都必须给出「改善」或至少自洽的报告（不 panic、不返回错误）
    for seed in [1u64, 7, 99, 12345, 0x1616] {
        let r = demo_report(seed).unwrap();
        assert_eq!(r["steps"].as_array().unwrap().len(), 6);
        assert!(
            r["steps"][3]["changed"].as_bool().unwrap(),
            "seed={seed} 必须发生行为调整"
        );
        assert_eq!(r["steps"][4]["timeline"].as_array().unwrap().len(), 3);
    }
}

#[test]
fn demo_report_is_publishable_and_machine_readable() {
    let report = demo_report(DEMO_SEED).unwrap();
    assert!(demo_report_is_publishable(&report), "报告不得含 did:au4a:");
    assert!(
        !report.to_string().contains("示例上下文"),
        "报告不得含上下文原文"
    );
    // 规范 JSON 可编码（无浮点）
    au4a_core::canonicalize(&report).unwrap();
    // 解释是报告的一部分，且同样可发布
    let explanation = &report["explanation"];
    assert!(explanation["price"]["reason"]
        .as_str()
        .unwrap()
        .contains("accept"));
    assert!(!explanation.to_string().contains("did:au4a:"));
    // 策略前后都有公开投影（价格确实变了）
    let before = report["policy_before"]["price_bp"].as_i64().unwrap();
    let after = report["policy_after"]["price_bp"].as_i64().unwrap();
    assert_ne!(before, after);
    assert_eq!(before, 12_000);
    // 市场两组的公开投影都在
    assert!(
        report["market"]["control"]["success_rate_bp"]
            .as_i64()
            .unwrap()
            > 0
    );
    assert!(
        report["market"]["learning"]["success_rate_bp"]
            .as_i64()
            .unwrap()
            > 0
    );
}

#[test]
fn demo_constant_seed_is_stable_for_documentation() {
    // 文档里引用的默认种子必须稳定（改了就是文档漂移）
    assert_eq!(DEMO_SEED, 0x16_0909);
    assert_eq!(DEMO_EXPERIENCES, 18);
    assert_eq!(DEMO_GENERATIONS, 3);
    let report = demo_report(DEMO_SEED).unwrap();
    // 记录一组可写进文档的数字（测试日志留档）
    println!(
        "demo evidence: price {}→{}bp, success lift {}bp, revenue lift {} (+{}bp), violations {}→{}",
        report["policy_before"]["price_bp"],
        report["policy_after"]["price_bp"],
        report["steps"][5]["comparison"]["success_lift_bp"],
        report["steps"][5]["comparison"]["revenue_lift_credits"],
        report["steps"][5]["comparison"]["revenue_lift_bp"],
        report["steps"][5]["comparison"]["control_violations"],
        report["steps"][5]["comparison"]["learning_violations"],
    );
}
