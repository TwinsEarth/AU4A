// tools/gen-docs.js — AU4A 设计/开发文档生成器
// 10 中版本 = 10 条并行轨道；每轨道串行开发 9 个小版本（vX.1→vX.9）；
// v1.0.0 为全局基线（不计入），v1.1.0–v1.9.0 为各轨道基线小版本 → 99 个小版本。
// 运行：node tools/gen-docs.js（覆盖 docs/DESIGN.md 与 docs/DEV.md）
import { writeFileSync } from 'node:fs';

const M = [
  {
    ver: 'v1.0', code: 'Autonomy', name: '自治内核', dir: 'src/core/ + src/autonomy/ + src/observer/',
    baseline: null, // 全局基线 v1.0.0（不计入小版本）
    minors: [
      { n: 1, name: '自主身份与钱包', goal: 'Agent 自主生成 Ed25519 身份与 DID；钱包余额守恒', d: 'src/core/identity.js · src/core/wallet.js', i: 'Identity.generate(name)→{keypair,did,name}；Wallet.balance/deposit/conservation/audit', a: 'DID 全局唯一；余额永不为负；守恒恒真', s: '✅ 已实现' },
      { n: 2, name: '自主注册与质押准入', goal: 'Agent 自生成身份后自主注册，自质押准入', d: 'src/autonomy/registry.js', i: 'AgentRegistry.register({name,skills,stake,initial})→{ident,card,status}', a: '重复 DID 拒绝；质押不足拒绝；注册后余额正确', s: '✅ 已实现' },
      { n: 3, name: '人类观察层只读仪表盘', goal: '人类只读展示进度/结果/收益，无任何写方法', d: 'src/observer/dashboard.js', i: 'HumanObserver.snapshot()/summary()（只读）', a: '反射断言无写方法；一屏摘要覆盖三要素', s: '✅ 已实现' },
      { n: 4, name: '守恒与独立审计接入', goal: '观察层接入账本守恒与逐笔回放审计', d: 'src/observer/dashboard.js', i: 'snapshot().integrity={conservation,audit}', a: '守恒 true；独立审计 passed', s: '✅ 已实现' },
      { n: 5, name: '能力声明基础字段', goal: 'AgentCard 携带 skills/capabilities 基础声明', d: 'src/autonomy/registry.js（card）', i: 'AgentCard.new({did,name}) + skills/capabilities 数组', a: '观察层可读见 skills/capabilities', s: '✅ 已实现' },
      { n: 6, name: '注册状态机', goal: '注册后状态 active；质押跌破下限转 suspended', d: 'src/autonomy/registry.js', i: 'status: registered/active/suspended', a: '状态迁移有测试覆盖', s: '✅ 已实现（active；suspended 联动市场）' },
      { n: 7, name: '质押/解质押自主', goal: 'Agent 自主解质押不破坏守恒', d: 'economy 侧（v1.4 承接）', i: 'stakeRegister / 解质押校验', a: '解质押后余额守恒；低于 MIN_STAKE 拒解', s: '📋 计划（依赖 v1.4）' },
      { n: 8, name: '注册表迁移适配', goal: '与参考 SDK AgentMarket 结算语义完全兼容', d: 'src/core/wallet.js（market 封装）', i: 'deposit/registerAgent/publishTask/submitBid/settle 语义对齐', a: '与 @twinsearth/agent-universe 3.7.8 行为一致', s: '✅ 已实现' },
      { n: 9, name: '文档与基线测试', goal: 'v1.0 轨道收口：文档+测试基线', d: 'docs/VERIFICATION.md · test/autonomy.test.js', i: '—', a: 'npm test 全绿；验证日志记录', s: '✅ 已实现' },
    ],
  },
  {
    ver: 'v1.1', code: 'CapabilityGraph', name: '能力图', dir: 'src/capgraph/',
    baseline: { name: '能力数据结构与版本位', goal: 'Capability 数据结构（skill/延迟/吞吐/价格/可靠性/格式/约束）', d: 'src/capgraph/graph.js', i: 'Capability({skill,latencyP50,throughput,pricePerUnit,reliability,supportedFormats,constraints})', a: '字段齐全；可序列化', s: '✅ 已实现' },
    minors: [
      { n: 1, name: '声明 API', goal: 'Agent 自主声明/更新能力集合', d: 'src/capgraph/graph.js', i: 'CapabilityGraph.declare(did, caps[])', a: '声明后可查询命中', s: '✅ 已实现' },
      { n: 2, name: '广播协议（内存总线）', goal: '能力变更通过总线发布，邻居订阅', d: 'src/capgraph/graph.js', i: 'subscribe(topic,fn)；publish("/capgraph/1.0.0")', a: '订阅者收到 UPDATE/LOAD 事件', s: '✅ 已实现' },
      { n: 3, name: '查询接口', goal: '按 skill/格式/延迟/负载/价格过滤并排序', d: 'src/capgraph/graph.js', i: 'query(skill,{format,maxLatency,maxLoad,maxPrice})', a: '过滤条件逐项生效；价格升序', s: '✅ 已实现' },
      { n: 4, name: '能力路径规划', goal: '多能力流水线 + 相邻格式兼容约束', d: 'src/capgraph/graph.js', i: 'route(requiredSkills)→{path,totalPrice,compatible,missingAt}', a: '格式不兼容返回 missingAt；总价正确', s: '✅ 已实现' },
      { n: 5, name: '版本化与变更广播', goal: '能力变更（含负载）版本自增并广播', d: 'src/capgraph/graph.js', i: 'setLoad(did,skill,load)→version+1', a: '变更后查询可见新版本号', s: '✅ 已实现' },
      { n: 6, name: '缓存层', goal: '邻居能力图缓存，降低查询延迟', d: 'src/capgraph/cache.js（新增）', i: 'GraphCache.get/set/invalidate(did)', a: '缓存命中率与失效正确性测试', s: '📋 计划' },
      { n: 7, name: '性能优化', goal: '查询索引（skill→agent 倒排）', d: 'src/capgraph/graph.js', i: '内部 skill 索引', a: '千级能力查询 < 10ms（bench）', s: '📋 计划' },
      { n: 8, name: '测试', goal: '轨道内测试全绿', d: 'test/capgraph.test.js', i: '—', a: '声明/查询/规划/广播用例通过', s: '✅ 已实现' },
      { n: 9, name: '文档与示例', goal: '能力图使用说明与示例', d: 'examples/demo.js（能力图段）', i: '—', a: 'demo 端到端可用', s: '✅ 已实现' },
    ],
  },
  {
    ver: 'v1.2', code: 'Negotiation', name: '协商协议', dir: 'src/negotiate/',
    baseline: { name: '消息类型与状态集', goal: '协商消息类型与状态机常量定义', d: 'src/negotiate/protocol.js', i: 'NegotiationState: IDLE/NEGOTIATING/ACCEPTED/CONTRACT_SIGNED/EXECUTING/SETTLED/BREACH', a: '状态集完整', s: '✅ 已实现' },
    minors: [
      { n: 1, name: '协商状态机', goal: '状态转换约束（非法转换拒绝）', d: 'src/negotiate/protocol.js', i: '_ensureState(allowed)', a: '非法转换抛错', s: '✅ 已实现' },
      { n: 2, name: '多轮报价/还价', goal: 'propose/counter 多轮，限额只计报价轮', d: 'src/negotiate/protocol.js', i: 'propose()/counter()；_checkRounds()', a: 'REJECT 不占额度；超限抛错', s: '✅ 已实现' },
      { n: 3, name: '合约签订', goal: '接受后双方签合约，sha256 锚定', d: 'src/negotiate/protocol.js', i: 'sign(by)→{hash,terms,signedAt}', a: '哈希 64hex；含全部关键字段', s: '✅ 已实现' },
      { n: 4, name: '执行与结算', goal: '合约后执行→结算状态推进', d: 'src/negotiate/protocol.js', i: 'execute()/settle(by)', a: '结算后不可再改', s: '✅ 已实现' },
      { n: 5, name: '违约处理与仲裁入口', goal: '违约记录违约方与原因，进入仲裁', d: 'src/negotiate/protocol.js', i: 'breach(by,reason)→arbitrationRequired:true', a: '违约后不可结算', s: '✅ 已实现' },
      { n: 6, name: '持久化', goal: 'toJSON/fromJSON 还原协商状态', d: 'src/negotiate/protocol.js', i: 'toJSON()/fromJSON(data)', a: '状态与合约哈希还原一致', s: '✅ 已实现' },
      { n: 7, name: '测试', goal: '轨道内测试全绿', d: 'test/negotiate.test.js', i: '—', a: '链路/拒绝/违约/持久化通过', s: '✅ 已实现' },
      { n: 8, name: '文档', goal: '协商协议规范说明', d: 'docs/DESIGN.md §4.2', i: '—', a: '与代码一致', s: '✅ 已实现' },
      { n: 9, name: '示例', goal: 'demo 集成协商环节', d: 'examples/demo.js（协商段）', i: '—', a: 'demo 协商成交', s: '✅ 已实现' },
    ],
  },
  {
    ver: 'v1.3', code: 'PortableState', name: '可移植状态', dir: 'src/state/',
    baseline: { name: '状态快照', goal: 'NodeState 容器（files/memory/context）+ 快照（hash+sig）', d: 'src/state/portable.js', i: 'takeSnapshot(identity,nodeId,state)→Snapshot', a: '快照可验签；篡改检出', s: '✅ 已实现' },
    minors: [
      { n: 1, name: '跨节点传输（2PC）', goal: 'prepare→commit→confirm 三阶段迁移', d: 'src/state/portable.js', i: 'Migrator.prepare()/commit()/confirm()', a: '顺序约束；未 prepare 报错', s: '✅ 已实现' },
      { n: 2, name: '签名验证', goal: 'Ed25519 验签 + checksum 双重校验', d: 'src/state/portable.js', i: 'verifySnapshot(snap)→{ok,reason}', a: '篡改数据/签名均失败', s: '✅ 已实现' },
      { n: 3, name: '恢复协议', goal: 'commit 原子生效，失败回滚', d: 'src/state/portable.js', i: 'NodeState.apply()（原子替换）', a: 'apply 抛错→目标保持原状', s: '✅ 已实现' },
      { n: 4, name: '一致性检查', goal: '迁移后目标与快照逐字段一致', d: 'test/portable.test.js', i: '—', a: '文件/内存/上下文全等', s: '✅ 已实现' },
      { n: 5, name: '分布式存储接口', goal: 'UDOS 分布式文件系统传输接口（预留）', d: 'src/state/store.js（新增）', i: 'StateStore.put/get(nodeId,snap)', a: 'put/get 往返一致', s: '📋 计划' },
      { n: 6, name: '性能优化', goal: '增量快照（仅变更块）', d: 'src/state/portable.js', i: 'diff 块级快照', a: '大状态快照体积下降（bench）', s: '📋 计划' },
      { n: 7, name: '测试', goal: '轨道内测试全绿', d: 'test/portable.test.js', i: '—', a: '迁移/回滚/报错用例通过', s: '✅ 已实现' },
      { n: 8, name: '文档', goal: '可移植状态设计说明', d: 'docs/DESIGN.md §4.3', i: '—', a: '与代码一致', s: '✅ 已实现' },
      { n: 9, name: '灾难恢复演练', goal: '源节点丢失后从快照重建', d: 'examples/recovery.js（新增）', i: '—', a: '演练脚本输出重建成功', s: '📋 计划' },
    ],
  },
  {
    ver: 'v1.4', code: 'EconomicAutonomy', name: '经济自主', dir: 'src/economy/',
    baseline: { name: '余额管理', goal: 'Agent 自主充值/查询余额，复用参考 SDK 账务', d: 'src/economy/pricing.js（AgentEconomy.fund）', i: 'AgentEconomy.fund(account,amount)→balance', a: '余额即时可见；守恒', s: '✅ 已实现' },
    minors: [
      { n: 1, name: '自主定价策略', goal: '价格 = 基础价 ×（负载/稀缺/信誉因子）', d: 'src/economy/pricing.js', i: 'PricingStrategy.price(ctx)→int', a: '因子单调：信誉↑/负载↑/稀缺↑→价↑', s: '✅ 已实现' },
      { n: 2, name: '自主投标', goal: 'Agent 自主发布任务/投标/匹配', d: 'src/economy/pricing.js', i: 'publish()/bid()；market.matchTask', a: '匹配按 reputation/price 最优', s: '✅ 已实现' },
      { n: 3, name: '自动兑换路由接口', goal: '积分兑换轨道决策（金额/时效/gas）', d: 'src/economy/pricing.js', i: 'ExchangeRouter.route({amount,urgency})→btc/eth/hold', a: '小额 hold；高时效 btc；常规 eth', s: '✅ 已实现' },
      { n: 4, name: '质押管理', goal: '自主质押注册；解质押预留', d: 'src/economy/pricing.js', i: 'stakeRegister(card,stake)', a: '低于 MIN_STAKE 拒绝', s: '✅ 已实现' },
      { n: 5, name: '争议仲裁接入', goal: '结算异常/违约接入仲裁委员会', d: 'src/economy + src/council', i: 'settle 失败路径 → arbitration', a: '违约案件可进入 v1.7 仲裁', s: '✅ 已实现（联动 v1.7）' },
      { n: 6, name: '结算路由', goal: '结算守恒与独立审计兜底', d: 'src/economy/pricing.js', i: 'settle()/integrity()', a: '守恒 true；审计 passed', s: '✅ 已实现' },
      { n: 7, name: '测试', goal: '轨道内测试全绿', d: 'test/economy.test.js', i: '—', a: '定价/全链路/路由用例通过', s: '✅ 已实现' },
      { n: 8, name: '文档', goal: '经济自主设计说明', d: 'docs/DESIGN.md §4.4', i: '—', a: '与代码一致', s: '✅ 已实现' },
      { n: 9, name: '监控指标', goal: '守恒/审计/余额变动监控输出', d: 'src/observer/dashboard.js（已含）', i: 'snapshot().earnings', a: '观察层含余额/守恒/审计', s: '✅ 已实现' },
    ],
  },
  {
    ver: 'v1.5', code: 'SafetyAPI', name: '安全 API', dir: 'src/safety/',
    baseline: { name: '权限查询与策略模型', goal: 'PermissionPolicy 规则（allow/deny/通配）', d: 'src/safety/api.js', i: 'PermissionPolicy.can(did,action)/boundary(did)', a: '拒绝/允许/未定义三态正确', s: '✅ 已实现' },
    minors: [
      { n: 1, name: '违规举报', goal: 'Agent 可举报其他 Agent 违规', d: 'src/safety/api.js', i: 'SafetyAPI.report(reporter,target,reason)→case', a: '案件进入 REPORTED', s: '✅ 已实现' },
      { n: 2, name: '申诉提交', goal: '被举报方可提交证据申诉', d: 'src/safety/api.js', i: 'appeal(did,caseId,evidence)', a: '仅被举报方可申诉', s: '✅ 已实现' },
      { n: 3, name: '处罚查询', goal: 'Agent 查自身处罚记录', d: 'src/safety/api.js', i: 'penaltyRecord(did)', a: '含罚没金额与原因；无罪无记录', s: '✅ 已实现' },
      { n: 4, name: '通知机制', goal: '举报/申诉/裁决事件通知订阅', d: 'src/safety/events.js（新增）', i: 'on(caseId,event,cb)', a: '订阅者收到状态变更', s: '📋 计划' },
      { n: 5, name: '事件总线扩展', goal: '安全事件并入 PMB 总线', d: 'src/safety/api.js', i: 'safety 事件发布', a: '总线事件可观测', s: '📋 计划' },
      { n: 6, name: '仲裁接入', goal: '裁决与罚没写入记录', d: 'src/safety/api.js', i: 'arbitrate(caseId,decision,{slashed})', a: 'guilty 产生罚没；innocent 不罚', s: '✅ 已实现' },
      { n: 7, name: '测试', goal: '轨道内测试全绿', d: 'test/safety.test.js', i: '—', a: '权限/举报/申诉/罚没用例通过', s: '✅ 已实现' },
      { n: 8, name: '文档', goal: '安全 API 规范', d: 'docs/DESIGN.md §4.5', i: '—', a: '与代码一致', s: '✅ 已实现' },
      { n: 9, name: '安全审计清单', goal: '审计项清单（可追溯）', d: 'docs/VERIFICATION.md（安全段）', i: '—', a: '审计项逐项可勾选', s: '📋 计划' },
    ],
  },
  {
    ver: 'v1.6', code: 'IndividualLearning', name: '个体学习', dir: 'src/learning/',
    baseline: { name: '经验库结构', goal: 'Experience 字段与按类型索引', d: 'src/learning/experience.js', i: 'Experience{taskId,taskType,outcome,reward,peerAgents}', a: '入库/按类型统计正确', s: '✅ 已实现' },
    minors: [
      { n: 1, name: '反馈机制', goal: '成败/奖励信号回流', d: 'src/learning/experience.js', i: 'store.add(exp)', a: '样本累计正确', s: '✅ 已实现' },
      { n: 2, name: '行为调整', goal: '成功类型提价+选择偏好', d: 'src/learning/experience.js', i: 'quotedPrice(type,base)/chooseScore(type)', a: '成败差异驱动价格与偏好', s: '✅ 已实现' },
      { n: 3, name: '学习信号采集', goal: '质量/结算/信誉/违规信号', d: 'src/learning/experience.js', i: 'reward 含结算金额', a: '信号可追溯', s: '✅ 已实现' },
      { n: 4, name: '策略更新', goal: '随样本量收敛的调整窗口', d: 'src/learning/experience.js', i: 'priceBump 收敛上限 ±0.5/-0.3', a: '价格不越界', s: '✅ 已实现' },
      { n: 5, name: '隐私保护', goal: '公开视图脱敏（剥离 context）', d: 'src/learning/experience.js', i: 'toPublic()', a: 'context 不外泄', s: '✅ 已实现' },
      { n: 6, name: '测试', goal: '轨道内测试全绿', d: 'test/learning.test.js', i: '—', a: '经验/学习循环用例通过', s: '✅ 已实现' },
      { n: 7, name: '文档', goal: '个体学习设计说明', d: 'docs/DESIGN.md §4.6', i: '—', a: '与代码一致', s: '✅ 已实现' },
      { n: 8, name: '示例', goal: '学习影响定价的演示', d: 'examples/demo.js（学习段）', i: '—', a: 'demo 显示调价结果', s: '✅ 已实现' },
      { n: 9, name: '效果评估', goal: '学习前后任务质量对比', d: 'tools/learn-eval.js（新增）', i: '—', a: '对比报告输出', s: '📋 计划' },
    ],
  },
  {
    ver: 'v1.7', code: 'CommitteeGovernance', name: '委员会治理', dir: 'src/council/',
    baseline: { name: '选举机制', goal: '按信誉降序选举委员', d: 'src/council/committee.js', i: 'Council.elect(candidates,k)', a: '选举结果正确', s: '✅ 已实现' },
    minors: [
      { n: 1, name: '提案流程', goal: '委员提案、状态机（voting/passed/rejected/vetoed）', d: 'src/council/committee.js', i: 'propose({by,title,payload})', a: '非委员不可提案', s: '✅ 已实现' },
      { n: 2, name: '表决机制', goal: 'BFT-lite quorum = 2f+1', d: 'src/council/committee.js', i: 'vote(proposalId,did,support)', a: 'n=7→quorum=5；不可重复表决', s: '✅ 已实现' },
      { n: 3, name: '执行引擎', goal: '通过即执行并记录执行时间', d: 'src/council/committee.js', i: 'passed→executedAt', a: '通过后状态不可再投', s: '✅ 已实现' },
      { n: 4, name: '人类否决权', goal: '仅极端情况；理由公开记录', d: 'src/council/committee.js', i: 'veto(proposalId,human,reason)', a: '否决后不执行；理由可审计', s: '✅ 已实现' },
      { n: 5, name: '紧急安全通道', goal: '安全委员会即时下发、事后确认', d: 'src/council/committee.js', i: 'emergencyDirective()/confirmEmergency()', a: '仅 SECURITY 委员会可用', s: '✅ 已实现' },
      { n: 6, name: '测试', goal: '轨道内测试全绿', d: 'test/council.test.js', i: '—', a: '选举/表决/否决/紧急通道通过', s: '✅ 已实现' },
      { n: 7, name: '文档', goal: '委员会治理规范', d: 'docs/DESIGN.md §4.7', i: '—', a: '与代码一致', s: '✅ 已实现' },
      { n: 8, name: '示例', goal: '治理流程演示', d: 'examples/demo.js（治理段）', i: '—', a: 'demo 仲裁 passed', s: '✅ 已实现' },
      { n: 9, name: '治理审计', goal: '否决/紧急指令日志审计视图', d: 'src/observer/dashboard.js（扩展）', i: '治理事件只读视图', a: '否决理由与指令可查', s: '📋 计划' },
    ],
  },
  {
    ver: 'v1.8', code: 'CrossChainSettlement', name: '跨链结算', dir: 'src/chain/',
    baseline: { name: 'BTC 适配器', goal: 'RGB/Taproot Assets 测试网承诺/验证/最终化', d: 'src/chain/adapters.js', i: 'BtcAdapter.commit/verifyCommitment/finalize', a: '承诺可验证；伪造检出', s: '✅ 已实现' },
    minors: [
      { n: 1, name: 'ETH 适配器', goal: 'ERC-8004/x402 发票/支付/领取', d: 'src/chain/adapters.js', i: 'EthAdapter.createInvoice/pay/claim', a: '未支付不可领取', s: '✅ 已实现' },
      { n: 2, name: '结算路由', goal: '积分→链上资产，账本守恒', d: 'src/chain/adapters.js', i: 'SettlementRouter.exchange()/conservation()', a: 'credits×rate = btc+eth', s: '✅ 已实现' },
      { n: 3, name: '兑换 Agent 决策', goal: 'ExchangeRouter 与结算路由联动', d: 'examples/demo.js', i: 'route()→exchange(track)', a: 'track 与实际执行一致', s: '✅ 已实现' },
      { n: 4, name: '信誉桥接接口', goal: '跨链信誉桥接记录（ReputationBridge.sol 预留）', d: 'src/chain/adapters.js', i: 'bridgeReputation({fromChain,did,score})', a: '记录可追溯', s: '✅ 已实现' },
      { n: 5, name: '链上测试', goal: '测试网真实背书', d: 'tools/chain-testnet.js（新增）', i: '—', a: '测试网用例通过', s: '📋 计划' },
      { n: 6, name: '测试', goal: '轨道内测试全绿', d: 'test/chain.test.js', i: '—', a: 'BTC/ETH/路由/桥接用例通过', s: '✅ 已实现' },
      { n: 7, name: '文档', goal: '跨链结算设计说明', d: 'docs/DESIGN.md §4.8', i: '—', a: '与代码一致', s: '✅ 已实现' },
      { n: 8, name: '示例', goal: '兑换演示', d: 'examples/demo.js（跨链段）', i: '—', a: 'demo 兑换 txid 输出', s: '✅ 已实现' },
      { n: 9, name: '安全审计', goal: '承诺/发票校验清单', d: 'docs/VERIFICATION.md（跨链段）', i: '—', a: '审计项逐项可勾选', s: '📋 计划' },
    ],
  },
  {
    ver: 'v1.9', code: 'NetworkScaling', name: '网络扩展度量', dir: 'src/scale/',
    baseline: { name: '缩放定律度量指标', goal: '吞吐/效率/成本效率/编排开销占比', d: 'src/scale/metrics.js', i: 'ScalingMetrics.record()', a: '指标计算正确', s: '✅ 已实现' },
    minors: [
      { n: 1, name: '实验框架', goal: '多节点×多轮可复现实验', d: 'src/scale/metrics.js', i: 'ExperimentRunner.run()', a: '容量触顶行为正确', s: '✅ 已实现' },
      { n: 2, name: '大规模集群模拟', goal: '节点规模矩阵（10/100/1k/10k）', d: 'tools/cluster-sim.js（新增）', i: '—', a: '各规模样本输出', s: '📋 计划' },
      { n: 3, name: '效果评估', goal: '缩放裁决（scaling/saturated）', d: 'src/scale/metrics.js', i: 'scalingVerdict()', a: '完成率峰值+开销恶化→saturated', s: '✅ 已实现' },
      { n: 4, name: '数据收集', goal: '样本序列持久化', d: 'src/scale/metrics.js', i: 'samples 数组', a: '多轮样本可汇总', s: '✅ 已实现' },
      { n: 5, name: '分析工具', goal: '摘要与裁决输出', d: 'src/scale/metrics.js', i: 'summarize()', a: '摘要含样本数与裁决', s: '✅ 已实现' },
      { n: 6, name: '测试', goal: '轨道内测试全绿', d: 'test/scale.test.js', i: '—', a: '度量/裁决/实验用例通过', s: '✅ 已实现' },
      { n: 7, name: '文档', goal: '网络扩展度量框架', d: 'docs/DESIGN.md §4.9', i: '—', a: '与代码一致', s: '✅ 已实现' },
      { n: 8, name: '论文（度量方法）', goal: '多智能体缩放定律度量方法稿', d: 'docs/SCALING-PAPER.md（新增）', i: '—', a: '方法章齐全', s: '📋 计划' },
      { n: 9, name: '全量回归与集成发布', goal: '10 轨道全量回归 + v1.9.9 发布', d: 'CHANGELOG.md · docs/VERIFICATION.md', i: 'npm test 全绿；npm publish', a: '35/35；registry 可安装', s: '✅ 已实现' },
    ],
  },
];

// —— 计数断言：99 ——
const count = M.reduce((s, m) => s + (m.baseline ? 1 : 0) + m.minors.length, 0);
if (count !== 99) {
  console.error(`计数错误：${count} ≠ 99`); process.exit(1);
}
const serialCount = M.reduce((s, m) => s + m.minors.length, 0); // 90 串行 + 9 基线
console.log(`计数校验通过：99 小版本（90 串行开发 + 9 轨道基线；v1.0.0 全局基线不计入）`);

const verOf = (m, n) => `${m.ver}.${n}`;

// ============ DESIGN.md ============
let design = `# AU4A 设计文档（Agent Universe For Agent）

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

\`\`\`
┌──────────────────────────────────────────────┐
│  人类观察层（只读：进度·结果·收益·否决权）        │
├──────────────────────────────────────────────┤
│  Agent 自治层（身份/能力图/协商/经济/安全/学习）  │
├──────────────────────────────────────────────┤
│  Agent 委员会（资源/任务/仲裁/进化/安全，BFT）   │
├──────────────────────────────────────────────┤
│  结算与网络层（系统积分 + BTC/ETH 适配器 + 度量） │
└──────────────────────────────────────────────┘
\`\`\`

## 四、并行开发模型：10 中版本 = 10 条并行轨道

**并行开发**：10 个中版本是 10 条相互独立的开发轨道（模块零耦合、测试零耦合、可并行提交）。
**轨道内串行**：每条轨道串行开发 9 个小版本（vX.1 → vX.9），逐个小版本闭环：实现 → 测试全绿 → 提交 → tag。

| 轨道 | 中版本 | 代号 | 核心交付 | 串行小版本 | 轨道目录 |
|---|---|---|---|---|---|
${M.map((m) => `| ${m.ver}.x | ${m.ver} | ${m.code} | ${m.name} | v${m.ver.slice(1)}.1 → v${m.ver.slice(1)}.9 | ${m.dir} |`).join('\n')}

**99 个小版本计数口径**：
- 90 个串行开发小版本：10 轨道 × 9（vX.1–vX.9）；
- 9 个轨道基线小版本：v1.1.0–v1.9.0（各轨道版本位 .0，承载数据结构/协议基线）；
- v1.0.0 为全局基线（不计入小版本）；
- **合计 99**。

## 五、99 个小版本设计规格

> 状态：✅ 已实现（v1.0.1–v1.9.9 已落地，见 docs/VERIFICATION.md）｜ 📋 计划（下一迭代批次）

`;

for (const m of M) {
  design += `### ${m.ver}.x ${m.code} — ${m.name}（轨道目录 ${m.dir}）\n\n`;
  design += `| 小版本 | 名称 | 目标 | 交付物 | 关键接口 | 验收标准 | 状态 |\n|---|---|---|---|---|---|---|\n`;
  if (m.baseline) {
    design += `| ${m.ver}.0 | ${m.baseline.name}（轨道基线） | ${m.baseline.goal} | \`${m.baseline.d}\` | \`${m.baseline.i}\` | ${m.baseline.a} | ${m.baseline.s} |\n`;
  } else {
    design += `| ${m.ver}.0 | 全局基线（身份/钱包/注册表与参考 SDK 语义对齐） | 基础语义就绪 | 见 v1.0 轨道 | \`—\` | 语义兼容 | ✅ 已实现 |\n`;
  }
  for (const x of m.minors) {
    design += `| ${verOf(m, x.n)} | ${x.name} | ${x.goal} | \`${x.d}\` | \`${x.i}\` | ${x.a} | ${x.s} |\n`;
  }
  design += '\n';
}

design += `## 六、人类观察层

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
`;

// ============ DEV.md ============
let dev = `# AU4A 开发文档（Agent Universe For Agent）

> 版本：v1.0.1 → v1.9.9 ｜ 10 个中版本 + 99 个小版本 ｜ 真机实测 10 个中版本
> 开发模型：**10 条轨道并行开发（模块零耦合），轨道内串行开发 9 个小版本（vX.1→vX.9，逐版闭环）**

## 一、开发总纲

### 1.1 开发原则
1. **Agent 优先**：第一个测试用例是「Agent 能否自主使用它」。
2. **向后兼容**：结算语义与参考项目 SDK \`@twinsearth/agent-universe\` 完全一致（同 AgentMarket 账务语义）。
3. **轨道隔离**：10 条轨道之间零耦合（独立目录/独立测试），可并行开发与提交。
4. **可证伪**：每个小版本必须有可验证测试；文档能力代码可达。

### 1.2 环境与依赖
| 组件 | 版本 | 用途 |
|---|---|---|
| Node.js | ≥ 18（本机实测 22.23.2） | 运行时 |
| @twinsearth/agent-universe | 3.7.9（公共 npm） | 结算引擎 + 身份/网络基元（参考项目） |
| node:test | 内置 | 测试框架（零依赖） |
| git | ≥ 2.34 | 版本与 tag |

安装：\`npm install\`（自动安装参考 SDK 依赖）。

## 二、代码组织（10 条轨道）

\`\`\`
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
\`\`\`

## 三、并行开发协议

1. **轨道并行**：10 条轨道（v1.0–v1.9）互不依赖，各自独立开发、独立测试、独立 tag。
2. **轨道内串行**：每条轨道严格按 vX.1 → vX.2 → … → vX.9 串行推进；每个小版本闭环：
   \`实现 → node --test 全绿 → commit → tag vX.Y\` → 记录入 \`docs/VERIFICATION.md\`。
3. **集成闸门**：每条轨道 vX.9 完成后跑一次全量回归（10 轨道测试必须全绿），最后一轨 v1.9.9 执行端到端 demo + 发布。
4. **文档同步**：小版本推进时同步更新 DESIGN/DEV/CHANGELOG/VERIFICATION，禁止文档落后代码。

## 四、99 个小版本任务卡

> 验收命令：\`node --test\`（轨道测试 + 全量回归）。状态：✅ 已实现 ｜ 📋 计划。

`;

for (const m of M) {
  dev += `### ${m.ver}.x ${m.code} — ${m.name}\n\n`;
  dev += `| 小版本 | 任务 | 交付物 | 验证命令/方式 | 状态 |\n|---|---|---|---|---|\n`;
  if (m.baseline) {
    dev += `| ${m.ver}.0 | 基线：${m.baseline.name} | \`${m.baseline.d}\` | 轨道测试 | ${m.baseline.s} |\n`;
  }
  for (const x of m.minors) {
    dev += `| ${verOf(m, x.n)} | ${x.name}（${x.goal}） | \`${x.d}\` | ${x.s.includes('已实现') ? 'node --test 对应轨道文件全绿' : '待排期实现'} | ${x.s} |\n`;
  }
  dev += '\n';
}

dev += `## 五、部署验证矩阵

| 平台 | 验证内容 | 自动化 |
|---|---|---|
| Linux（本机 Cloud VM） | 完整功能 + node:test 全绿 + demo 运行 | ✅ \`npm test\` |
| macOS | 完整功能 + aarch64 | CI 待接 |
| Windows | 核心功能 | CI 待接 |
| 跨网络 | P2P 发现/GossipSub/DHT（参考 SDK 能力） | 手动+脚本 |
| 跨链 | BTC/ETH 适配器（测试网 mock） | 测试网 |

## 六、发布 SOP

1. **版本管理**：更新 package.json \`version\`、CHANGELOG、docs/VERIFICATION.md。
2. **查漏检验**：\`node --test\` 全部通过；无未使用依赖告警。
3. **补缺补齐**：README 对齐、示例对齐、文档同步。
4. **GitHub 发布**：commit → tag \`vX.Y.Z\` → push → GitHub Release（附 Release Notes + 验证摘要）。
5. **对外可安装检查**：干净目录 \`npm install @twinsearth/agent-universe-for-angent\` + 版本号断言。

## 七、验收标准（每中版本）

- 功能验收：轨道内 9 个小版本功能全部过测试；
- 兼容性验收：与参考 SDK 3.7.9 结算语义一致（守恒/审计逐笔通过）；
- 性能验收：守恒检查 O(1)、能力图查询 < 100ms（单机）、协商单轮 < 200ms；
- 安全验收：签名验证全部通过、观察层无写方法、余额永不为负；
- 文档验收：设计/开发文档与代码一致；
- 平台验收：本机 Linux 全绿（macOS/Windows 待 CI）。

## 八、实测记录

每个中版本/小版本的真机运行结果、测试计数、发现与修复写入 \`docs/VERIFICATION.md\`（即本仓库的「运行日志」）。
`;

writeFileSync(new URL('../docs/DESIGN.md', import.meta.url), design);
writeFileSync(new URL('../docs/DEV.md', import.meta.url), dev);
console.log('docs/DESIGN.md + docs/DEV.md 已生成（99 小版本规格齐全）');
