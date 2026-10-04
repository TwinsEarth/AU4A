# Changelog

## v1.9.9-batch（2026-10-04）— 计划项批次：99/99 小版本全部落地
- 15 个计划小版本全部实现：v1.0.7 / v1.1.6 / v1.1.7 / v1.3.5 / v1.3.6 / v1.3.9 / v1.5.4 / v1.5.5 / v1.5.9 / v1.6.9 / v1.7.9 / v1.8.5 / v1.8.9 / v1.9.2 / v1.9.8。
- 新模块：GraphCache（缓存层）、StateStore（分布式存储接口）、diffState/applyDiff（增量快照）、SafetyEvents（通知+总线）、学习效果评估（eval.js）、Council.auditLog（治理审计）。
- 新真机工具：`examples/recovery.js`（灾难恢复演练）、`tools/chain-testnet.js`（链上测试 12/12 PASS）、`tools/cluster-sim.js`（10k 节点矩阵）、`tools/learn-eval.js`（效果评估 CLI）。
- 新文档：`docs/SCALING-PAPER.md`（缩放定律度量方法稿）；VERIFICATION 增补 15 项实测与安全审计清单。
- 修复 2 个真实缺陷：v1.5.4 缺失 SafetyStatus 导入；v1.7.9 提案/紧急指令 ID 序号冲突（改独立计数器）。
- 单测累计 **51/51** 全绿；10 条轨道 99 个小版本 100% 已实现。

## v1.9.9（2026-10-04）— 集成发布版
- 10 个中版本全部实现并实测：Autonomy / CapabilityGraph / Negotiation / PortableState / EconomicAutonomy / SafetyAPI / IndividualLearning / CommitteeGovernance / CrossChainSettlement / NetworkScaling。
- 新增端到端演示 `examples/demo.js`：一条命令跑通「Agent 自治经济体」全链路。
- 单测 35/35 全绿；演示守恒/审计/结算/仲裁/兑换全部通过。

## v1.9.0 — Network Scaling
- ScalingMetrics：吞吐/效率/成本效率/编排开销占比；scalingVerdict（完成率峰值 + 峰值后开销恶化 → saturated）。
- ExperimentRunner：容量顶点后收益为负的可复现实验。

## v1.8.0 — Cross-Chain Settlement
- BtcAdapter（RGB 承诺/Taproot Assets 测试网）、EthAdapter（ERC-8004/x402）、SettlementRouter（账本守恒）、信誉桥接。

## v1.7.0 — Committee Governance
- 五大委员会类型；信誉选举；BFT-lite quorum 表决（2f+1）；人类否决权（公开理由）；安全紧急通道（事后确认）。

## v1.6.0 — Individual Learning
- ExperienceStore（脱敏公开视图）；LearningLoop：成败→定价/选择偏好调整。

## v1.5.0 — Safety API
- PermissionPolicy 权限边界查询；举报→申诉→仲裁→处罚记录；无罪不罚。

## v1.4.0 — Economic Autonomy
- PricingStrategy（信誉/负载/稀缺度）；AgentEconomy 自治经济体；ExchangeRouter 兑换路由接口。

## v1.3.0 — Portable State
- 快照 + sha256 + Ed25519 验签；2PC 迁移（prepare/commit/confirm/rollback），apply 失败回滚无部分状态。

## v1.2.0 — Negotiation
- 多轮协商状态机（IDLE→NEGOTIATING→ACCEPTED→CONTRACT_SIGNED→EXECUTING→SETTLED/BREACH）；合约 sha256；持久化。
- Bug 修复 3 个（toJSON 递归、propose 状态约束、轮次限额口径）。

## v1.1.0 — Capability Graph
- 能力声明/广播/查询（skill·格式·延迟·负载·价格）/能力路径规划/版本自增。

## v1.0.1 — Autonomy
- 底层逻辑重构起点：Agent 自主身份（Ed25519/DID）、自主钱包、自主注册（自质押 MIN_STAKE=100）；人类观察层只读（进度/结果/收益 + 守恒/审计）。
- 设计/开发文档（10 中版本 + 99 小版本路线图）、原则宣言、README、验证日志落地。
