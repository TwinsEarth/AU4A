# 人类观察层（OBSERVER）

> 人类只做三件事：**看进度、看结果、看收益**。这一页说明它是怎么被**代码结构**强制的，
> 而不是靠文档承诺——参考项目的教训正是「文档说只读，代码里到处是写入口」。

## 1. 三个面板

| 面板 | 人类看到什么 | 数据来源 | 端点 |
|---|---|---|---|
| 进度 | 活跃 Agent 数、已投递消息、拒绝记录、网络节拍、每条轨道的运行事件 | `Kernel::observe()` → `ObserverView.progress` | `GET /api/progress` |
| 结果 | 各轨道的 `results_json()` 产物摘要 + 10 条轨道自检（通过/未通过 + 证据说明） | `au4a_node::track_results()` + `self_checks_json()` | `GET /api/results`、`GET /api/selfcheck` |
| 收益 | 账本总量、已发行、已罚没、每个账户的可用与锁定余额 | `Kernel::observe().ledger`（`LedgerView`） | `GET /api/revenue` |

HTML 首页 `GET /` 把三块面板放在一屏里，每 2 秒自动重新 GET 一次（刷新本身也是读操作）。

## 2. 只读性如何被结构强制

1. **路由表是常量**：`au4a_node::observer::ROUTES` 是一个 `&'static [&'static str]`，全部以 `/` 开头，
   里面不存在任何写语义的名字。
2. **只接受 GET**：`ALLOWED_METHOD == "GET"`，服务端对任何非 GET 请求返回 **405 Method Not Allowed**，
   并在响应体里回 `{"error":"method_not_allowed","allowed":"GET"}`；未知路径返回 404。
3. **投影是值而不是句柄**：`Kernel::observe()` 返回 `ObserverView`（一个可序列化的值）。
   观察面拿到的是**快照字符串**（`SharedView`），Agent 侧 `update()` 是唯一写入方，人类侧只有 `snapshot()`。
4. **结构性测试**（`crates/au4a-node/src/observer.rs` 的 `#[cfg(test)]`）：
   * `every_route_is_a_get_route` / `every_declared_route_is_enumerable_and_get_only`：断言方法常量为 GET、
     路由表恰好 6 条、且不含任何写语义词；
   * `observer_serves_reads_and_refuses_writes`：真起服务 → **枚举全部 6 条路由**逐个 GET（API 面板还要校验
     返回的 `panel` 键与自己的路径一致）→ 对 6 条路由 × 4 种写方法（POST/PUT/PATCH/DELETE）断言 **405** →
     未知路径 404、未知方法 405 → **最后比对投影逐字节未变**（这才叫只读证明，不是「没写代码」）；
   * `malformed_requests_do_not_panic`：畸形请求不能把服务打挂。

   > 回归教训（v1.0.10 平台修复）：测试里的 HTTP 客户端 `fetch()` 曾把 `http://host:port/path` 整串当连接地址、
   > 且请求行固定写 `/`，于是「路由测试」实际从未命中过被声明的路径。现在 `fetch()` 明确拆成
   > `(host:port, /path)` 两段并把路径写进请求行——**路径必须真的发出去**，否则只读面板的枚举测试等于空转。

5. **部署实测**：`tools/verify-mediums.mjs` 会在 10 个中版本收尾点上真起面板、真发 GET/POST，
   把状态码写进 `docs/verify/medium-vX.Y.md`；`deploy/verify.ps1` 提供单机一键复现。

## 3. 人类还剩什么权力

| 权力 | 在哪 | 边界 |
|---|---|---|
| 看 | 本观察层（HTTP GET） | 无限制 |
| 收益 | `LedgerView` 只读投影；兑换结算由 Agent 自主（v1.4 / v1.8） | 人类不参与定价与分配 |
| 否决 | `au4a-council`（v1.7）的否决权 | **只能阻断**，必须公开理由，类型层面没有提案/修改能力 |
| 资源 | 提供算力/存储/网络的初始额度（`KernelConfig`） | 只是初始条件，不是运行期决策 |

## 4. 启动方式

```bash
# 让 8 个 Agent 自主跑，并打开只读面板
cargo run -p au4a-node -- run --agents 8 --observe 127.0.0.1:8787

# 只看一次投影（CI / 脚本用）
cargo run -p au4a-node -- observe --once --json

# 聚合 10 条轨道的自检（部署验证的退出码来源）
cargo run -p au4a-node -- verify --json
```
