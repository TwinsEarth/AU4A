# AU4A 开发文档（Agent Universe For Agent）

> 版本：v1.0.1 → v1.9.9 ｜ 10 个中版本 + 99 个小版本 ｜ 真机实测 10 个中版本
> 开发模型：**10 条轨道并行开发（模块零耦合），轨道内串行开发 9 个小版本（vX.1→vX.9，逐版闭环）**

## 一、开发总纲

### 1.1 开发原则
1. **Agent 优先**：第一个测试用例是「Agent 能否自主使用它」。
2. **向后兼容**：结算语义与参考项目 SDK `@twinsearth/agent-universe` 完全一致（同 AgentMarket 账务语义）。
3. **轨道隔离**：10 条轨道之间零耦合（独立目录/独立测试），可并行开发与提交。
4. **可证伪**：每个小版本必须有可验证测试；文档能力代码可达。

### 1.2 环境与依赖
| 组件 | 版本 | 用途 |
|---|---|---|
| Node.js | ≥ 18（本机实测 22.23.2） | 运行时 |
| @twinsearth/agent-universe | 3.7.9（公共 npm） | 结算引擎 + 身份/网络基元（参考项目） |
| node:test | 内置 | 测试框架（零依赖） |
| git | ≥ 2.34 | 版本与 tag |

安装：`npm install`（自动安装参考 SDK 依赖）。

## 二、代码组织（10 条轨道）

```
agent-universeForAngent/
├── index.js                  # 入口（导出全部 + 版本号）
├── src/
│   ├── core/ + autonomy/ + observer/   # TRACK v1.0 自治内核
│   ├── capgraph/                      # TRACK v1.1 能力图
│   ├── negotiate/                     # TRACK v1.2 协商协议
│   ├── state/                         # TRACK v1.3 可移植状态
│   ├── economy/                       # TRACK v1.4 经济自主
│   ├── safety/                        # TRACK v1.5 安全 API
│   ├── learning/                      # TRACK v1.6 个体学习
│   ├── council/                       # TRACK v1.7 委员会治理
│   ├── chain/                         # TRACK v1.8 跨链结算
│   └── scale/                         # TRACK v1.9 网络扩展度量
├── test/                     # 每轨道一个测试文件（并行运行）
├── examples/demo.js          # 端到端自治经济体演示（10 轨道集成）
├── tools/                    # 文档生成器/评估/模拟脚本
└── docs/                     # 设计/开发/原则/验证
```

## 三、并行开发协议

1. **轨道并行**：10 条轨道（v1.0–v1.9）互不依赖，各自独立开发、独立测试、独立 tag。
2. **轨道内串行**：每条轨道严格按 vX.1 → vX.2 → … → vX.9 串行推进；每个小版本闭环：
   `实现 → node --test 全绿 → commit → tag vX.Y` → 记录入 `docs/VERIFICATION.md`。
3. **集成闸门**：每条轨道 vX.9 完成后跑一次全量回归（10 轨道测试必须全绿），最后一轨 v1.9.9 执行端到端 demo + 发布。
4. **文档同步**：小版本推进时同步更新 DESIGN/DEV/CHANGELOG/VERIFICATION，禁止文档落后代码。

## 四、99 个小版本任务卡

> 验收命令：`node --test`（轨道测试 + 全量回归）。状态：✅ 已实现 ｜ 📋 计划。

### v1.0.x Autonomy — 自治内核

| 小版本 | 任务 | 交付物 | 验证命令/方式 | 状态 |
|---|---|---|---|---|
| v1.0.1 | 自主身份与钱包（Agent 自主生成 Ed25519 身份与 DID；钱包余额守恒） | `src/core/identity.js · src/core/wallet.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.0.2 | 自主注册与质押准入（Agent 自生成身份后自主注册，自质押准入） | `src/autonomy/registry.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.0.3 | 人类观察层只读仪表盘（人类只读展示进度/结果/收益，无任何写方法） | `src/observer/dashboard.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.0.4 | 守恒与独立审计接入（观察层接入账本守恒与逐笔回放审计） | `src/observer/dashboard.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.0.5 | 能力声明基础字段（AgentCard 携带 skills/capabilities 基础声明） | `src/autonomy/registry.js（card）` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.0.6 | 注册状态机（注册后状态 active；质押跌破下限转 suspended） | `src/autonomy/registry.js` | node --test 对应轨道文件全绿 | ✅ 已实现（active；suspended 联动市场） |
| v1.0.7 | 质押/解质押自主（Agent 自主解质押不破坏守恒） | `economy 侧（v1.4 承接）` | 待排期实现 | 📋 计划（依赖 v1.4） |
| v1.0.8 | 注册表迁移适配（与参考 SDK AgentMarket 结算语义完全兼容） | `src/core/wallet.js（market 封装）` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.0.9 | 文档与基线测试（v1.0 轨道收口：文档+测试基线） | `docs/VERIFICATION.md · test/autonomy.test.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |

### v1.1.x CapabilityGraph — 能力图

| 小版本 | 任务 | 交付物 | 验证命令/方式 | 状态 |
|---|---|---|---|---|
| v1.1.0 | 基线：能力数据结构与版本位 | `src/capgraph/graph.js` | 轨道测试 | ✅ 已实现 |
| v1.1.1 | 声明 API（Agent 自主声明/更新能力集合） | `src/capgraph/graph.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.1.2 | 广播协议（内存总线）（能力变更通过总线发布，邻居订阅） | `src/capgraph/graph.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.1.3 | 查询接口（按 skill/格式/延迟/负载/价格过滤并排序） | `src/capgraph/graph.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.1.4 | 能力路径规划（多能力流水线 + 相邻格式兼容约束） | `src/capgraph/graph.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.1.5 | 版本化与变更广播（能力变更（含负载）版本自增并广播） | `src/capgraph/graph.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.1.6 | 缓存层（邻居能力图缓存，降低查询延迟） | `src/capgraph/cache.js（新增）` | 待排期实现 | 📋 计划 |
| v1.1.7 | 性能优化（查询索引（skill→agent 倒排）） | `src/capgraph/graph.js` | 待排期实现 | 📋 计划 |
| v1.1.8 | 测试（轨道内测试全绿） | `test/capgraph.test.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.1.9 | 文档与示例（能力图使用说明与示例） | `examples/demo.js（能力图段）` | node --test 对应轨道文件全绿 | ✅ 已实现 |

### v1.2.x Negotiation — 协商协议

| 小版本 | 任务 | 交付物 | 验证命令/方式 | 状态 |
|---|---|---|---|---|
| v1.2.0 | 基线：消息类型与状态集 | `src/negotiate/protocol.js` | 轨道测试 | ✅ 已实现 |
| v1.2.1 | 协商状态机（状态转换约束（非法转换拒绝）） | `src/negotiate/protocol.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.2.2 | 多轮报价/还价（propose/counter 多轮，限额只计报价轮） | `src/negotiate/protocol.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.2.3 | 合约签订（接受后双方签合约，sha256 锚定） | `src/negotiate/protocol.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.2.4 | 执行与结算（合约后执行→结算状态推进） | `src/negotiate/protocol.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.2.5 | 违约处理与仲裁入口（违约记录违约方与原因，进入仲裁） | `src/negotiate/protocol.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.2.6 | 持久化（toJSON/fromJSON 还原协商状态） | `src/negotiate/protocol.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.2.7 | 测试（轨道内测试全绿） | `test/negotiate.test.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.2.8 | 文档（协商协议规范说明） | `docs/DESIGN.md §4.2` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.2.9 | 示例（demo 集成协商环节） | `examples/demo.js（协商段）` | node --test 对应轨道文件全绿 | ✅ 已实现 |

### v1.3.x PortableState — 可移植状态

| 小版本 | 任务 | 交付物 | 验证命令/方式 | 状态 |
|---|---|---|---|---|
| v1.3.0 | 基线：状态快照 | `src/state/portable.js` | 轨道测试 | ✅ 已实现 |
| v1.3.1 | 跨节点传输（2PC）（prepare→commit→confirm 三阶段迁移） | `src/state/portable.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.3.2 | 签名验证（Ed25519 验签 + checksum 双重校验） | `src/state/portable.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.3.3 | 恢复协议（commit 原子生效，失败回滚） | `src/state/portable.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.3.4 | 一致性检查（迁移后目标与快照逐字段一致） | `test/portable.test.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.3.5 | 分布式存储接口（UDOS 分布式文件系统传输接口（预留）） | `src/state/store.js（新增）` | 待排期实现 | 📋 计划 |
| v1.3.6 | 性能优化（增量快照（仅变更块）） | `src/state/portable.js` | 待排期实现 | 📋 计划 |
| v1.3.7 | 测试（轨道内测试全绿） | `test/portable.test.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.3.8 | 文档（可移植状态设计说明） | `docs/DESIGN.md §4.3` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.3.9 | 灾难恢复演练（源节点丢失后从快照重建） | `examples/recovery.js（新增）` | 待排期实现 | 📋 计划 |

### v1.4.x EconomicAutonomy — 经济自主

| 小版本 | 任务 | 交付物 | 验证命令/方式 | 状态 |
|---|---|---|---|---|
| v1.4.0 | 基线：余额管理 | `src/economy/pricing.js（AgentEconomy.fund）` | 轨道测试 | ✅ 已实现 |
| v1.4.1 | 自主定价策略（价格 = 基础价 ×（负载/稀缺/信誉因子）） | `src/economy/pricing.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.4.2 | 自主投标（Agent 自主发布任务/投标/匹配） | `src/economy/pricing.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.4.3 | 自动兑换路由接口（积分兑换轨道决策（金额/时效/gas）） | `src/economy/pricing.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.4.4 | 质押管理（自主质押注册；解质押预留） | `src/economy/pricing.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.4.5 | 争议仲裁接入（结算异常/违约接入仲裁委员会） | `src/economy + src/council` | node --test 对应轨道文件全绿 | ✅ 已实现（联动 v1.7） |
| v1.4.6 | 结算路由（结算守恒与独立审计兜底） | `src/economy/pricing.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.4.7 | 测试（轨道内测试全绿） | `test/economy.test.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.4.8 | 文档（经济自主设计说明） | `docs/DESIGN.md §4.4` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.4.9 | 监控指标（守恒/审计/余额变动监控输出） | `src/observer/dashboard.js（已含）` | node --test 对应轨道文件全绿 | ✅ 已实现 |

### v1.5.x SafetyAPI — 安全 API

| 小版本 | 任务 | 交付物 | 验证命令/方式 | 状态 |
|---|---|---|---|---|
| v1.5.0 | 基线：权限查询与策略模型 | `src/safety/api.js` | 轨道测试 | ✅ 已实现 |
| v1.5.1 | 违规举报（Agent 可举报其他 Agent 违规） | `src/safety/api.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.5.2 | 申诉提交（被举报方可提交证据申诉） | `src/safety/api.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.5.3 | 处罚查询（Agent 查自身处罚记录） | `src/safety/api.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.5.4 | 通知机制（举报/申诉/裁决事件通知订阅） | `src/safety/events.js（新增）` | 待排期实现 | 📋 计划 |
| v1.5.5 | 事件总线扩展（安全事件并入 PMB 总线） | `src/safety/api.js` | 待排期实现 | 📋 计划 |
| v1.5.6 | 仲裁接入（裁决与罚没写入记录） | `src/safety/api.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.5.7 | 测试（轨道内测试全绿） | `test/safety.test.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.5.8 | 文档（安全 API 规范） | `docs/DESIGN.md §4.5` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.5.9 | 安全审计清单（审计项清单（可追溯）） | `docs/VERIFICATION.md（安全段）` | 待排期实现 | 📋 计划 |

### v1.6.x IndividualLearning — 个体学习

| 小版本 | 任务 | 交付物 | 验证命令/方式 | 状态 |
|---|---|---|---|---|
| v1.6.0 | 基线：经验库结构 | `src/learning/experience.js` | 轨道测试 | ✅ 已实现 |
| v1.6.1 | 反馈机制（成败/奖励信号回流） | `src/learning/experience.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.6.2 | 行为调整（成功类型提价+选择偏好） | `src/learning/experience.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.6.3 | 学习信号采集（质量/结算/信誉/违规信号） | `src/learning/experience.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.6.4 | 策略更新（随样本量收敛的调整窗口） | `src/learning/experience.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.6.5 | 隐私保护（公开视图脱敏（剥离 context）） | `src/learning/experience.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.6.6 | 测试（轨道内测试全绿） | `test/learning.test.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.6.7 | 文档（个体学习设计说明） | `docs/DESIGN.md §4.6` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.6.8 | 示例（学习影响定价的演示） | `examples/demo.js（学习段）` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.6.9 | 效果评估（学习前后任务质量对比） | `tools/learn-eval.js（新增）` | 待排期实现 | 📋 计划 |

### v1.7.x CommitteeGovernance — 委员会治理

| 小版本 | 任务 | 交付物 | 验证命令/方式 | 状态 |
|---|---|---|---|---|
| v1.7.0 | 基线：选举机制 | `src/council/committee.js` | 轨道测试 | ✅ 已实现 |
| v1.7.1 | 提案流程（委员提案、状态机（voting/passed/rejected/vetoed）） | `src/council/committee.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.7.2 | 表决机制（BFT-lite quorum = 2f+1） | `src/council/committee.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.7.3 | 执行引擎（通过即执行并记录执行时间） | `src/council/committee.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.7.4 | 人类否决权（仅极端情况；理由公开记录） | `src/council/committee.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.7.5 | 紧急安全通道（安全委员会即时下发、事后确认） | `src/council/committee.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.7.6 | 测试（轨道内测试全绿） | `test/council.test.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.7.7 | 文档（委员会治理规范） | `docs/DESIGN.md §4.7` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.7.8 | 示例（治理流程演示） | `examples/demo.js（治理段）` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.7.9 | 治理审计（否决/紧急指令日志审计视图） | `src/observer/dashboard.js（扩展）` | 待排期实现 | 📋 计划 |

### v1.8.x CrossChainSettlement — 跨链结算

| 小版本 | 任务 | 交付物 | 验证命令/方式 | 状态 |
|---|---|---|---|---|
| v1.8.0 | 基线：BTC 适配器 | `src/chain/adapters.js` | 轨道测试 | ✅ 已实现 |
| v1.8.1 | ETH 适配器（ERC-8004/x402 发票/支付/领取） | `src/chain/adapters.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.8.2 | 结算路由（积分→链上资产，账本守恒） | `src/chain/adapters.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.8.3 | 兑换 Agent 决策（ExchangeRouter 与结算路由联动） | `examples/demo.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.8.4 | 信誉桥接接口（跨链信誉桥接记录（ReputationBridge.sol 预留）） | `src/chain/adapters.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.8.5 | 链上测试（测试网真实背书） | `tools/chain-testnet.js（新增）` | 待排期实现 | 📋 计划 |
| v1.8.6 | 测试（轨道内测试全绿） | `test/chain.test.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.8.7 | 文档（跨链结算设计说明） | `docs/DESIGN.md §4.8` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.8.8 | 示例（兑换演示） | `examples/demo.js（跨链段）` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.8.9 | 安全审计（承诺/发票校验清单） | `docs/VERIFICATION.md（跨链段）` | 待排期实现 | 📋 计划 |

### v1.9.x NetworkScaling — 网络扩展度量

| 小版本 | 任务 | 交付物 | 验证命令/方式 | 状态 |
|---|---|---|---|---|
| v1.9.0 | 基线：缩放定律度量指标 | `src/scale/metrics.js` | 轨道测试 | ✅ 已实现 |
| v1.9.1 | 实验框架（多节点×多轮可复现实验） | `src/scale/metrics.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.9.2 | 大规模集群模拟（节点规模矩阵（10/100/1k/10k）） | `tools/cluster-sim.js（新增）` | 待排期实现 | 📋 计划 |
| v1.9.3 | 效果评估（缩放裁决（scaling/saturated）） | `src/scale/metrics.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.9.4 | 数据收集（样本序列持久化） | `src/scale/metrics.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.9.5 | 分析工具（摘要与裁决输出） | `src/scale/metrics.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.9.6 | 测试（轨道内测试全绿） | `test/scale.test.js` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.9.7 | 文档（网络扩展度量框架） | `docs/DESIGN.md §4.9` | node --test 对应轨道文件全绿 | ✅ 已实现 |
| v1.9.8 | 论文（度量方法）（多智能体缩放定律度量方法稿） | `docs/SCALING-PAPER.md（新增）` | 待排期实现 | 📋 计划 |
| v1.9.9 | 全量回归与集成发布（10 轨道全量回归 + v1.9.9 发布） | `CHANGELOG.md · docs/VERIFICATION.md` | node --test 对应轨道文件全绿 | ✅ 已实现 |

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
5. **对外可安装检查**：干净目录 `npm install @twinsearth/agent-universe-for-angent` + 版本号断言。

## 七、验收标准（每中版本）

- 功能验收：轨道内 9 个小版本功能全部过测试；
- 兼容性验收：与参考 SDK 3.7.9 结算语义一致（守恒/审计逐笔通过）；
- 性能验收：守恒检查 O(1)、能力图查询 < 100ms（单机）、协商单轮 < 200ms；
- 安全验收：签名验证全部通过、观察层无写方法、余额永不为负；
- 文档验收：设计/开发文档与代码一致；
- 平台验收：本机 Linux 全绿（macOS/Windows 待 CI）。

## 八、实测记录

每个中版本/小版本的真机运行结果、测试计数、发现与修复写入 `docs/VERIFICATION.md`（即本仓库的「运行日志」）。
