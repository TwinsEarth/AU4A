//! v1.0.3 人类观察层的集成测试（只用公开 API）。
//!
//! 重点是**结构性只读**：不是「我们约定不去写」，而是「入口枚举里根本不存在写路径」。
//! 测试枚举每一条路由、每一个能力，并把渲染前后的内核状态逐字节比对。

use au4a_core::all_passed;
use au4a_kernel::{
    observer_api, observer_self_checks, scenario, Kernel, KernelConfig, Observer,
    ObserverCapability, ObserverRoute, TRACK,
};

fn rich_kernel() -> Kernel {
    let mut k = Kernel::new(KernelConfig::default());
    scenario(&mut k).unwrap();
    k
}

#[test]
fn the_human_entry_surface_can_be_enumerated_and_contains_no_write_path() {
    let api = observer_api();
    assert_eq!(api["writable"], false);
    assert_eq!(api["capabilities"].as_array().map(|a| a.len()), Some(1));
    assert_eq!(api["capabilities"][0], "read");
    let routes = api["routes"].as_array().cloned().unwrap_or_default();
    assert_eq!(routes.len(), ObserverRoute::ALL.len());
    for route in &routes {
        assert_eq!(route["capability"], "read");
        assert_eq!(route["writable"], false);
        assert_eq!(route["effects"].as_array().map(|a| a.len()), Some(0));
        let name = route["route"].as_str().unwrap_or("");
        for verb in [
            "write",
            "approve",
            "grant",
            "schedule",
            "price",
            "adjudicate",
            "propose",
        ] {
            assert!(!name.contains(verb), "人类入口里出现了写动词：{name}");
        }
    }
    // 能力枚举本身只有 Read：写能力连类型都没有。
    assert_eq!(ObserverCapability::ALL, [ObserverCapability::Read]);
    for route in ObserverRoute::ALL {
        assert_eq!(route.capability(), ObserverCapability::Read);
        assert!(route.effects().is_empty());
    }
}

#[test]
fn rendering_a_live_kernel_does_not_change_it() {
    let k = rich_kernel();
    let before_registry = k.registry_fingerprint().unwrap();
    let before_view = k.observe_json();
    let before_now = k.now();

    let first = Observer::report(&k);
    let second = Observer::report(&k);

    assert_eq!(first, second, "同一状态必须渲染出同一份报告");
    assert_eq!(first.fingerprint().unwrap(), second.fingerprint().unwrap());
    assert!(first.all_read_only());
    assert_eq!(before_registry, k.registry_fingerprint().unwrap());
    assert_eq!(before_view, k.observe_json());
    assert_eq!(before_now, k.now());
    assert_eq!(first.network_id, "au4a-local");
}

#[test]
fn three_projections_show_progress_results_and_yield() {
    let k = rich_kernel();
    let report = Observer::report(&k);
    assert_eq!(report.projections.len(), 3);

    let progress = report.projection(ObserverRoute::Progress).unwrap();
    assert_eq!(progress.route, "progress");
    assert_eq!(progress.title, "进度");
    assert!(progress.payload["event_count"].as_u64().unwrap_or(0) > 0);
    assert!(progress.payload["messages_delivered"].as_u64().unwrap_or(0) > 0);

    let results = report.projection(ObserverRoute::Results).unwrap();
    assert_eq!(results.payload["self_checks_passed"], true);
    assert_eq!(results.payload["audit"]["clean"], true);
    assert_eq!(results.payload["observer_api"]["writable"], false);

    let yield_view = report.projection(ObserverRoute::Yield).unwrap();
    assert_eq!(
        yield_view.payload["agents"].as_array().map(|a| a.len()),
        Some(k.agent_count()),
        "收益面板逐 Agent 展示可用/锁定余额"
    );
    assert!(yield_view.payload["ledger"]["minted"].as_i64().unwrap_or(0) > 0);
    assert!(yield_view.payload["ledger"]["total"].as_i64().unwrap_or(0) > 0);
}

#[test]
fn the_kernel_self_check_includes_real_observer_checks() {
    let k = rich_kernel();
    let checks = k.self_check();
    assert!(
        all_passed(&checks),
        "{:?}",
        checks.iter().filter(|c| !c.passed).collect::<Vec<_>>()
    );
    let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
    for expected in [
        "observer.routes",
        "observer.read_only",
        "observer.no_side_effect",
        "observer.no_write_route",
        "ledger.conservation",
    ] {
        assert!(names.contains(&expected), "缺少自检项 {expected}");
    }
    for check in &checks {
        assert_eq!(check.track, TRACK);
        assert!(!check.detail.is_empty(), "自检项必须写清断言了什么");
    }
    assert_eq!(observer_self_checks(&k).len(), 4);
}

#[test]
fn the_report_contains_no_command_channel() {
    let k = rich_kernel();
    let report = Observer::report(&k).json();
    let text = serde_json::to_string(&report).unwrap_or_default();
    // 报告是给人看的投影：里面不能有「待批准」「操作指令」这类东西。
    for forbidden in [
        "pending_approval",
        "commands",
        "mutations",
        "actions_to_apply",
    ] {
        assert!(
            !text.contains(forbidden),
            "报告里出现了指令通道：{forbidden}"
        );
    }
    // 但报告必须真的包含三个面板。
    let projections = report["projections"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let names: Vec<String> = projections
        .iter()
        .filter_map(|p| p["route"].as_str().map(|s| s.to_string()))
        .collect();
    assert_eq!(names, vec!["progress", "results", "yield"]);
}
