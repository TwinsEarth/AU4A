//! 人类观察层：**只有 GET** 的只读 HTTP 面板。
//!
//! 设计要点不是「我们承诺不给人类写入口」，而是**结构上不存在写入口**：
//! 路由表是一个 `&'static [&'static str]` 常量，服务端只接受 `GET`，其余方法一律 405，
//! 未知路径 404。测试里会真的起一个服务、发一次 POST、再比对状态快照逐字节未变。
//!
//! 三个面板（进度 / 结果 / 收益）共用同一份投影：`au4a_node::observe_all(&Kernel)` 的规范 JSON。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{json, Value};

/// 全部路由。**没有一条是写路由**——这就是「人类只观察」的机器可验证形式。
pub const ROUTES: [&str; 6] = [
    "/",
    "/api/progress",
    "/api/results",
    "/api/revenue",
    "/api/selfcheck",
    "/api/tracks",
];

/// 唯一允许的方法。
pub const ALLOWED_METHOD: &str = "GET";

/// 面板共享的只读投影：运行中由 Agent 侧刷新，观察侧只能读取快照。
#[derive(Clone)]
pub struct SharedView {
    inner: Arc<Mutex<String>>,
}

impl SharedView {
    pub fn new(initial: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(initial.into())),
        }
    }

    /// Agent 侧更新投影（人类侧没有对应方法）。
    pub fn update(&self, view: impl Into<String>) {
        if let Ok(mut guard) = self.inner.lock() {
            *guard = view.into();
        }
    }

    pub fn snapshot(&self) -> String {
        self.inner
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|_| "{}".to_string())
    }
}

/// 运行中的观察面板。
pub struct Observer {
    pub addr: SocketAddr,
    running: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Observer {
    /// 启动面板。`addr` 支持 `127.0.0.1:0`（由系统分配端口，测试用）。
    pub fn start(addr: &str, view: SharedView) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        let local = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let running = Arc::new(AtomicBool::new(true));
        let flag = running.clone();
        let join = std::thread::spawn(move || {
            while flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let view = view.clone();
                        // 每个连接一个线程；本地只读面板，连接数很小。
                        std::thread::spawn(move || {
                            let _ = handle(stream, &view);
                        });
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            addr: local,
            running,
            join: Some(join),
        })
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn handle(mut stream: TcpStream, view: &SharedView) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    // 读掉请求头（不需要 body：只读面板不解析任何输入）
    let mut header = String::new();
    loop {
        header.clear();
        let n = reader.read_line(&mut header)?;
        if n == 0 || header == "\r\n" || header == "\n" {
            break;
        }
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    // 去掉 query，路由匹配只看路径
    let route = path.split('?').next().unwrap_or("/").to_string();

    if method != ALLOWED_METHOD {
        return respond(
            &mut stream,
            405,
            "application/json; charset=utf-8",
            &json!({
                "error": "method_not_allowed",
                "allowed": ALLOWED_METHOD,
                "detail": "观察层是只读的：人类只能看进度、结果与收益。",
                "routes": ROUTES,
            })
            .to_string(),
        );
    }

    let snapshot = view.snapshot();
    let value: Value = serde_json::from_str(&snapshot).unwrap_or(Value::Null);

    match route.as_str() {
        "/" => respond(
            &mut stream,
            200,
            "text/html; charset=utf-8",
            &render_html(&value),
        ),
        "/api/progress" => respond(&mut stream, 200, JSON_CT, &panel(&value, "progress")),
        "/api/results" => respond(&mut stream, 200, JSON_CT, &panel(&value, "results")),
        "/api/revenue" => respond(&mut stream, 200, JSON_CT, &panel(&value, "revenue")),
        "/api/selfcheck" => respond(&mut stream, 200, JSON_CT, &panel(&value, "selfcheck")),
        "/api/tracks" => respond(&mut stream, 200, JSON_CT, &panel(&value, "tracks")),
        _ => respond(
            &mut stream,
            404,
            JSON_CT,
            &json!({ "error": "not_found", "routes": ROUTES }).to_string(),
        ),
    }
}

const JSON_CT: &str = "application/json; charset=utf-8";

fn panel(value: &Value, key: &str) -> String {
    let payload = json!({
        "version": value.get("version").cloned().unwrap_or(Value::Null),
        "panel": key,
        "read_only": true,
        "allowed_method": ALLOWED_METHOD,
        "data": value.get(key).cloned().unwrap_or(Value::Null),
    });
    au4a_core::canonicalize(&payload).unwrap_or_else(|_| "{}".to_string())
}

fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nAllow: {ALLOWED_METHOD}\r\nConnection: close\r\n\r\n",
        body.as_bytes().len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    stream.flush()
}

/// 人类真正看到的界面：三块只读面板 + 自检结果。没有表单、没有按钮。
fn render_html(value: &Value) -> String {
    let version = value
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let progress = value.get("progress").cloned().unwrap_or(Value::Null);
    let results = value.get("results").cloned().unwrap_or(Value::Null);
    let revenue = value.get("revenue").cloned().unwrap_or(Value::Null);
    let selfcheck = value.get("selfcheck").cloned().unwrap_or(Value::Null);

    let agents = progress
        .get("agents")
        .and_then(|a| a.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let delivered = progress
        .get("messages_delivered")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let refusals = progress
        .get("refusal_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let now = progress.get("now").and_then(|v| v.as_u64()).unwrap_or(0);
    let total = revenue
        .get("total")
        .and_then(|v| v.get("0"))
        .and_then(|v| v.as_i64());
    let total_txt = total
        .map(|t| t.to_string())
        .or_else(|| revenue.get("total").map(|t| t.to_string()))
        .unwrap_or_else(|| "—".to_string());
    let minted = revenue
        .get("minted")
        .map(|v| v.to_string())
        .unwrap_or_else(|| "—".into());
    let slashed = revenue
        .get("slashed")
        .map(|v| v.to_string())
        .unwrap_or_else(|| "—".into());
    let checks_passed = selfcheck
        .get("passed")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let checks_total = selfcheck.get("total").and_then(|v| v.as_u64()).unwrap_or(0);

    let escalation = json!({
        "agent_count": agents,
        "messages_delivered": delivered,
        "refusals": refusals,
        "clock": now,
    });

    format!(
        r#"<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8">
<meta http-equiv="refresh" content="2">
<title>AU4A 观察面板 {version}</title>
<style>
body{{font-family:system-ui,Segoe UI,sans-serif;background:#0b0f14;color:#d7e0ea;margin:0;padding:24px}}
h1{{font-size:18px;margin:0 0 4px}} .sub{{color:#7d8b9a;font-size:13px;margin-bottom:20px}}
.grid{{display:grid;grid-template-columns:repeat(auto-fit,minmax(300px,1fr));gap:16px}}
.card{{background:#131a22;border:1px solid #1f2a36;border-radius:10px;padding:16px}}
.card h2{{font-size:14px;margin:0 0 10px;color:#8fd0ff;font-weight:600}}
.kv{{font-size:13px;line-height:1.9}} .kv b{{color:#eaf2fa}}
pre{{background:#0e141b;border:1px solid #1f2a36;border-radius:8px;padding:10px;overflow:auto;max-height:320px;font-size:12px;color:#9fb3c8}}
.note{{margin-top:20px;color:#7d8b9a;font-size:12px}}
</style></head><body>
<h1>Agent Universe For Agent — 人类观察面板</h1>
<div class="sub">版本 {version} ｜ 只读：本面板只有 GET 路由，非 GET 一律 405 ｜ 人类只做三件事：看进度、看结果、看收益</div>
<div class="grid">
  <div class="card"><h2>① 进度</h2><div class="kv">
    活跃 Agent：<b>{agents}</b><br>已投递消息：<b>{delivered}</b><br>拒绝记录：<b>{refusals}</b><br>网络节拍：<b>{now}</b><br>自检：<b>{checks_passed}/{checks_total}</b>
  </div><pre>{progress}</pre></div>
  <div class="card"><h2>② 结果</h2><pre>{results}</pre></div>
  <div class="card"><h2>③ 收益</h2><div class="kv">
    账本总量：<b>{total_txt}</b><br>已发行：<b>{minted}</b><br>已罚没：<b>{slashed}</b>
  </div><pre>{revenue}</pre></div>
</div>
<div class="note">读路由：{routes} ｜ 写路由：无。<br>结构化数据：/api/progress · /api/results · /api/revenue · /api/selfcheck · /api/tracks<br>本页每 2 秒自动重新 GET 一次（刷新也是一种读操作）。</div>
</body></html>"#,
        version = version,
        agents = agents,
        delivered = delivered,
        refusals = refusals,
        now = now,
        checks_passed = checks_passed,
        checks_total = checks_total,
        total_txt = total_txt,
        minted = minted,
        slashed = slashed,
        progress = escape(&pretty(&progress)),
        results = escape(&pretty(&results)),
        revenue = escape(&pretty(&revenue)),
        routes = ROUTES.join(" · "),
    ) + &format!("\n<!-- escalation {escalation} -->\n")
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| "—".into())
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// 用一次真实 HTTP 请求读一个端点（测试与部署验证共用）。
///
/// 注意：URL 里的**路径必须真的发出去**——面板的路由匹配看的是请求行里的路径，
/// 而不是连接字符串。这里把 `http://host:port/path` 拆成 `(host:port, /path)` 两段。
pub fn fetch(url: &str, method: &str) -> std::io::Result<(u16, String)> {
    let rest = url.trim_start_matches("http://");
    let (addr, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    let mut stream = TcpStream::connect(addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let req = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes())?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw)?;
    let status = raw
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);
    let body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    Ok((status, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> SharedView {
        SharedView::new(
            json!({
                "version": "v1.9.9",
                "progress": { "agents": [], "messages_delivered": 3, "refusal_count": 0, "now": 7 },
                "results": { "tracks": {} },
                "revenue": { "total": 0, "minted": 0, "slashed": 0, "accounts": {} },
                "selfcheck": { "passed": 2, "total": 2 },
                "tracks": ["1.0 au4a-kernel"],
            })
            .to_string(),
        )
    }

    #[test]
    fn every_route_is_a_get_route() {
        assert_eq!(ALLOWED_METHOD, "GET");
        for r in ROUTES {
            assert!(r.starts_with('/'), "{r}");
        }
        // 路由表里不存在任何以写方法命名的入口
        let joined = ROUTES.join(" ");
        for forbidden in [
            "POST", "PUT", "PATCH", "DELETE", "set", "update", "write", "admin",
        ] {
            assert!(!joined.contains(forbidden), "路由表包含写语义：{forbidden}");
        }
    }

    #[test]
    fn observer_serves_reads_and_refuses_writes() {
        let v = view();
        let before = v.snapshot();
        let mut obs = Observer::start("127.0.0.1:0", v.clone()).unwrap();
        let base = obs.url();

        // 每一条声明过的路由都必须真的在它自己的路径上应答 200（路径必须被真的发出去）。
        for route in ROUTES {
            let (status, body) = fetch(&format!("{base}{route}"), "GET").unwrap();
            assert_eq!(status, 200, "GET {route} 应当 200，实际 {status}");
            if route == "/" {
                assert!(body.contains("观察面板"), "首页应当渲染三面板");
            } else {
                let key = route.trim_start_matches("/api/");
                assert!(
                    body.contains(&format!("\"panel\":\"{key}\"")),
                    "GET {route} 返回了错误的面板：{body}"
                );
                assert!(body.contains("\"read_only\":true"), "面板必须声明只读");
                assert!(body.contains("\"allowed_method\":\"GET\""));
            }
        }

        // 任何写方法、对任何路由，都必须 405；写路由不是「被禁用」，而是不存在。
        for route in ROUTES {
            for method in ["POST", "PUT", "PATCH", "DELETE"] {
                let (status, body) = fetch(&format!("{base}{route}"), method).unwrap();
                assert_eq!(status, 405, "{method} {route} 必须被拒");
                assert!(body.contains("method_not_allowed"));
            }
        }

        // 未知路径 404；连方法名都不认识的请求同样 405（而不是 200 或 panic）。
        let (status, _) = fetch(&format!("{base}/api/does-not-exist"), "GET").unwrap();
        assert_eq!(status, 404);
        let (status, _) = fetch(&format!("{base}/api/progress"), "BREW").unwrap();
        assert_eq!(status, 405);

        // 只读性证明：一轮读写混合之后，投影逐字节未变
        assert_eq!(v.snapshot(), before);
        obs.stop();
    }

    #[test]
    fn every_declared_route_is_enumerable_and_get_only() {
        // 结构性证据：路由表本身就是全部入口；没有任何一条带写语义的名字。
        assert_eq!(ROUTES.len(), 6);
        let joined = ROUTES.join(" ");
        for forbidden in [
            "write", "update", "set", "delete", "create", "admin", "approve",
        ] {
            assert!(!joined.to_lowercase().contains(forbidden), "{forbidden}");
        }
        assert_eq!(ALLOWED_METHOD, "GET");
    }

    #[test]
    fn malformed_requests_do_not_panic() {
        let mut obs = Observer::start("127.0.0.1:0", view()).unwrap();
        let addr = obs.addr;
        // 空请求
        {
            let mut s = TcpStream::connect(addr).unwrap();
            let _ = s.write_all(b"\r\n\r\n");
        }
        // 只有方法没有版本
        {
            let mut s = TcpStream::connect(addr).unwrap();
            let _ = s.write_all(b"GET\r\n\r\n");
        }
        // 服务仍然活着
        let (status, _) = fetch(&obs.url(), "GET").unwrap();
        assert_eq!(status, 200);
        obs.stop();
    }
}
