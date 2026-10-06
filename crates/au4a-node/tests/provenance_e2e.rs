//! v2.6.2 端到端测试：**起一个带身份的节点 → GET /api/results → 取 provenance.report → 验签必须通过**。
//!
//! 这是"面板可验证来源"的闭环证明：不是断言"字段存在"，而是把响应里的报告**真的拿去验签**。
//! 同时覆盖：匿名模式（无密钥）不带来源、私钥不出现在任何响应体里。

use std::io::{Read, Write};
use std::net::TcpStream;

use au4a_node::identity::NodeIdentity;
use au4a_node::observer::{Observer, SharedView};

const SEED: &str = "0a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20212223242526272829";

/// 极简 HTTP 客户端（只支持 GET；测试里不需要更多）。
fn http_get(addr: std::net::SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect");
    let req = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).expect("write");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).expect("read");
    let text = String::from_utf8_lossy(&raw).to_string();
    // 去掉响应头
    match text.find("\r\n\r\n") {
        Some(i) => text[i + 4..].to_string(),
        None => text,
    }
}

/// v2.6.4：**直接调用库函数** `publish_snapshot`，不再复刻 `main.rs` 的闭包——
/// 测试与真实节点从此用同一份实现，漂移不可能发生（这是本版存在的唯一理由）。
fn snapshot_with_provenance(identity: &NodeIdentity) -> (String, String) {
    let (kernel, _outcomes, _dids) = au4a_node::scenario::run_full(2, |_| {}).expect("scenario");
    let snapshot = au4a_node::publish_snapshot(&kernel, Some(identity));
    (snapshot, identity.did().as_str().to_string())
}

#[test]
fn a_panel_delivered_over_http_carries_a_verifiable_signature() {
    let identity = NodeIdentity::from_seed_hex(SEED).expect("identity");
    let (snapshot, node_did) = snapshot_with_provenance(&identity);

    let mut observer = Observer::start("127.0.0.1:0", SharedView::new(snapshot)).expect("start");
    let addr = observer.addr;

    // ① 默认嵌入：面板响应里带 provenance
    let body = http_get(addr, "/api/results");
    let panel: serde_json::Value = serde_json::from_str(&body).expect("panel json");
    assert_eq!(panel["read_only"], true);
    let provenance = &panel["provenance"];
    assert_eq!(
        provenance["node_did"],
        serde_json::Value::String(node_did.clone())
    );
    assert_eq!(provenance["scheme"], "ed25519");
    assert!(
        !provenance["sig"].as_str().unwrap_or("").is_empty(),
        "面板必须带上签名"
    );

    // ② 闭环：把响应里的报告**真的拿去验签**
    let report: au4a_kernel::ObserverReport =
        serde_json::from_value(provenance["report"].clone()).expect("report 必须是可反序列化的");
    report
        .verify_provenance()
        .expect("HTTP 送来的报告必须验签通过");
    assert_eq!(report.node_did.as_deref(), Some(node_did.as_str()));

    // ③ 独立路由返回同一份来源信息
    let prov_body = http_get(addr, "/api/provenance");
    let prov: serde_json::Value = serde_json::from_str(&prov_body).expect("provenance json");
    assert_eq!(prov["node"]["sig"], provenance["sig"]);
    assert_eq!(prov["node"]["node_did"], provenance["node_did"]);
    let report2: au4a_kernel::ObserverReport =
        serde_json::from_value(prov["node"]["report"].clone()).expect("report");
    report2
        .verify_provenance()
        .expect("独立路由的报告同样要验得过");

    // ④ 私钥永不出现在任何响应体里
    for route in [
        "/",
        "/api/progress",
        "/api/results",
        "/api/revenue",
        "/api/provenance",
    ] {
        let text = http_get(addr, route);
        assert!(
            !text.contains(SEED),
            "{route} 的响应体里出现了私钥种子（绝不允许）"
        );
    }

    // ⑤ 篡改面板里的报告 → 验签必须失败（证明上面的"通过"不是因为验签形同虚设）
    let mut tampered: au4a_kernel::ObserverReport = report.clone();
    tampered.now += 1;
    assert!(
        tampered.verify_provenance().is_err(),
        "被篡改的报告不得通过验签"
    );

    observer.stop();
}

#[test]
fn anonymous_node_serves_panels_without_provenance() {
    // 无密钥：快照里没有 provenance → 面板/provenance 路由都返回 null，其余行为不变（只读、200）。
    let (kernel, _o, _d) = au4a_node::scenario::run_full(1, |_| {}).expect("scenario");
    let snapshot = au4a_node::scenario::view_json(&kernel);
    let mut observer = Observer::start("127.0.0.1:0", SharedView::new(snapshot)).expect("start");
    let addr = observer.addr;

    let panel: serde_json::Value =
        serde_json::from_str(&http_get(addr, "/api/results")).expect("panel json");
    assert_eq!(panel["read_only"], true);
    assert_eq!(
        panel["provenance"],
        serde_json::Value::Null,
        "匿名模式的 provenance 必须是 null"
    );

    let prov: serde_json::Value =
        serde_json::from_str(&http_get(addr, "/api/provenance")).expect("provenance json");
    assert_eq!(prov["node"], serde_json::Value::Null);
    assert_eq!(prov["read_only"], true);

    observer.stop();
}
