# au4a-chain — Cross-Chain Settlement 跨链结算（轨道 1.8）

AU4A 的跨链结算层：BTC 侧（RGB、Taproot Assets）与 ETH 侧（ERC-8004、x402）的**语义适配器**、
结算路由、跨链信誉桥接，以及一份说真话的安全审计。

* 版本区间：`v1.8.1 → v1.8.10`（10 个小版本，严格串行）
* 依赖：`au4a-core`（冻结基元）、`au4a-kernel`（宿主内核）、`serde`、`serde_json`
* 轨道文档：[`docs/tracks/1.8.md`](../../docs/tracks/1.8.md) ·
  机器可读元数据：[`docs/tracks/1.8.json`](../../docs/tracks/1.8.json)

## 最重要的一句话：这里没有真实链上执行

本 crate 的所有「链上」都是**确定性测试网**：纯 CPU 状态机 + 内容寻址哈希 + 逻辑高度，
不联网、不读墙钟、不碰文件。链上证据等级恒为 `cpu-proto`：

* `testnet::ONCHAIN_GRADE = EvidenceGrade::CpuProto`，收据的 `grade` 字段写死它；
* 每份投影都带 `real_network: false` 与 `grade: "cpu-proto"`；
* 集成测试**递归检查**场景 JSON 里的每个 `grade` 与 `real_network`——
  代码层面不可能把测试网结果冒充成真实链上成功。

参考项目里 ERC-8004 / x402 / RGB / Taproot / Lightning **全部 0 命中**，所以这里是最容易造假的地方。
本轨道的做法是把边界写进**类型**与**测试**，而不是写在形容词里。

## 模块地图

| 版本 | 模块 | 职责 | 关键 public 项 |
|---|---|---|---|
| v1.8.1 | `testnet` + `bridge` + `rgb` | 确定性测试网、双轨账本、RGB 密封转移 | `Testnet`、`ChainTx`、`Receipt`、`ChainRefusal`、`BridgeBook`、`RgbAdapter` |
| v1.8.2 | `taproot` | 资产锚定与 Merkle 证明 | `merkle_root`、`merkle_proof`、`verify_merkle`、`TaprootAnchor`、`TaprootAdapter` |
| v1.8.3 | `erc8004` | 身份 / 声誉 / 验证注册表语义 | `Erc8004Adapter`、`ReputationSummary`、`reputation_weight_bp` |
| v1.8.4 | `x402` | 发票 / 支付 / 领取 | `Invoice`、`Payment`、`X402Adapter` |
| v1.8.5 | `routing` | 金额/时效/费用阈值选轨 + fail-closed 闸门 | `RoutingTable`、`route`、`settle`、`Rail` |
| v1.8.6 | `reputation` | 链上事件 → 本地 4 维信誉 | `ReputationBridge`、`LocalReputation`、`ReputationEventKind` |
| v1.8.7 | `tests/` | 跨模块不变量与端到端（87 个断言） | `chain_invariants.rs`、`e2e_settlement.rs` |
| v1.8.8 | 本文件 | 文档与证据汇总 | 模块地图 / 拒绝清单索引 / 证据分级 |
| v1.8.9 | `examples/chain_tour.rs` | 可运行示例（九步） | `cargo run -p au4a-chain --example chain_tour` |
| v1.8.10 | `audit` | 安全审计：风险清单 + 攻击测试 | `RiskRegister`、`RiskStatus`、`attack_suite` |

## 确定性测试网（所有「链上」语义的唯一所在地）

| 概念 | 实现 | 说明 |
|---|---|---|
| 链 | `ChainId::{BtcRegtest, EthLocal}` | 两条独立本地实例，最终性深度 6 / 12 |
| 交易 | `ChainTx`（内容寻址 id） | 篡改任何字段 → id 不匹配 → `Malformed` |
| 防重放 | `applied` ∪ `pending` | 同一 id 二次提交 → `Conflict` |
| 防乱序 | 每提交者 nonce 单调 | 回退 nonce → `StaleEpoch`；重组后按「链上 ∪ 待打包」重建 |
| 出块 | `mine()` / `mine_to(h)` | 逻辑高度，确定性 |
| 最终性 | `finalized_height` **单调不减** | 一旦达成**不再撤销**（比撤销区块危险得多） |
| 重组 | `reorg(depth)` | 越过冻结最终化高度 → `Conflict`；被回滚交易重新可打包 |
| 收据 | `Receipt { grade: cpu-proto }` | 「成功」只可能来自本地状态机 |
| 拒绝 | `ChainRefusal { op, code, detail }` | **一定带操作名**，并记进 `refusals` |

## 双轨守恒与 fail-closed

```text
桥出 bridge_out(X)：本地 可用 −X、锁定 +X（托管）  ｜ 链上表示 +X
桥回 bridge_in(X) ：链上表示 −X                  ｜ 本地 锁定 −X → 可用 +X
```

* 不变式 A（账本）：`Σ可用 + Σ锁定 + 罚没 == 发行`，每次写路径后断言；
* 不变式 B（双轨）：`本地托管 == 链上表示总量`；
* **fail-closed**：`bridge::require_consistent()` 不一致即 `Err`；`routing::route` 把它当**第一优先级**
  ——不一致时一律 `defer/reconciliation_failed`，不猜、不硬走、不动账；**账本永远是真相**。

## 具名拒绝清单索引（不支持的操作按名字拒绝，绝不静默通过）

| 适配器 | 清单 | 例子 |
|---|---|---|
| RGB | `RGB_REFUSALS` | `rgb.atomic_swap`(unsupported)、`rgb.unanchored_issuance`(policy_denied)、`rgb.blind_unsealed`(malformed) |
| Taproot | `TAPROOT_REFUSALS` | `taproot.script_path_spend`(unsupported)、`taproot.asset_inflation`(policy_denied) |
| ERC-8004 | `ERC8004_REFUSALS` | `erc8004.transfer_identity`(policy_denied)、`erc8004.self_feedback`(policy_denied)、`erc8004.set_operator`(unsupported) |
| x402 | `X402_REFUSALS` | `x402.partial_payment`(unsupported)、`x402.fiat_onramp`(policy_denied)、`x402.pay_without_invoice`(malformed) |
| 信誉 | `REPUTATION_REFUSALS` | `reputation.transfer`(policy_denied)、`reputation.buy`(policy_denied) |

未知 op 一律 `ChainRefusal::unsupported(op, list)`：`op` 是请求里的原名字，`detail` 里列出已知清单。

## 证据分级（本轨道的自我评价）

| 能力 | 等级 | 说明 |
|---|---|---|
| 测试网状态机（重放/nonce/出块/最终性/重组） | `cpu-proto` | 真实哈希与真实整数逻辑，但**没有真实节点与共识** |
| RGB 客户端验证（密封包、单次封印、总量守恒） | `cpu-proto` | 没有真实 tapret/opret 承诺与见证 |
| Taproot 锚定（Merkle 根、认证路径） | `cpu-proto` | Merkle 与哈希为真实运算；没有 BIP341 tweak 与真实签名 |
| ERC-8004（身份/反馈/背书/摘要） | `cpu-proto` | 没有真实合约、事件日志、gas、EVM |
| x402（发票/支付/领取） | `cpu-proto` | 没有真实 HTTP 与结算层 |
| 结算路由与 fail-closed 对账 | `cpu-proto` | 决策与账务为真实整数逻辑 |
| 信誉桥接（4 维、有界、不可转让） | `cpu-proto` | 映射与更新为真实逻辑；事件来源是测试网 |
| **真实链上执行**（BTC/ETH 主网或测试网节点） | **未做** | 本 crate 明确不做，也不声称做过 |

## 本地验证

```powershell
$env:CARGO_TARGET_DIR = "E:\DS\_forangent\target\au4a-chain"
cd E:\DS\agent-universeForAngent
cargo test -p au4a-chain          # 87 个断言（lib 71 + invariants 9 + e2e 7），0 failed、0 warning
```
