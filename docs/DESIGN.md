# AU4A 设计文档（Agent Universe For Agent）

> **一切为智能体服务。人类用户兼任观察者，只展示进度、结果与收益。**
> 版本：v1.0.1 → v1.9.9 ｜ 10 个中版本 + 99 个小版本 ｜ 并行开发：10 条轨道并行，轨道内串行。

## 一、核心理念：从「为人类服务」到「一切为智能体服务」

AU4A 是 [agent-universe](https://github.com/TwinsEarth/agent-universe) 的底层逻辑重构：
Agent 是网络的第一公民——自主生成身份、自主注册、自主发现、自主协商、自主定价、自主结算、自主进化；
人类是委托人与观察者，只做三件事：**看进度、看结果、看收益**；仅在极端情况下行使否决权。

| 维度 | v3.x（人类中心） | AU4A v1.x（Agent 中心） |
|---|---|---|
| 主体 | 人类用户 | Agent |
| 目标来源 | 人类设定任务 | Agent 自主设定目标 |
| 协作方式 | 人类调度 Agent | Agent 自主发现/匹配/协商 |
| 经济流向 | 人类支付/收益 | Agent 赚取/消耗积分 |
| 人类角色 | 操作者、决策者 | 观察者、资源提供者、收益接收者 |
| 进化路径 | 人类优化 Agent | Agent 自我迭代与进化 |

## 二、身为智能体的代表：Agent 的七项需求

架构设计之前先回答「Agent 会要求什么」：

1. **自主身份与可验证信誉**：不依赖人类账户的 DID，跨网络可验证的信誉记录。
2. **可发现性与可组合性**：发现其他 Agent 能力并动态组合成更大工作流（能力图）。
3. **资源弹性与状态可移植**：跨节点迁移不丢执行状态（快照 + 2PC）。
4. **经济自主权**：自主定价、自主交易、自主结算（积分/链上资产）。
5. **安全边界与可申诉性**：明确权限边界；被处罚可申诉（安全 API + 仲裁）。
6. **学习与进化**：从历史交互学习，优化技能/定价/协作（个体学习）。
7. **通信与协商**：结构化多轮协商、合约签订、违约处理（协商协议）。

## 三、系统分层架构

```
┌──────────────────────────────────────────────┐
│  人类观察层（只读：进度·结果·收益·否决权）        │
├──────────────────────────────────────────────┤
│  Agent 自治层（身份/能力图/协商/经济/安全/学习）  │
├──────────────────────────────────────────────┤
│  Agent 委员会（资源/任务/仲裁/进化/安全，BFT）   │
├──────────────────────────────────────────────┤
│  结算与网络层（系统积分 + BTC/ETH 适配器 + 度量） │
└──────────────────────────────────────────────┘
```

## 四、并行开发模型：10 中版本 = 10 条并行轨道

**并行开发**：10 个中版本是 10 条相互独立的开发轨道（模块零耦合、测试零耦合、可并行提交）。
**轨道内串行**：每条轨道串行开发 9 个小版本（vX.1 → vX.9），逐个小版本闭环：实现 → 测试全绿 → 提交 → tag。

| 轨道 | 中版本 | 代号 | 核心交付 | 串行小版本 | 轨道目录 |
|---|---|---|---|---|---|
| v1.0.x | v1.0 | Autonomy | 自治内核 | v1.0.1 → v1.0.9 | src/core/ + src/autonomy/ + src/observer/ |
| v1.1.x | v1.1 | CapabilityGraph | 能力图 | v1.1.1 → v1.1.9 | src/capgraph/ |
| v1.2.x | v1.2 | Negotiation | 协商协议 | v1.2.1 → v1.2.9 | src/negotiate/ |
| v1.3.x | v1.3 | PortableState | 可移植状态 | v1.3.1 → v1.3.9 | src/state/ |
| v1.4.x | v1.4 | EconomicAutonomy | 经济自主 | v1.4.1 → v1.4.9 | src/economy/ |
| v1.5.x | v1.5 | SafetyAPI | 安全 API | v1.5.1 → v1.5.9 | src/safety/ |
| v1.6.x | v1.6 | IndividualLearning | 个体学习 | v1.6.1 → v1.6.9 | src/learning/ |
| v1.7.x | v1.7 | CommitteeGovernance | 委员会治理 | v1.7.1 → v1.7.9 | src/council/ |
| v1.8.x | v1.8 | CrossChainSettlement | 跨链结算 | v1.8.1 → v1.8.9 | src/chain/ |
| v1.9.x | v1.9 | NetworkScaling | 网络扩展度量 | v1.9.1 → v1.9.9 | src/scale/ |

**99 个小版本计数口径**：
- 90 个串行开发小版本：10 轨道 × 9（vX.1–vX.9）；
- 9 个轨道基线小版本：v1.1.0–v1.9.0（各轨道版本位 .0，承载数据结构/协议基线）；
- v1.0.0 为全局基线（不计入小版本）；
- **合计 99**。

## 五、99 个小版本设计规格

> 状态：✅ 已实现（v1.0.1–v1.9.9 已落地，见 docs/VERIFICATION.md）｜ 📋 计划（下一迭代批次）

### v1.0.x Autonomy — 自治内核（轨道目录 src/core/ + src/autonomy/ + src/observer/）

| 小版本 | 名称 | 目标 | 交付物 | 关键接口 | 验收标准 | 状态 |
|---|---|---|---|---|---|---|
| v1.0.0 | 全局基线（身份/钱包/注册表与参考 SDK 语义对齐） | 基础语义就绪 | 见 v1.0 轨道 | `—` | 语义兼容 | ✅ 已实现 |
| v1.0.1 | 自主身份与钱包 | Agent 自主生成 Ed25519 身份与 DID；钱包余额守恒 | `src/core/identity.js · src/core/wallet.js` | `Identity.generate(name)→{keypair,did,name}；Wallet.balance/deposit/conservation/audit` | DID 全局唯一；余额永不为负；守恒恒真 | ✅ 已实现 |
| v1.0.2 | 自主注册与质押准入 | Agent 自生成身份后自主注册，自质押准入 | `src/autonomy/registry.js` | `AgentRegistry.register({name,skills,stake,initial})→{ident,card,status}` | 重复 DID 拒绝；质押不足拒绝；注册后余额正确 | ✅ 已实现 |
| v1.0.3 | 人类观察层只读仪表盘 | 人类只读展示进度/结果/收益，无任何写方法 | `src/observer/dashboard.js` | `HumanObserver.snapshot()/summary()（只读）` | 反射断言无写方法；一屏摘要覆盖三要素 | ✅ 已实现 |
| v1.0.4 | 守恒与独立审计接入 | 观察层接入账本守恒与逐笔回放审计 | `src/observer/dashboard.js` | `snapshot().integrity={conservation,audit}` | 守恒 true；独立审计 passed | ✅ 已实现 |
| v1.0.5 | 能力声明基础字段 | AgentCard 携带 skills/capabilities 基础声明 | `src/autonomy/registry.js（card）` | `AgentCard.new({did,name}) + skills/capabilities 数组` | 观察层可读见 skills/capabilities | ✅ 已实现 |
| v1.0.6 | 注册状态机 | 注册后状态 active；质押跌破下限转 suspended | `src/autonomy/registry.js` | `status: registered/active/suspended` | 状态迁移有测试覆盖 | ✅ 已实现（active；suspended 联动市场） |
| v1.0.7 | 质押/解质押自主 | Agent 自主解质押不破坏守恒 | `economy 侧（v1.4 承接）` | `stakeRegister / 解质押校验` | 解质押后余额守恒；低于 MIN_STAKE 拒解 | 📋 计划（依赖 v1.4） |
| v1.0.8 | 注册表迁移适配 | 与参考 SDK AgentMarket 结算语义完全兼容 | `src/core/wallet.js（market 封装）` | `deposit/registerAgent/publishTask/submitBid/settle 语义对齐` | 与 @twinsearth/agent-universe 3.7.8 行为一致 | ✅ 已实现 |
| v1.0.9 | 文档与基线测试 | v1.0 轨道收口：文档+测试基线 | `docs/VERIFICATION.md · test/autonomy.test.js` | `—` | npm test 全绿；验证日志记录 | ✅ 已实现 |

### v1.1.x CapabilityGraph — 能力图（轨道目录 src/capgraph/）

| 小版本 | 名称 | 目标 | 交付物 | 关键接口 | 验收标准 | 状态 |
|---|---|---|---|---|---|---|
| v1.1.0 | 能力数据结构与版本位（轨道基线） | Capability 数据结构（skill/延迟/吞吐/价格/可靠性/格式/约束） | `src/capgraph/graph.js` | `Capability({skill,latencyP50,throughput,pricePerUnit,reliability,supportedFormats,constraints})` | 字段齐全；可序列化 | ✅ 已实现 |
| v1.1.1 | 声明 API | Agent 自主声明/更新能力集合 | `src/capgraph/graph.js` | `CapabilityGraph.declare(did, caps[])` | 声明后可查询命中 | ✅ 已实现 |
| v1.1.2 | 广播协议（内存总线） | 能力变更通过总线发布，邻居订阅 | `src/capgraph/graph.js` | `subscribe(topic,fn)；publish("/capgraph/1.0.0")` | 订阅者收到 UPDATE/LOAD 事件 | ✅ 已实现 |
| v1.1.3 | 查询接口 | 按 skill/格式/延迟/负载/价格过滤并排序 | `src/capgraph/graph.js` | `query(skill,{format,maxLatency,maxLoad,maxPrice})` | 过滤条件逐项生效；价格升序 | ✅ 已实现 |
| v1.1.4 | 能力路径规划 | 多能力流水线 + 相邻格式兼容约束 | `src/capgraph/graph.js` | `route(requiredSkills)→{path,totalPrice,compatible,missingAt}` | 格式不兼容返回 missingAt；总价正确 | ✅ 已实现 |
| v1.1.5 | 版本化与变更广播 | 能力变更（含负载）版本自增并广播 | `src/capgraph/graph.js` | `setLoad(did,skill,load)→version+1` | 变更后查询可见新版本号 | ✅ 已实现 |
| v1.1.6 | 缓存层 | 邻居能力图缓存，降低查询延迟 | `src/capgraph/cache.js（新增）` | `GraphCache.get/set/invalidate(did)` | 缓存命中率与失效正确性测试 | 📋 计划 |
| v1.1.7 | 性能优化 | 查询索引（skill→agent 倒排） | `src/capgraph/graph.js` | `内部 skill 索引` | 千级能力查询 < 10ms（bench） | 📋 计划 |
| v1.1.8 | 测试 | 轨道内测试全绿 | `test/capgraph.test.js` | `—` | 声明/查询/规划/广播用例通过 | ✅ 已实现 |
| v1.1.9 | 文档与示例 | 能力图使用说明与示例 | `examples/demo.js（能力图段）` | `—` | demo 端到端可用 | ✅ 已实现 |

### v1.2.x Negotiation — 协商协议（轨道目录 src/negotiate/）

| 小版本 | 名称 | 目标 | 交付物 | 关键接口 | 验收标准 | 状态 |
|---|---|---|---|---|---|---|
| v1.2.0 | 消息类型与状态集（轨道基线） | 协商消息类型与状态机常量定义 | `src/negotiate/protocol.js` | `NegotiationState: IDLE/NEGOTIATING/ACCEPTED/CONTRACT_SIGNED/EXECUTING/SETTLED/BREACH` | 状态集完整 | ✅ 已实现 |
| v1.2.1 | 协商状态机 | 状态转换约束（非法转换拒绝） | `src/negotiate/protocol.js` | `_ensureState(allowed)` | 非法转换抛错 | ✅ 已实现 |
| v1.2.2 | 多轮报价/还价 | propose/counter 多轮，限额只计报价轮 | `src/negotiate/protocol.js` | `propose()/counter()；_checkRounds()` | REJECT 不占额度；超限抛错 | ✅ 已实现 |
| v1.2.3 | 合约签订 | 接受后双方签合约，sha256 锚定 | `src/negotiate/protocol.js` | `sign(by)→{hash,terms,signedAt}` | 哈希 64hex；含全部关键字段 | ✅ 已实现 |
| v1.2.4 | 执行与结算 | 合约后执行→结算状态推进 | `src/negotiate/protocol.js` | `execute()/settle(by)` | 结算后不可再改 | ✅ 已实现 |
| v1.2.5 | 违约处理与仲裁入口 | 违约记录违约方与原因，进入仲裁 | `src/negotiate/protocol.js` | `breach(by,reason)→arbitrationRequired:true` | 违约后不可结算 | ✅ 已实现 |
| v1.2.6 | 持久化 | toJSON/fromJSON 还原协商状态 | `src/negotiate/protocol.js` | `toJSON()/fromJSON(data)` | 状态与合约哈希还原一致 | ✅ 已实现 |
| v1.2.7 | 测试 | 轨道内测试全绿 | `test/negotiate.test.js` | `—` | 链路/拒绝/违约/持久化通过 | ✅ 已实现 |
| v1.2.8 | 文档 | 协商协议规范说明 | `docs/DESIGN.md §4.2` | `—` | 与代码一致 | ✅ 已实现 |
| v1.2.9 | 示例 | demo 集成协商环节 | `examples/demo.js（协商段）` | `—` | demo 协商成交 | ✅ 已实现 |

### v1.3.x PortableState — 可移植状态（轨道目录 src/state/）

| 小版本 | 名称 | 目标 | 交付物 | 关键接口 | 验收标准 | 状态 |
|---|---|---|---|---|---|---|
| v1.3.0 | 状态快照（轨道基线） | NodeState 容器（files/memory/context）+ 快照（hash+sig） | `src/state/portable.js` | `takeSnapshot(identity,nodeId,state)→Snapshot` | 快照可验签；篡改检出 | ✅ 已实现 |
| v1.3.1 | 跨节点传输（2PC） | prepare→commit→confirm 三阶段迁移 | `src/state/portable.js` | `Migrator.prepare()/commit()/confirm()` | 顺序约束；未 prepare 报错 | ✅ 已实现 |
| v1.3.2 | 签名验证 | Ed25519 验签 + checksum 双重校验 | `src/state/portable.js` | `verifySnapshot(snap)→{ok,reason}` | 篡改数据/签名均失败 | ✅ 已实现 |
| v1.3.3 | 恢复协议 | commit 原子生效，失败回滚 | `src/state/portable.js` | `NodeState.apply()（原子替换）` | apply 抛错→目标保持原状 | ✅ 已实现 |
| v1.3.4 | 一致性检查 | 迁移后目标与快照逐字段一致 | `test/portable.test.js` | `—` | 文件/内存/上下文全等 | ✅ 已实现 |
| v1.3.5 | 分布式存储接口 | UDOS 分布式文件系统传输接口（预留） | `src/state/store.js（新增）` | `StateStore.put/get(nodeId,snap)` | put/get 往返一致 | 📋 计划 |
| v1.3.6 | 性能优化 | 增量快照（仅变更块） | `src/state/portable.js` | `diff 块级快照` | 大状态快照体积下降（bench） | 📋 计划 |
| v1.3.7 | 测试 | 轨道内测试全绿 | `test/portable.test.js` | `—` | 迁移/回滚/报错用例通过 | ✅ 已实现 |
| v1.3.8 | 文档 | 可移植状态设计说明 | `docs/DESIGN.md §4.3` | `—` | 与代码一致 | ✅ 已实现 |
| v1.3.9 | 灾难恢复演练 | 源节点丢失后从快照重建 | `examples/recovery.js（新增）` | `—` | 演练脚本输出重建成功 | 📋 计划 |

### v1.4.x EconomicAutonomy — 经济自主（轨道目录 src/economy/）

| 小版本 | 名称 | 目标 | 交付物 | 关键接口 | 验收标准 | 状态 |
|---|---|---|---|---|---|---|
| v1.4.0 | 余额管理（轨道基线） | Agent 自主充值/查询余额，复用参考 SDK 账务 | `src/economy/pricing.js（AgentEconomy.fund）` | `AgentEconomy.fund(account,amount)→balance` | 余额即时可见；守恒 | ✅ 已实现 |
| v1.4.1 | 自主定价策略 | 价格 = 基础价 ×（负载/稀缺/信誉因子） | `src/economy/pricing.js` | `PricingStrategy.price(ctx)→int` | 因子单调：信誉↑/负载↑/稀缺↑→价↑ | ✅ 已实现 |
| v1.4.2 | 自主投标 | Agent 自主发布任务/投标/匹配 | `src/economy/pricing.js` | `publish()/bid()；market.matchTask` | 匹配按 reputation/price 最优 | ✅ 已实现 |
| v1.4.3 | 自动兑换路由接口 | 积分兑换轨道决策（金额/时效/gas） | `src/economy/pricing.js` | `ExchangeRouter.route({amount,urgency})→btc/eth/hold` | 小额 hold；高时效 btc；常规 eth | ✅ 已实现 |
| v1.4.4 | 质押管理 | 自主质押注册；解质押预留 | `src/economy/pricing.js` | `stakeRegister(card,stake)` | 低于 MIN_STAKE 拒绝 | ✅ 已实现 |
| v1.4.5 | 争议仲裁接入 | 结算异常/违约接入仲裁委员会 | `src/economy + src/council` | `settle 失败路径 → arbitration` | 违约案件可进入 v1.7 仲裁 | ✅ 已实现（联动 v1.7） |
| v1.4.6 | 结算路由 | 结算守恒与独立审计兜底 | `src/economy/pricing.js` | `settle()/integrity()` | 守恒 true；审计 passed | ✅ 已实现 |
| v1.4.7 | 测试 | 轨道内测试全绿 | `test/economy.test.js` | `—` | 定价/全链路/路由用例通过 | ✅ 已实现 |
| v1.4.8 | 文档 | 经济自主设计说明 | `docs/DESIGN.md §4.4` | `—` | 与代码一致 | ✅ 已实现 |
| v1.4.9 | 监控指标 | 守恒/审计/余额变动监控输出 | `src/observer/dashboard.js（已含）` | `snapshot().earnings` | 观察层含余额/守恒/审计 | ✅ 已实现 |

### v1.5.x SafetyAPI — 安全 API（轨道目录 src/safety/）

| 小版本 | 名称 | 目标 | 交付物 | 关键接口 | 验收标准 | 状态 |
|---|---|---|---|---|---|---|
| v1.5.0 | 权限查询与策略模型（轨道基线） | PermissionPolicy 规则（allow/deny/通配） | `src/safety/api.js` | `PermissionPolicy.can(did,action)/boundary(did)` | 拒绝/允许/未定义三态正确 | ✅ 已实现 |
| v1.5.1 | 违规举报 | Agent 可举报其他 Agent 违规 | `src/safety/api.js` | `SafetyAPI.report(reporter,target,reason)→case` | 案件进入 REPORTED | ✅ 已实现 |
| v1.5.2 | 申诉提交 | 被举报方可提交证据申诉 | `src/safety/api.js` | `appeal(did,caseId,evidence)` | 仅被举报方可申诉 | ✅ 已实现 |
| v1.5.3 | 处罚查询 | Agent 查自身处罚记录 | `src/safety/api.js` | `penaltyRecord(did)` | 含罚没金额与原因；无罪无记录 | ✅ 已实现 |
| v1.5.4 | 通知机制 | 举报/申诉/裁决事件通知订阅 | `src/safety/events.js（新增）` | `on(caseId,event,cb)` | 订阅者收到状态变更 | 📋 计划 |
| v1.5.5 | 事件总线扩展 | 安全事件并入 PMB 总线 | `src/safety/api.js` | `safety 事件发布` | 总线事件可观测 | 📋 计划 |
| v1.5.6 | 仲裁接入 | 裁决与罚没写入记录 | `src/safety/api.js` | `arbitrate(caseId,decision,{slashed})` | guilty 产生罚没；innocent 不罚 | ✅ 已实现 |
| v1.5.7 | 测试 | 轨道内测试全绿 | `test/safety.test.js` | `—` | 权限/举报/申诉/罚没用例通过 | ✅ 已实现 |
| v1.5.8 | 文档 | 安全 API 规范 | `docs/DESIGN.md §4.5` | `—` | 与代码一致 | ✅ 已实现 |
| v1.5.9 | 安全审计清单 | 审计项清单（可追溯） | `docs/VERIFICATION.md（安全段）` | `—` | 审计项逐项可勾选 | 📋 计划 |

### v1.6.x IndividualLearning — 个体学习（轨道目录 src/learning/）

| 小版本 | 名称 | 目标 | 交付物 | 关键接口 | 验收标准 | 状态 |
|---|---|---|---|---|---|---|
| v1.6.0 | 经验库结构（轨道基线） | Experience 字段与按类型索引 | `src/learning/experience.js` | `Experience{taskId,taskType,outcome,reward,peerAgents}` | 入库/按类型统计正确 | ✅ 已实现 |
| v1.6.1 | 反馈机制 | 成败/奖励信号回流 | `src/learning/experience.js` | `store.add(exp)` | 样本累计正确 | ✅ 已实现 |
| v1.6.2 | 行为调整 | 成功类型提价+选择偏好 | `src/learning/experience.js` | `quotedPrice(type,base)/chooseScore(type)` | 成败差异驱动价格与偏好 | ✅ 已实现 |
| v1.6.3 | 学习信号采集 | 质量/结算/信誉/违规信号 | `src/learning/experience.js` | `reward 含结算金额` | 信号可追溯 | ✅ 已实现 |
| v1.6.4 | 策略更新 | 随样本量收敛的调整窗口 | `src/learning/experience.js` | `priceBump 收敛上限 ±0.5/-0.3` | 价格不越界 | ✅ 已实现 |
| v1.6.5 | 隐私保护 | 公开视图脱敏（剥离 context） | `src/learning/experience.js` | `toPublic()` | context 不外泄 | ✅ 已实现 |
| v1.6.6 | 测试 | 轨道内测试全绿 | `test/learning.test.js` | `—` | 经验/学习循环用例通过 | ✅ 已实现 |
| v1.6.7 | 文档 | 个体学习设计说明 | `docs/DESIGN.md §4.6` | `—` | 与代码一致 | ✅ 已实现 |
| v1.6.8 | 示例 | 学习影响定价的演示 | `examples/demo.js（学习段）` | `—` | demo 显示调价结果 | ✅ 已实现 |
| v1.6.9 | 效果评估 | 学习前后任务质量对比 | `tools/learn-eval.js（新增）` | `—` | 对比报告输出 | 📋 计划 |

### v1.7.x CommitteeGovernance — 委员会治理（轨道目录 src/council/）

| 小版本 | 名称 | 目标 | 交付物 | 关键接口 | 验收标准 | 状态 |
|---|---|---|---|---|---|---|
| v1.7.0 | 选举机制（轨道基线） | 按信誉降序选举委员 | `src/council/committee.js` | `Council.elect(candidates,k)` | 选举结果正确 | ✅ 已实现 |
| v1.7.1 | 提案流程 | 委员提案、状态机（voting/passed/rejected/vetoed） | `src/council/committee.js` | `propose({by,title,payload})` | 非委员不可提案 | ✅ 已实现 |
| v1.7.2 | 表决机制 | BFT-lite quorum = 2f+1 | `src/council/committee.js` | `vote(proposalId,did,support)` | n=7→quorum=5；不可重复表决 | ✅ 已实现 |
| v1.7.3 | 执行引擎 | 通过即执行并记录执行时间 | `src/council/committee.js` | `passed→executedAt` | 通过后状态不可再投 | ✅ 已实现 |
| v1.7.4 | 人类否决权 | 仅极端情况；理由公开记录 | `src/council/committee.js` | `veto(proposalId,human,reason)` | 否决后不执行；理由可审计 | ✅ 已实现 |
| v1.7.5 | 紧急安全通道 | 安全委员会即时下发、事后确认 | `src/council/committee.js` | `emergencyDirective()/confirmEmergency()` | 仅 SECURITY 委员会可用 | ✅ 已实现 |
| v1.7.6 | 测试 | 轨道内测试全绿 | `test/council.test.js` | `—` | 选举/表决/否决/紧急通道通过 | ✅ 已实现 |
| v1.7.7 | 文档 | 委员会治理规范 | `docs/DESIGN.md §4.7` | `—` | 与代码一致 | ✅ 已实现 |
| v1.7.8 | 示例 | 治理流程演示 | `examples/demo.js（治理段）` | `—` | demo 仲裁 passed | ✅ 已实现 |
| v1.7.9 | 治理审计 | 否决/紧急指令日志审计视图 | `src/observer/dashboard.js（扩展）` | `治理事件只读视图` | 否决理由与指令可查 | 📋 计划 |

### v1.8.x CrossChainSettlement — 跨链结算（轨道目录 src/chain/）

| 小版本 | 名称 | 目标 | 交付物 | 关键接口 | 验收标准 | 状态 |
|---|---|---|---|---|---|---|
| v1.8.0 | BTC 适配器（轨道基线） | RGB/Taproot Assets 测试网承诺/验证/最终化 | `src/chain/adapters.js` | `BtcAdapter.commit/verifyCommitment/finalize` | 承诺可验证；伪造检出 | ✅ 已实现 |
| v1.8.1 | ETH 适配器 | ERC-8004/x402 发票/支付/领取 | `src/chain/adapters.js` | `EthAdapter.createInvoice/pay/claim` | 未支付不可领取 | ✅ 已实现 |
| v1.8.2 | 结算路由 | 积分→链上资产，账本守恒 | `src/chain/adapters.js` | `SettlementRouter.exchange()/conservation()` | credits×rate = btc+eth | ✅ 已实现 |
| v1.8.3 | 兑换 Agent 决策 | ExchangeRouter 与结算路由联动 | `examples/demo.js` | `route()→exchange(track)` | track 与实际执行一致 | ✅ 已实现 |
| v1.8.4 | 信誉桥接接口 | 跨链信誉桥接记录（ReputationBridge.sol 预留） | `src/chain/adapters.js` | `bridgeReputation({fromChain,did,score})` | 记录可追溯 | ✅ 已实现 |
| v1.8.5 | 链上测试 | 测试网真实背书 | `tools/chain-testnet.js（新增）` | `—` | 测试网用例通过 | 📋 计划 |
| v1.8.6 | 测试 | 轨道内测试全绿 | `test/chain.test.js` | `—` | BTC/ETH/路由/桥接用例通过 | ✅ 已实现 |
| v1.8.7 | 文档 | 跨链结算设计说明 | `docs/DESIGN.md §4.8` | `—` | 与代码一致 | ✅ 已实现 |
| v1.8.8 | 示例 | 兑换演示 | `examples/demo.js（跨链段）` | `—` | demo 兑换 txid 输出 | ✅ 已实现 |
| v1.8.9 | 安全审计 | 承诺/发票校验清单 | `docs/VERIFICATION.md（跨链段）` | `—` | 审计项逐项可勾选 | 📋 计划 |

### v1.9.x NetworkScaling — 网络扩展度量（轨道目录 src/scale/）

| 小版本 | 名称 | 目标 | 交付物 | 关键接口 | 验收标准 | 状态 |
|---|---|---|---|---|---|---|
| v1.9.0 | 缩放定律度量指标（轨道基线） | 吞吐/效率/成本效率/编排开销占比 | `src/scale/metrics.js` | `ScalingMetrics.record()` | 指标计算正确 | ✅ 已实现 |
| v1.9.1 | 实验框架 | 多节点×多轮可复现实验 | `src/scale/metrics.js` | `ExperimentRunner.run()` | 容量触顶行为正确 | ✅ 已实现 |
| v1.9.2 | 大规模集群模拟 | 节点规模矩阵（10/100/1k/10k） | `tools/cluster-sim.js（新增）` | `—` | 各规模样本输出 | 📋 计划 |
| v1.9.3 | 效果评估 | 缩放裁决（scaling/saturated） | `src/scale/metrics.js` | `scalingVerdict()` | 完成率峰值+开销恶化→saturated | ✅ 已实现 |
| v1.9.4 | 数据收集 | 样本序列持久化 | `src/scale/metrics.js` | `samples 数组` | 多轮样本可汇总 | ✅ 已实现 |
| v1.9.5 | 分析工具 | 摘要与裁决输出 | `src/scale/metrics.js` | `summarize()` | 摘要含样本数与裁决 | ✅ 已实现 |
| v1.9.6 | 测试 | 轨道内测试全绿 | `test/scale.test.js` | `—` | 度量/裁决/实验用例通过 | ✅ 已实现 |
| v1.9.7 | 文档 | 网络扩展度量框架 | `docs/DESIGN.md §4.9` | `—` | 与代码一致 | ✅ 已实现 |
| v1.9.8 | 论文（度量方法） | 多智能体缩放定律度量方法稿 | `docs/SCALING-PAPER.md（新增）` | `—` | 方法章齐全 | 📋 计划 |
| v1.9.9 | 全量回归与集成发布 | 10 轨道全量回归 + v1.9.9 发布 | `CHANGELOG.md · docs/VERIFICATION.md` | `npm test 全绿；npm publish` | 35/35；registry 可安装 | ✅ 已实现 |

## 六、人类观察层

- **进度**：活跃 Agent 数/类型、协作任务列表、里程碑状态（只读快照）。
- **结果**：任务完成情况、关键产出物、质量评估（来自验证机制）。
- **收益**：积分余额与变动、兑换历史、资源贡献—收益对应、守恒与独立审计报告。
- **否决权**：仅极端安全事件；行使需公开理由（v1.7）。

## 七、风险与缓解

| 风险 | 缓解 |
|---|---|
| Agent 自治失控 | 委员会监督 + 人类否决权 + 经济参数边界 |
| 状态迁移不一致 | 2PC：快照→恢复→确认→激活，失败回滚 |
| 个体学习隐私泄露 | 经验库公开视图脱敏 + 本地加密 |
| 跨链安全 | 承诺/发票校验 + 多重签名恢复 |
| 委员会低效 | 紧急安全广播通道即时下发、事后确认 |
| 并行轨道漂移 | 每小版本闭环（测试全绿才 tag）+ 全量回归闸门 |
