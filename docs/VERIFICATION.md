# AU4A 真机验证日志（VERIFICATION）

> 平台：Linux Cloud VM（Node v22.23.2 / npm 10.9.8 / git 2.34.1）
> 环境：`npm install` 安装参考 SDK @twinsearth/agent-universe@3.7.8（公共 npm，1s 完成）
> 命令：`node --test`（全量单测）/ `node examples/demo.js`（端到端演示）
> 原则：每版全绿（# fail 0）才 commit + 打 tag。

## v1.0.1 — Autonomy 自治内核（2026-10-04）
- 交付：`src/core/identity.js`（自主身份）、`src/core/wallet.js`（钱包+守恒+审计）、`src/autonomy/registry.js`（自主注册）、`src/observer/dashboard.js`（人类观察层只读）
- 测试：`node --test` → **5/5 通过**
  - Agent 自主注册（DID 自主生成、质押后余额正确）
  - 多 Agent 互不冲突
  - 质押不足拒绝
  - 人类观察层只读（进度/结果/收益/守恒/独立审计；无写方法）
  - Identity 签名/验签
- 已知问题：无。tag: v1.0.1

## v1.1.0 — Capability Graph 能力图
- 测试：+3（累计 **8/8** 通过）
  - 声明与查询：按 skill/格式/延迟/负载/价格过滤并按价格排序
  - 能力路径规划：多能力流水线 + 格式兼容（url→summarize 不兼容被拒）
  - 能力变更版本自增 + 总线广播（LOAD 事件）
- 已知问题：无。tag: v1.1.0

## v1.2.0 — Negotiation 协商协议
- 测试：+4（累计 **12/12** 通过）
  - 多轮协商 → 合约(sha256) → 执行 → 结算（完整链路）
  - 拒绝回到 IDLE；总报价轮 ≤ maxRounds×2，第 5 轮触发限额
  - 违约进入仲裁（不可再结算）
  - toJSON/fromJSON 持久化还原
- Bug 修复 3 个：
  1. `toJSON` 内 `JSON.stringify(this)` 触发 toJSON 钩子 → RangeError 死循环；改为显式纯对象序列化；
  2. `propose` 仅允许 IDLE 无法多轮再出价；放开为 IDLE/NEGOTIATING；
  3. 轮次限额把 REJECT 计入报价轮导致提前触发；改为只统计 PROPOSE/COUNTER，且 propose 同样校验。
- tag: v1.2.0

## v1.3.0 — Portable State 可移植状态
- 测试：+4（累计 **16/16** 通过）
  - 快照 → 验签 → 2PC 迁移：文件/内存/上下文完整恢复
  - 篡改快照 → checksum 校验失败
  - apply 失败 → rollback → 目标保持原状（无部分状态）
  - 未 prepare 直接 commit / 未 commit 直接 confirm 均报错
- 语义修正：迁移以快照为准（目标被完整替换，从中断点继续），非合并保留旧文件；修正断言后全绿。
- tag: v1.3.0

## v1.4.0 — Economic Autonomy 经济自主
- 测试：+3（累计 **19/19** 通过）
  - 自主定价：信誉↑/负载↑/稀缺↑ → 价格↑
  - 自治经济体：发布→投标→匹配→结算→守恒 + 独立审计（复用参考 SDK AgentMarket 结算引擎）
  - 兑换路由：小额聚合 hold / 高时效 BTC / 常规 ETH / gas 偏高转 BTC
- 已知问题：无。tag: v1.4.0

## v1.5.0 — Safety API 安全 API
- 测试：+3（累计 **22/22** 通过）
  - 权限边界查询：允许/拒绝/未定义动作
  - 举报→申诉→仲裁→处罚记录（含申诉身份校验）
  - 无罪裁决不产生处罚记录
- 已知问题：无。tag: v1.5.0

## v1.6.0 — Individual Learning 个体学习
- 测试：+2（累计 **24/24** 通过）
  - 经验库：入库/按类型统计/公开视图脱敏（context 不外泄）
  - 学习循环：连续成功类型提价 + 选择偏好高于连续失败类型
- 已知问题：无。tag: v1.6.0

## v1.7.0 — Committee Governance 委员会治理
- 测试：+4（累计 **28/28** 通过）
  - 信誉选举降序选委员
  - BFT-lite quorum 表决（n=7 → f=2 → quorum=5，通过即执行）
  - 人类否决权：阻止执行并公开理由
  - 安全委员会紧急通道：即时下发、事后确认；非安全委员会禁用
- 修复 1 个测试逻辑：第 5 票重复投票 → 改为 4 票后第 5 票触发 quorum。
- tag: v1.7.0

## v1.8.0 — Cross-Chain Settlement 跨链结算
- 测试：+3（累计 **31/31** 通过）
  - BTC 适配器：RGB 承诺 → 验证 → 最终化；伪造承诺校验失败
  - ETH 适配器：x402 发票 → 支付 → 领取（未支付不可领取）
  - 结算路由：积分→BTC/ETH 账本守恒 + 信誉桥接
- 说明：本版为测试网适配器（确定性 mock，守恒校验）；主网仅需替换背书函数。
- tag: v1.8.0

## v1.9.0 — Network Scaling 网络扩展度量
- 测试：+4（累计 **35/35** 通过）
  - 度量记录：吞吐/效率/成本效率/编排开销占比
  - 缩放裁决：容量顶点后开销恶化 → saturated（规模收益为负）
  - 实验运行器：节点数超过容量后完成量触顶、开销增长
  - 样本不足 → insufficient
- 修复：ExperimentRunner 缺内部度量实例；裁决从“成本效率峰值”改为“完成率峰值 + 峰值后开销恶化”。
- tag: v1.9.0

## v1.9.9 — 集成发布版（端到端演示）
- 单测：**35/35 通过**（10 个中版本全部模块）
- 端到端 `node examples/demo.js`：Agent 自主注册 → 能力图发现 → 权限自检 → 多轮协商 → 投标/匹配/完成/结算 → 个体学习调价 → 仲裁委员会 quorum 裁决 → 积分兑换 BTC（RGB 承诺）→ 缩放度量 → 人类观察层一屏展示
  - 守恒 true · 独立审计 true · 3/3 任务完成 · 兑换 txid 已生成 · 仲裁提案 passed（quorum=3）
- 修复 1 个演示逻辑：仲裁委员 3 人时 quorum=1 不具代表性 → 改 4 人（quorum=3）；兑换轨道 amount<100 时 hold 不应执行换汇 → 演示参数改为高时效大额（BTC）。
- tag: v1.9.9
