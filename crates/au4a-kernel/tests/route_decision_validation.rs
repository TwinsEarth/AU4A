//! v2.4.0 回归测试：路由判定记录的**反序列化校验**。
//!
//! 修复前 `RouteDecision` 是纯 `derive(Deserialize)`：外部 JSON 可以造出
//! `{"accepted": true, "code": "unauthorized"}`（既声称通过、又附恶意拒绝码），
//! 或 `accepted: true` 但 `recipients: []`（声称投递成功却没有收件人）。
//! 判定记录是投递侧的"账"，被伪造的"已投递"会污染审计与观察面板。

use au4a_kernel::RouteDecision;

fn decision(accepted: bool, recipients: &str, code: &str) -> String {
    format!(
        r#"{{"id":"abc","kind":"settle.request","class":"settlement","accepted":{accepted},"recipients":[{recipients}],"code":{code},"reason":"r"}}"#
    )
}

#[test]
fn a_forged_route_decision_is_refused_by_serde() {
    // ① 通过 + 同时带拒绝码 → 自相矛盾
    assert!(serde_json::from_str::<RouteDecision>(&decision(
        true,
        "\"did:au4a:x\"",
        "\"unauthorized\""
    ))
    .is_err());
    // ② 通过但没有收件人
    assert!(serde_json::from_str::<RouteDecision>(&decision(true, "", "null")).is_err());
    // ③ 被拒却不给类型化理由码
    assert!(serde_json::from_str::<RouteDecision>(&decision(false, "", "null")).is_err());
    // ④ `class` 与 `kind` 的归类不一致（自己声明更宽的类别）
    let wrong_class = r#"{"id":"abc","kind":"council.motion","class":"announce","accepted":true,"recipients":["did:au4a:x"],"code":null,"reason":"r"}"#;
    assert!(serde_json::from_str::<RouteDecision>(wrong_class).is_err());
    // ⑤ 空 id / 空 reason
    let empty_id = r#"{"id":"  ","kind":"settle.request","class":"settlement","accepted":true,"recipients":["did:au4a:x"],"code":null,"reason":"r"}"#;
    assert!(serde_json::from_str::<RouteDecision>(empty_id).is_err());
}

#[test]
fn a_legitimate_route_decision_roundtrips() {
    let ok = decision(true, "\"did:au4a:x\"", "null");
    let parsed: RouteDecision = serde_json::from_str(&ok).unwrap();
    assert!(parsed.accepted);
    let text = serde_json::to_string(&parsed).unwrap();
    assert_eq!(
        serde_json::from_str::<RouteDecision>(&text).unwrap(),
        parsed
    );

    // 被拒但带码的判定同样合法
    let refused = decision(false, "", "\"conflict\"");
    let parsed_refused: RouteDecision = serde_json::from_str(&refused).unwrap();
    assert!(!parsed_refused.accepted);
    assert_eq!(parsed_refused.code, Some(au4a_core::RefusalCode::Conflict));
}
