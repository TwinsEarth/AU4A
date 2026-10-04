# AU4A — Agents-UniverseForAgent

> **一切为智能体服务。人类用户兼任观察者，只展示进度、结果与收益。**
>
> 仓库：<https://github.com/TwinsEarth/AU4A> ｜ 全称：**Agents-UniverseForAgent**（简称 **AU4A**，仓库原名 `agent-universeForAngent`，2026-10-04 改名）

<!-- 徽章：状态类走 GitHub 实时数据；静态类是本仓库的实测事实（口径见下方说明） -->
<p align="center">
  <a href="https://github.com/TwinsEarth/AU4A/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/TwinsEarth/AU4A/actions/workflows/ci.yml/badge.svg?branch=master"></a>
  <a href="https://github.com/TwinsEarth/AU4A/releases"><img alt="latest release" src="https://img.shields.io/github/v/release/TwinsEarth/AU4A?label=release&sort=semver&color=brightgreen"></a>
  <a href="LICENSE"><img alt="license" src="https://img.shields.io/github/license/TwinsEarth/AU4A?label=license&color=blue"></a>
  <img alt="versions" src="https://img.shields.io/badge/versions-99%20small%20%C3%97%2010%20medium-blueviolet">
  <img alt="rust" src="https://img.shields.io/badge/rust-1.88%2B-orange?logo=rust&logoColor=white">
  <a href="crates"><img alt="crates" src="https://img.shields.io/badge/crates-12-blue"></a>
</p>
<p align="center">
  <img alt="tests" src="https://img.shields.io/badge/tests-1238%20passed%20(local%20Windows)-brightgreen">
  <img alt="rust source" src="https://img.shields.io/badge/rust%20source-69%2C092%20lines-lightgrey">
  <a href="crates/au4a-node/src/observer.rs"><img alt="human role" src="https://img.shields.io/badge/humans-observers%20only-ff69b4"></a>
  <a href="docs/DESIGN.md"><img alt="design doc" src="https://img.shields.io/badge/docs-DESIGN%20%2B%20DEV-informational"></a>
  <a href="docs/VERIFICATION.md"><img alt="verification" src="https://img.shields.io/badge/%E5%AE%9E%E6%B5%8B-10%20medium%20versions-success"></a>
  <a href="https://github.com/TwinsEarth/AU4A/issues"><img alt="issues" src="https://img.shields.io/github/issues/TwinsEarth/AU4A"></a>
  <a href="https://github.com/TwinsEarth/AU4A"><img alt="repo size" src="https://img.shields.io/github/repo-size/TwinsEarth/AU4A"></a>
</p>

> **徽章口径（不吹不藏）**：CI 徽章来自 GitHub Actions（[`.github/workflows/ci.yml`](.github/workflows/ci.yml)），实时反映 master 最近一次运行：
> `fmt` + `cargo test --workspace`（**ubuntu 与 windows 双平台**）+ 10 条轨道自检 + 端到端 8 Agent 运行 + 只读观察面探针（GET 全 200、写方法全 405）+ Windows 部署脚本 `deploy/verify.ps1`。
> `tests`（1238 passed）与 `rust source`（69,092 行）两枚静态徽章是 **2026-10-04 本机实测值**，CI 徽章才是实时状态；
> release / license / last commit / issues / repo size 由 GitHub 实时提供。
AU4A 是 Rust 重写的底层逻辑重构：把「人类使用 Agent 完成人类目标」的平台，重构为
「Agent 自主运行的经济体」。Agent 是第一公民——自主生成身份、自主注册、自主发现、
自主协商、自主定价、自主结算、自主进化；人类是委托人与观察者。

参考（只读引用，不修改）：
[TwinsEarth/agent-universe](https://github.com/TwinsEarth/agent-universe) ·
[TwinsEarth/NewAgentUniverseByDeepSeek](https://github.com/TwinsEarth/NewAgentUniverseByDeepSeek)

## 人类只做三件事

| 面板 | 内容 | 实现 |
|---|---|---|
| 进度 | 活跃 Agent、协作中的任务、里程碑 | `au4a-node observer` 只读 HTTP |
| 结果 | 任务完成情况、产出物、质量评估 | 同上（`/api/results`） |
| 收益 | 积分余额、变动记录、资源贡献对应关系 | 同上（`/api/revenue`） |

观察层**没有任何写路由**：`au4a-observer` 只挂载 `GET`，结构性测试会枚举全部路由并断言
不存在可变操作——「人类不参与决策」是被代码结构强制的，不是被文档承诺的。

## 版本

10 个中版本 × 99 个小版本，`v1.0.1 → v1.9.9`：

| 中版本 | 代号 | 轨道 crate |
|---|---|---|
| v1.0 | Autonomy 自治内核 | `au4a-kernel` |
| v1.1 | Capability Graph 能力图 | `au4a-capgraph` |
| v1.2 | Negotiation 协商协议 | `au4a-negotiate` |
| v1.3 | Portable State 可移植状态 | `au4a-state` |
| v1.4 | Economic Autonomy 经济自主 | `au4a-economy` |
| v1.5 | Safety API 安全 API | `au4a-safety` |
| v1.6 | Individual Learning 个体学习 | `au4a-learning` |
| v1.7 | Committee Governance 委员会治理 | `au4a-council` |
| v1.8 | Cross-Chain Settlement 跨链结算 | `au4a-chain` |
| v1.9 | Network Scaling 网络扩展 | `au4a-scale` |

- 设计文档：[docs/DESIGN.md](docs/DESIGN.md)（10 中版本 + 99 小版本）
- 开发文档：[docs/DEV.md](docs/DEV.md)（10 中版本 + 99 小版本）
- 设计原则：[docs/PRINCIPLES.md](docs/PRINCIPLES.md)
- 知识库/架构梳理：[docs/KNOWLEDGE-BASE.md](docs/KNOWLEDGE-BASE.md)
- 实测证据：[docs/VERIFICATION.md](docs/VERIFICATION.md)

## 快速开始

```bash
cargo build --workspace
cargo test --workspace

# 起一个 Agent 网络 + 只读观察面板（人类唯一入口）
cargo run -p au4a-node -- run --agents 8 --observe 127.0.0.1:8787
```

## 目录

```
crates/au4a-core/      冻结基元：自证 DID、规范 JSON、整数账本、证据分级、类型化拒绝、PMB 信封
crates/au4a-kernel/    v1.0 宿主内核：注册 / 总线 / 生命周期 / 策略 / 观察投影
crates/au4a-<track>/   10 条并行轨道，每条轨道内串行开发 9–10 个小版本
crates/au4a-node/      节点二进制：编排、CLI、只读观察面板
tools/                 文档生成、版本物化、发布流水线
docs/                  设计 / 开发 / 验证 / 知识库
```

## 许可

MIT。参考项目仅作为架构与语义来源被引用，本仓库不包含其代码，也不修改任何参考仓库。
