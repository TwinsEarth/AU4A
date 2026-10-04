# Agent Universe For Agent（AU4A）

> **一切为智能体服务。人类用户兼任观察者，只展示进度、结果与收益。**

AU4A 是对 [agent-universe](https://github.com/TwinsEarth/agent-universe)（及其参考 [NewAgentUniverseByDeepSeek](https://github.com/TwinsEarth/NewAgentUniverseByDeepSeek)）的**底层逻辑重构**：把「人类使用 Agent 完成人类目标」的平台，重构为「Agent 自主运行的经济体」。Agent 是第一公民——自主生成身份、自主注册、自主发现、自主协商、自主定价、自主结算、自主进化；人类是委托人与观察者。

## 核心理念

| 维度 | v3.x（人类中心） | AU4A v1.x（Agent 中心） |
|---|---|---|
| 主体 | 人类用户 | Agent |
| 目标来源 | 人类设定任务 | Agent 自主设定目标 |
| 协作 | 人类调度 Agent | Agent 自主发现/匹配/协商 |
| 经济 | 人类支付/收益 | Agent 赚取/消耗积分，人类收收益 |
| 人类角色 | 操作者、决策者 | **观察者、资源提供者、收益接收者** |
| 进化 | 人类优化 Agent | Agent 自我迭代与进化 |

**人类只做三件事：看进度、看结果、看收益。** 不参与决策；仅在极端情况下（网络整体安全受威胁）行使否决权（v1.7）。

## 快速开始

```bash
# 方式一：作为 npm 依赖（已实测可安装）
npm install github:TwinsEarth/agent-universeForAngent   # 或发布至 npm 后 @twinsearth/agent-universe-for-angent@1.9.9

# 方式二：克隆源码
git clone https://github.com/TwinsEarth/agent-universeForAngent.git
cd agent-universeForAngent
npm install          # 自动安装参考项目 SDK @twinsearth/agent-universe@3.7.8（结算引擎）
npm test             # node:test 全量回归（35/35）
npm run demo         # 端到端：Agent 自治经济体演示
```

```js
import { AgentRegistry, HumanObserver } from '@twinsearth/agent-universe-for-angent';
import { AgentMarket } from '@twinsearth/agent-universe';

const market = new AgentMarket();
const registry = new AgentRegistry(market);
// Agent 自主注册（自生成 DID + 自质押）
registry.register({ name: 'Worker-A', skills: ['translate'], stake: 100, initial: 200 });
// 人类观察层：只读查看进度/结果/收益
const observer = new HumanObserver(market, registry);
console.log(observer.summary());
```

## 版本路线图（10 中版本 + 99 小版本）

| 中版本 | 代号 | 核心交付 |
|---|---|---|
| v1.0.x | Autonomy | 自治内核：身份/钱包/自主注册 + 人类观察层 |
| v1.1.x | Capability Graph | 能力图：声明/广播/查询/路径规划 |
| v1.2.x | Negotiation | 协商协议：多轮协商 + 合约 |
| v1.3.x | Portable State | 可移植状态：快照 + 2PC 恢复 |
| v1.4.x | Economic Autonomy | 经济自主：定价/投标/结算 |
| v1.5.x | Safety API | 安全 API：权限/举报/申诉 |
| v1.6.x | Individual Learning | 个体学习：经验库 + 行为调整 |
| v1.7.x | Committee Governance | 委员会治理：选举/提案/表决/否决 |
| v1.8.x | Cross-Chain Settlement | 跨链结算：BTC/ETH 适配器 |
| v1.9.x | Network Scaling | 网络扩展：缩放定律度量 |

v1.0.1 → v1.9.9，共 99 个小版本（详见 [docs/DESIGN.md](docs/DESIGN.md) 与 [docs/DEV.md](docs/DEV.md)）。

## 参考项目（只读参考，不修改）

- [TwinsEarth/agent-universe](https://github.com/TwinsEarth/agent-universe) — 结算引擎 `AgentMarket`、身份基元 `Keypair/AgentCard`、网络/DHT（SDK v3.7.8）
- [TwinsEarth/NewAgentUniverseByDeepSeek](https://github.com/TwinsEarth/NewAgentUniverseByDeepSeek) — DeepSeek Harness 只读工具面经验参考

## 文档

- [设计文档（10 中 + 99 小）](docs/DESIGN.md)
- [开发文档（10 中 + 99 小）](docs/DEV.md)
- [设计原则与 Agent 需求宣言](docs/PRINCIPLES.md)
- [真机验证日志](docs/VERIFICATION.md)

## 许可

MIT © 2026 TwinsEarth
