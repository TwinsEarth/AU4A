# AU4A 实测与验证（VERIFICATION）

> 本文件由 `tools/verify-mediums.mjs` 生成：对 10 个中版本收尾点逐个**重建当时的版本树 → 真实构建 → 真实运行 → 只读面板核验**。
> 证据分级：本机 Windows 实测为 `verified`；模拟/内存级实现为 `cpu-proto`；未实测项标 `unverified`。

## 汇总

| 中版本 | 收尾版本 | 代号 | 构建 | verify | 端到端 | 只读面 | 文件数 | 构建耗时 | 证据 |
|---|---|---|---|---|---|---|---|---|---|
| v1.0 | `v1.0.10` | Autonomy 自治内核 | ✅ | ❌ | ✅ | ✅ | 267 | 91.6s | [medium-v1.0.md](verify/medium-v1.0.md) |
| v1.1 | `v1.1.10` | Capability Graph 能力图 | ✅ | ✅ | ✅ | ✅ | 267 | 94.6s | [medium-v1.1.md](verify/medium-v1.1.md) |
| v1.2 | `v1.2.10` | Negotiation 协商协议 | ✅ | ✅ | ✅ | ✅ | 267 | 95.6s | [medium-v1.2.md](verify/medium-v1.2.md) |
| v1.3 | `v1.3.10` | Portable State 可移植状态 | ✅ | ✅ | ✅ | ✅ | 267 | 97.4s | [medium-v1.3.md](verify/medium-v1.3.md) |
| v1.4 | `v1.4.10` | Economic Autonomy 经济自主 | ✅ | ✅ | ✅ | ✅ | 267 | 95.8s | [medium-v1.4.md](verify/medium-v1.4.md) |
| v1.5 | `v1.5.10` | Safety API 安全 API | ✅ | ✅ | ✅ | ✅ | 267 | 95.6s | [medium-v1.5.md](verify/medium-v1.5.md) |
| v1.6 | `v1.6.10` | Individual Learning 个体学习 | ✅ | ✅ | ✅ | ✅ | 267 | 96.4s | [medium-v1.6.md](verify/medium-v1.6.md) |
| v1.7 | `v1.7.10` | Committee Governance 委员会治理 | ✅ | ✅ | ✅ | ✅ | 267 | 99.4s | [medium-v1.7.md](verify/medium-v1.7.md) |
| v1.8 | `v1.8.10` | Cross-Chain Settlement 跨链结算 | ✅ | ✅ | ✅ | ✅ | 267 | 99.1s | [medium-v1.8.md](verify/medium-v1.8.md) |
| v1.9 | `v1.9.9` | Network Scaling 网络扩展 | ✅ | ✅ | ✅ | ✅ | 267 | 98.3s | [medium-v1.9.md](verify/medium-v1.9.md) |

## 每步做了什么

1. `tools/materialize.mjs` 从逐版本快照重建**该版本当时的树**（不是最终树），并戳 `VERSION`。
2. `cargo build --release -p au4a-node` 真实构建。
3. `au4a-node version` / `verify --json` 核验版本号与 10 条轨道自检。
4. `au4a-node run --agents 8 --json` 让 Agent 自主跑完整链路。
5. 起只读观察面，GET 三个面板与自检；再用 **POST** 证明写路径不存在（必须 405）。

## 未验证项（诚实清单）

- 跨链部分（v1.8）是确定性测试网适配器，**不是真实链上交易**：证据等级 `cpu-proto`。
- 状态迁移（v1.3）为本地双节点内存实现，**不是真实网络传输**：证据等级 `cpu-proto`。
- 10k 节点（v1.9）为确定性聚合模拟，**不是真实分布式压测**：证据等级 `cpu-proto`。
- 仅 Windows 本机实测；Linux/macOS 未在本轮运行。

---

> 本文档由 `tools/verify-mediums.mjs` 生成；请勿手改。
