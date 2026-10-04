//! 人类观察层（v1.0.3）：**结构性只读**。
//!
//! AU4A 里人类只有三个只读投影：**进度 / 结果 / 收益**。人类不能发起、不能批准、不能调度、
//! 不能定价、不能裁决。这不是「权限配置上不给」，而是**类型上不存在写路径**：
//!
//! 1. 本模块里没有任何一处出现 `&mut Kernel`：所有渲染入口都取共享借用（`&Kernel`）；
//! 2. [`ObserverCapability`] **只有 `Read` 一个变体**——写能力连类型都没有；
//! 3. [`ObserverRoute::effects`] 对每条路由都返回空效果表（声明「这条路不产生副作用」）；
//! 4. [`observer_api`] 把入口清单导出成 `writable: false` 的 JSON，供人与测试枚举；
//! 5. [`observer_self_checks`] 渲染前后比对注册表指纹与观察投影：**观察不会改变内核状态**。
//!
//! 下面两条 `const _` 是编译期证据：入口的函数签名就是只读的，改错方向编译不过。

use au4a_core::{canonical_hash, CoreError, CoreResult, SelfCheck};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::Kernel;

/// 人类的三个只读投影路由。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObserverRoute {
    /// 进度：发生了什么（进度事件流）。
    Progress,
    /// 结果：内核产物与自检/审计结论。
    Results,
    /// 收益：账本投影与每个 Agent 的可用/锁定余额。
    Yield,
}

impl ObserverRoute {
    pub const ALL: [ObserverRoute; 3] = [
        ObserverRoute::Progress,
        ObserverRoute::Results,
        ObserverRoute::Yield,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ObserverRoute::Progress => "progress",
            ObserverRoute::Results => "results",
            ObserverRoute::Yield => "yield",
        }
    }

    /// 人类可读的中文面板名。
    pub fn title(self) -> &'static str {
        match self {
            ObserverRoute::Progress => "进度",
            ObserverRoute::Results => "结果",
            ObserverRoute::Yield => "收益",
        }
    }

    /// 这条路由需要的能力。**永远只是 `Read`**。
    pub fn capability(self) -> ObserverCapability {
        ObserverCapability::Read
    }

    /// 这条路由的副作用表。**永远为空**。
    pub fn effects(self) -> &'static [&'static str] {
        &[]
    }
}

/// 观察层的能力。只有一个变体：观察层没有写能力，类型上就无法表达「写」。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObserverCapability {
    Read,
}

impl ObserverCapability {
    pub const ALL: [ObserverCapability; 1] = [ObserverCapability::Read];

    pub fn as_str(self) -> &'static str {
        match self {
            ObserverCapability::Read => "read",
        }
    }

    /// 是否可写。对观察层永远是 `false`。
    pub fn writable(self) -> bool {
        false
    }
}

/// 一个投影：路由 + 能力 + 副作用 + 载荷 + 内容指纹。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObserverProjection {
    pub route: String,
    pub title: String,
    pub capability: String,
    pub writable: bool,
    pub effects: Vec<String>,
    pub payload: Value,
    /// 载荷的内容寻址指纹（人类可以把两次观察的指纹贴在一起比对，无法伪造）。
    pub fingerprint: String,
}

/// 三个投影的打包（人类界面一次取全部）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObserverReport {
    pub network_id: String,
    pub now: u64,
    pub projections: Vec<ObserverProjection>,
}

impl ObserverReport {
    pub fn projection(&self, route: ObserverRoute) -> Option<&ObserverProjection> {
        self.projections.iter().find(|p| p.route == route.as_str())
    }

    /// 全部投影都是只读且无副作用。
    pub fn all_read_only(&self) -> bool {
        !self.projections.is_empty()
            && self
                .projections
                .iter()
                .all(|p| !p.writable && p.effects.is_empty() && p.capability == "read")
    }

    pub fn json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// 报告指纹：同一状态 → 同一指纹（观察层自身可被比对）。
    pub fn fingerprint(&self) -> CoreResult<String> {
        let value = serde_json::to_value(self).map_err(|_| CoreError::Encoding)?;
        canonical_hash(&value)
    }
}

/// 人类观察层。无状态：它只是「把内核的只读投影渲染成值」。
pub struct Observer;

impl Observer {
    /// 渲染一条路由。注意参数是 `&Kernel`——人类拿不到可变借用。
    pub fn render(kernel: &Kernel, route: ObserverRoute) -> ObserverProjection {
        let payload = match route {
            ObserverRoute::Progress => json!({
                "now": kernel.now(),
                "event_count": kernel.progress_events().len(),
                "events": kernel.progress_events(),
                "messages_delivered": kernel.results_json()["delivered"],
            }),
            ObserverRoute::Results => json!({
                "results": kernel.results_json(),
                // 注意：这里刻意不用 `Kernel::self_check()`——它会调用本模块的自检，
                // 而本模块的自检会再渲染本面板，形成递归。宿主的「结果」面板展示
                // 审计结论（同样的不变式，不含观察层自身），观察层自身的检查由
                // `observer_self_checks` 单独提供、由节点聚合。
                "self_checks": kernel.audit().to_self_checks(crate::TRACK),
                "self_checks_passed": kernel.audit().is_clean(),
                "audit": kernel.audit().to_json(),
                "lifecycle": kernel.lifecycles().to_json(),
                "observer_api": observer_api(),
            }),
            ObserverRoute::Yield => json!({
                "ledger": kernel.ledger().view(),
                "agents": kernel
                    .observe()
                    .agents
                    .iter()
                    .map(|row| json!({
                        "did": row.did,
                        "display": row.display,
                        "available": row.available,
                        "locked": row.locked,
                        "stake": row.stake,
                    }))
                    .collect::<Vec<Value>>(),
            }),
        };
        let fingerprint = canonical_hash(&json!({
            "route": route.as_str(),
            "capability": route.capability().as_str(),
            "payload": payload,
        }))
        .unwrap_or_default();
        ObserverProjection {
            route: route.as_str().to_string(),
            title: route.title().to_string(),
            capability: route.capability().as_str().to_string(),
            writable: route.capability().writable(),
            effects: route.effects().iter().map(|e| (*e).to_string()).collect(),
            payload,
            fingerprint,
        }
    }

    /// 渲染全部三条路由。
    pub fn render_all(kernel: &Kernel) -> Vec<ObserverProjection> {
        ObserverRoute::ALL
            .iter()
            .map(|route| Observer::render(kernel, *route))
            .collect()
    }

    /// 人类界面拿到的那一个值。
    pub fn report(kernel: &Kernel) -> ObserverReport {
        ObserverReport {
            network_id: kernel.config().network_id.clone(),
            now: kernel.now(),
            projections: Observer::render_all(kernel),
        }
    }
}

/// 入口清单：把「人类能做什么」枚举成人可读、机器可校验的 JSON。
///
/// 顶层 `writable: false`、每条路由 `effects: []`——人类能做的只有「看」。
pub fn observer_api() -> Value {
    json!({
        "writable": false,
        "capabilities": ObserverCapability::ALL
            .iter()
            .map(|c| c.as_str())
            .collect::<Vec<&str>>(),
        "routes": ObserverRoute::ALL
            .iter()
            .map(|route| json!({
                "route": route.as_str(),
                "title": route.title(),
                "capability": route.capability().as_str(),
                "writable": route.capability().writable(),
                "effects": route.effects(),
            }))
            .collect::<Vec<Value>>(),
    })
}

/// 观察层的自检：真的渲染一遍，并断言渲染前后内核状态**没有变化**。
///
/// 每个检查都建立在真实运行上，不是「应该没问题」。
pub fn observer_self_checks(kernel: &Kernel) -> Vec<SelfCheck> {
    let before_registry = kernel.registry_fingerprint();
    let before_view = kernel.observe_json();
    let before_now = kernel.now();
    let report = Observer::report(kernel);
    let after_registry = kernel.registry_fingerprint();
    let after_view = kernel.observe_json();

    let routes_ok = report.projections.len() == ObserverRoute::ALL.len();
    let read_only = report.all_read_only();
    let unchanged = before_registry == after_registry
        && before_view == after_view
        && before_now == kernel.now();
    let no_write_effects = ObserverRoute::ALL
        .iter()
        .all(|r| r.effects().is_empty() && !r.capability().writable());

    let mut checks = Vec::new();
    checks.push(if routes_ok {
        SelfCheck::pass(
            crate::TRACK,
            "observer.routes",
            format!("{} 条只读路由（进度/结果/收益）全部渲染成功", report.projections.len()),
        )
    } else {
        SelfCheck::fail(
            crate::TRACK,
            "observer.routes",
            format!("路由数 {} != {}", report.projections.len(), ObserverRoute::ALL.len()),
        )
    });
    checks.push(if read_only {
        SelfCheck::pass(
            crate::TRACK,
            "observer.read_only",
            "全部投影 capability=read、writable=false、effects=[]",
        )
    } else {
        SelfCheck::fail(crate::TRACK, "observer.read_only", "存在非只读投影")
    });
    checks.push(if unchanged {
        SelfCheck::pass(
            crate::TRACK,
            "observer.no_side_effect",
            "渲染前后注册表指纹 / 观察投影 / 逻辑时钟读数均未变化",
        )
    } else {
        SelfCheck::fail(crate::TRACK, "observer.no_side_effect", "观察改变了内核状态")
    });
    checks.push(if no_write_effects {
        SelfCheck::pass(
            crate::TRACK,
            "observer.no_write_route",
            "结构枚举：ObserverCapability 只有 Read 变体，没有任何路由声明副作用",
        )
    } else {
        SelfCheck::fail(crate::TRACK, "observer.no_write_route", "存在声明了副作用的路由")
    });
    checks
}

/// 编译期证据 1：内核观察入口取共享借用（不是 `&mut self`）。
const _: fn(&Kernel) -> crate::ObserverView = Kernel::observe;
/// 编译期证据 2：观察层渲染入口只接受 `&Kernel`——人类拿不到可变借用。
const _: fn(&Kernel, ObserverRoute) -> ObserverProjection = Observer::render;
/// 编译期证据 3：报告入口同样只接受 `&Kernel`。
const _: fn(&Kernel) -> ObserverReport = Observer::report;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bootstrap, KernelConfig};
    use au4a_core::all_passed;

    fn kernel() -> Kernel {
        bootstrap(KernelConfig::default()).unwrap()
    }

    #[test]
    fn every_route_is_read_only_and_has_no_effects() {
        for route in ObserverRoute::ALL {
            assert_eq!(route.capability(), ObserverCapability::Read);
            assert!(!route.capability().writable());
            assert!(route.effects().is_empty(), "{} 声明了副作用", route.as_str());
        }
        assert_eq!(ObserverCapability::ALL.len(), 1, "写能力连类型都没有");
    }

    #[test]
    fn api_surface_declares_itself_non_writable() {
        let api = observer_api();
        assert_eq!(api["writable"], false);
        let routes = api["routes"].as_array().cloned().unwrap_or_default();
        assert_eq!(routes.len(), 3);
        for route in routes {
            assert_eq!(route["writable"], false);
            assert_eq!(route["capability"], "read");
            assert_eq!(route["effects"].as_array().map(|a| a.len()), Some(0));
        }
        assert_eq!(api["capabilities"].as_array().map(|a| a.len()), Some(1));
        // 人类不能发起/批准/调度/定价/裁决：路由名里没有这类动词。
        for route in ObserverRoute::ALL {
            let name = route.as_str();
            for verb in ["approve", "grant", "schedule", "price", "adjudicate", "mint", "propose"] {
                assert!(!name.contains(verb), "路由 {name} 像是一个写入口");
            }
        }
    }

    #[test]
    fn rendering_does_not_change_the_kernel() {
        let k = kernel();
        let before_registry = k.registry_fingerprint().unwrap();
        let before_view = k.observe_json();
        let first = Observer::report(&k);
        let second = Observer::report(&k);
        assert_eq!(first, second);
        assert_eq!(first.fingerprint().unwrap(), second.fingerprint().unwrap());
        assert_eq!(before_registry, k.registry_fingerprint().unwrap());
        assert_eq!(before_view, k.observe_json());
    }

    #[test]
    fn projections_cover_progress_results_and_yield() {
        let k = kernel();
        let report = Observer::report(&k);
        assert!(report.all_read_only());
        assert_eq!(report.projections.len(), 3);
        let progress = report.projection(ObserverRoute::Progress).unwrap();
        assert!(progress.payload["event_count"].as_u64().unwrap_or(0) > 0);
        let results = report.projection(ObserverRoute::Results).unwrap();
        assert_eq!(results.payload["self_checks_passed"], true);
        assert_eq!(results.payload["audit"]["clean"], true);
        let yield_view = report.projection(ObserverRoute::Yield).unwrap();
        assert_eq!(
            yield_view.payload["ledger"]["total"],
            serde_json::to_value(k.ledger().view().total).unwrap()
        );
        assert_eq!(
            yield_view.payload["agents"].as_array().map(|a| a.len()),
            Some(3)
        );
    }

    #[test]
    fn observer_self_checks_pass_on_a_live_kernel() {
        let k = kernel();
        let checks = observer_self_checks(&k);
        assert!(all_passed(&checks), "{checks:?}");
        assert_eq!(checks.len(), 4);
        assert!(checks.iter().any(|c| c.name == "observer.no_write_route"));
        assert!(checks.iter().any(|c| c.name == "observer.no_side_effect"));
    }

    #[test]
    fn report_json_roundtrips() {
        let k = kernel();
        let report = Observer::report(&k);
        let json = report.json();
        let back: ObserverReport = serde_json::from_value(json).unwrap();
        assert_eq!(back, report);
    }
}
