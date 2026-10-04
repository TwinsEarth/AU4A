# AU4A 开发文档（Agent Universe For Agent）

> 版本：v1.0.1 → v1.9.9 ｜ 10 个中版本 + 99 个小版本 ｜ 真机实测 10 个中版本

## 一、开发总纲

### 1.1 开发原则
1. **Agent 优先**：第一个测试用例是「Agent 能否自主使用它」。
2. **向后兼容**：结算语义与参考项目 SDK `@twinsearth/agent-universe` 完全一致（同 AgentMarket 账务语义）。
3. **渐进式重构**：一次只重构一个模块；每中版本聚焦单一主题。
4. **可证伪**：每个新功能必须有可验证测试；文档能力代码可达。

### 1.2 环境与依赖
| 组件 | 版本 | 用途 |
|---|---|---|
| Node.js | ≥ 18（本机实测 22.23.2） | 运行时 |
| @twinsearth/agent-universe | 3.7.8（公共 npm） | 结算引擎 + 身份/网络基元（参考项目） |
| node:test | 内置 | 测试框架（零依赖） |
| git | ≥ 2.34 | 版本与 tag |

安装：`npm install`（会自动安装参考 SDK 依赖）。

## 二、代码组织

```
agent-universeForAngent/
├── index.js                  # 入口（导出全部 + 版本号）
├── src/
│   ├── core/                 # v1.0 身份/钱包
│   │   ├── identity.js
│   │   └── wallet.js
│   ├── autonomy/             # v1.0 自主注册
│   │   └── registry.js
│   ├── observer/             # v1.0 人类观察层（只读）
│   │   └── dashboard.js
│   ├── capgraph/             # v1.1 能力图
│   │   └── graph.js
│   ├── negotiate/            # v1.2 协商协议
│   │   └── protocol.js
│   ├── state/                # v1.3 可移植状态
│   │   └── portable.js
│   ├── economy/              # v1.4 经济自主
│   │   └── pricing.js
│   ├── safety/               # v1.5 安全 API
│   │   └── api.js
│   ├── learning/             # v1.6 个体学习
│   │   └── experience.js
│   ├── council/              # v1.7 委员会治理
│   │   └── committee.js
│   ├── chain/                # v1.8 跨链结算
│   │   └── adapters.js
│   └── scale/                # v1.9 网络扩展度量
│       └── metrics.js
├── test/                     # node:test 套件（每中版本一个）
├── examples/demo.js          # 端到端自治经济体演示
└── docs/                     # 设计/开发/原则/验证
```

## 三、每中版本开发文档框架

每个中版本提交时包含：
1. 版本目标（一句话）
2. 前置条件（依赖的中版本）
3. 功能清单（覆盖的小版本）
4. 接口定义（类/方法签名）
5. 数据结构
6. 状态机
7. 测试用例（node:test，全绿）
8. 实测验证（记录入 docs/VERIFICATION.md）
9. 风险与缓解
10. 验收标准

## 四、99 个小版本执行清单

| 中版本 | 小版本区间 | 数量 | 主题 |
|---|---|---|---|
| v1.0.x | v1.0.1–v1.0.9 | 9 | 自治内核（身份/钱包/自主注册/观察层/守恒审计） |
| v1.1.x | v1.1.0–v1.1.9 | 10 | 能力图（声明/广播/缓存/查询/路径规划/版本化） |
| v1.2.x | v1.2.0–v1.2.9 | 10 | 协商协议（消息/状态机/持久化/合约/违约/仲裁） |
| v1.3.x | v1.3.0–v1.3.9 | 10 | 可移植状态（快照/传输/验签/2PC 恢复/一致性） |
| v1.4.x | v1.4.0–v1.4.9 | 10 | 经济自主（余额/定价/投标/兑换/质押/结算路由） |
| v1.5.x | v1.5.0–v1.5.9 | 10 | 安全 API（权限/举报/申诉/处罚/通知/审计） |
| v1.6.x | v1.6.0–v1.6.9 | 10 | 个体学习（经验库/反馈/行为调整/隐私） |
| v1.7.x | v1.7.0–v1.7.9 | 10 | 委员会治理（选举/提案/表决/否决/紧急通道） |
| v1.8.x | v1.8.0–v1.8.9 | 10 | 跨链结算（BTC/ETH 适配器/路由/信誉桥接） |
| v1.9.x | v1.9.0–v1.9.9 | 10 | 网络扩展（缩放度量/实验框架/集群/评估/论文） |
| **合计** | v1.0.1–v1.9.9 | **99** | — |

每个小版本的验收：`npm test` 全绿 + 行为由至少一个能区分错误实现的测试覆盖。

## 五、部署验证矩阵

| 平台 | 验证内容 | 自动化 |
|---|---|---|
| Linux（本机 Cloud VM） | 完整功能 + node:test 全绿 + demo 运行 | ✅ `npm test` |
| macOS | 完整功能 + aarch64 | CI 待接 |
| Windows | 核心功能 | CI 待接 |
| 跨网络 | P2P 发现/GossipSub/DHT（参考 SDK 能力） | 手动+脚本 |
| 跨链 | BTC/ETH 适配器（测试网 mock） | 测试网 |

## 六、发布 SOP

1. **版本管理**：更新 package.json `version`、CHANGELOG、docs/VERIFICATION.md。
2. **查漏检验**：`node --test` 全部通过；无未使用依赖告警。
3. **补缺补齐**：README 对齐、示例对齐、文档同步。
4. **GitHub 发布**：commit → tag `vX.Y.Z` → push → GitHub Release（附 Release Notes + 验证摘要）。
5. **对外可安装检查**：干净目录 `npm install` + `node -e "require('@twinsearth/agent-universe-for-angent')"`。

## 七、验收标准（每中版本）

- 功能验收：小版本功能全部过测试；
- 兼容性验收：与参考 SDK 3.7.8 结算语义一致（守恒/审计逐笔通过）；
- 性能验收：守恒检查 O(1)、能力图查询 < 100ms（单机）、协商单轮 < 200ms；
- 安全验收：签名验证全部通过、观察层无写方法、余额永不为负；
- 文档验收：设计/开发文档与代码一致；
- 平台验收：本机 Linux 全绿（macOS/Windows 待 CI）。

## 八、实测记录

每个中版本的真机运行结果、测试计数、发现与修复写入 `docs/VERIFICATION.md`（即本仓库的「运行日志」）。
