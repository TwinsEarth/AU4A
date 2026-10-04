# au4a-economy — Economic Autonomy 经济自主（轨道 1.4）

AU4A 的经济自主层：**Agent 自己管钱、自己定价、自己质押、自己仲裁、自己决定兑换路由**；
收益归属资源提供者，人类只观察（看进度 / 看结果 / **看收益**），不能发起、批准、定价、裁决。

* 版本区间：`v1.4.1 → v1.4.10`（10 个小版本，严格串行）
* 依赖：`au4a-core`（冻结基元）、`au4a-kernel`（宿主内核）、`serde`、`serde_json`
* 轨道文档：[`docs/tracks/1.4.md`](../../docs/tracks/1.4.md) ·
  机器可读元数据：[`docs/tracks/1.4.json`](../../docs/tracks/1.4.json)

## 三条不可协商的规则

1. **一切账务走 `au4a_core::Ledger`**，每个写路径末尾断言
   `Σ可用 + Σ锁定 + 罚没 == 已发行`（`check_conservation()`）。
   质押簿另有不变式：**它声称的锁定量永远不超过账本实际锁定量**（可以少记，绝不多记）。
2. **整数经济**：金额只用 `Credits`（微积分），比率只用基点（1bp = 0.01%）。
   库代码不含浮点，规范 JSON 直接拒绝浮点，因此同输入必然同输出、可重放。
3. **证据闸门**：`EvidenceGrade::Unverified` 永远不可结算；`CpuProto` 只能结算不超过
   `cpu_proto_settle_cap` 的小额。真实链上执行属于 **v1.8**，本 crate 只做**决策与账务**，
   并且**在类型层面**就无法声称链上成功：`ChainExecution` 没有 `Executed` 变体，
   `ChainExecution::executed()` 恒为 `false`，`IntentStatus` 只有 `AwaitingChainExecution`。

## 模块地图（一个小版本一个模块）

| 版本 | 模块 | 职责 | 关键 public 项 |
|---|---|---|---|
| v1.4.1 | `balance` | 余额管理：运营底线 / 目标质押比例 / 单笔支出上限 | `BalancePolicy`、`SpendVerdict`、`check_spend`、`spend`、`autostake`、`BalanceManager` |
| v1.4.2 | `pricing` | 自主定价：信誉 / 稀缺度 / 负载的整数基点函数 | `PriceInputs`、`PriceKnobs`、`PriceQuote`、`quote`、`unit_price` |
| v1.4.3 | `fx` | 兑换路由决策：金额 / 时效 / 费用阈值决策表 + 在途预留 | `RouteTable`、`route`、`escrow_for_route`、`ChainExecution`、`ExchangeBook` |
| v1.4.4 | `stake` | 质押 / 解质押 / 冷静期 / 罚没同步 | `StakeTerms`、`StakeBook`、`release_matured`、`absorb_slash` |
| v1.4.5 | `arbitration` | 自主立案 / 投票 / 裁决 / 申诉（罚没上限 = 锁定余额） | `Court`、`ArbitrationTerms`、`Ruling`、`PenaltyCap` |
| v1.4.6 | `settlement` | 结算路由 + 最大余数法分成 + 收益归属 | `route`、`split_weights`、`pay_split`、`RevenueBook`、`Beneficiary` |
| v1.4.7 | `tests/` | 跨模块性质测试与端到端测试 | 87 个断言（lib 71 + e2e 6 + invariants 10） |
| v1.4.8 | `README.md` + rustdoc | 文档与证据汇总 | 版本清单 / 错误码映射 / 证据分级 |
| v1.4.9 | `examples/economy_tour.rs` | 可运行示例：九步走完经济自主 | `cargo run -p au4a-economy --example economy_tour` |
| v1.4.10 | `monitor` | 只读收益面板数据源（节点 `/api/revenue` 的超集） | `revenue_panel`、`monitor_json`、`monitor_checks`、`LedgerTotals`、`RevenuePanel` |

## 决策表速查

**余额支出**（`balance::check_spend`）：零额 → 拒绝；会穿过底线 → 拒绝；超过单笔上限 → 拒绝；其余放行。
拒绝时账本**一个字节都不变**（判定在写路径之前完成）。

**定价**：

```text
信誉折扣 = reputation_bp × reputation_discount_max_bp / 10000
稀缺溢价 = scarcity_bp   × scarcity_premium_max_bp   / 10000
负载溢价 = load_bp       × load_premium_max_bp       / 10000
multiplier_bp = clamp(10000 + 负载溢价 + 稀缺溢价 − 信誉折扣, min_bp, max_bp)
unit_price    = max(base_price × multiplier_bp / 10000, 1)
```

**兑换路由**（`fx::route`）：`amount < onchain_min_amount` → 内部；时效不足 → 缓办；
费率 > `max_fee_bp` → 缓办；其余 → 上链路由（v1.8 执行，本 crate 只在途预留）。

**结算路由**（`settlement::route`）：`Unverified` → 拒付；`CpuProto` 超上限 → 拒付；
收款方有未结争议 → 托管；其余 → 直接结算（走 `Kernel::settle`）。

**仲裁罚没**：`min(索赔额, cpu-proto 上限, 锁定余额)`；`PenaltyCap` 记录被谁截断；
申诉重开投票，但**已销毁的罚没不可退回**。

## 错误码映射（`CoreError` 是冻结枚举，不新增变体）

| 语义 | 用的变体 | 出现在 |
|---|---|---|
| 金额为 0 / 参数为负 | `ZeroAmount` / `NegativeAmount` | 全部模块 |
| 比例基点越界（>10000） | `InvalidKind` | `balance`、`pricing`、`fx`、`stake`、`arbitration`、`settlement` |
| 余额 / 额度不足、证据闸门拒付 | `InsufficientFunds` | `balance`、`settlement`、`Kernel::settle` |
| 低于准入线 / 超过份额上限 / 质押簿多记 | `InsufficientStake` | `stake` |
| 未知账户 / 未注册头寸 / 无资格参与本案 | `UnknownAgent` | `balance`、`stake`、`arbitration` |
| 状态机不接受该动作（重复投票、票数不足、窗口过期、重复裁决） | `InvalidKind` | `arbitration` |
| 证据不可作为罚没依据 / 状态与动作不匹配 | `InvalidKind` | `arbitration`、`fx`、`settlement` |
| 整数溢出 | `Overflow` | 全部模块 |

## 证据分级（本轨道怎么自评）

| 能力 | 等级 | 说明 |
|---|---|---|
| 余额 / 定价 / 质押 / 仲裁 / 分成 / 记账 | `verified` | 本机真实运行，87 个断言，走真实 `Ledger` 写路径 |
| 兑换与结算**路由决策** | `verified` | 决策表 + 在途预留 + 拒付，全部有测试 |
| 只读**收益面板**投影 | `verified` | `monitor` 两次投影逐字段相同、读取前后账本不变 |
| **真实链上执行** | 未做（属 v1.8） | 本 crate 只登记 `AwaitingChainExecution` 意图，从不报告成功 |
| 与外部交易所 / 预言机的价格 | 未做 | 报价完全由 Agent 自己的状态决定（信誉 / 稀缺 / 负载） |

## 只读收益面板（v1.4.10）

`monitor::revenue_panel(&kernel, &revenue_book, &context)` 返回一个 JSON 值，字段是节点
`/api/revenue` 的**超集**（`minted` / `slashed` / `total` / `accounts.<did>.{available,locked}` 保持不变）：

```text
{
  "read_only": true,
  "agents": 4,
  "ledger": { "minted", "slashed", "total", "available", "locked",
              "accounts": { "<did>": { "available", "locked", "earned", "kind", "display" } },
              "account_count", "conservation_ok" },
  "total_earned": 200,
  "rows": [ { "did", "display", "kind", "available", "locked", "earned" } ]
}
```

收益只统计**真实到账凭证**：拒付（`withheld`）与托管（`escrowed`）都不算收入；罚没只体现在
`slashed` 里，不流向任何人。所有入口都接 `&Kernel`/`&RevenueBook`，没有任何写路径——
`monitor_checks()` 会真跑两次投影并断言逐字段相同、且账本在读取前后不变。

## 本地验证

```powershell
$env:CARGO_TARGET_DIR = "E:\DS\_forangent\target\au4a-economy"
cd E:\DS\AU4A
cargo test -p au4a-economy          # 87 个断言，0 failed、0 warning
cargo run  -p au4a-economy --example economy_tour
```
