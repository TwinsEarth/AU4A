# AU4A 开发文档（Agent Universe For Agent）

> 版本：v1.0.1 → v1.9.9 ｜ 10 个中版本 + 99 个小版本 ｜ 本机实测 10 个中版本收尾点
> 开发模型：**10 条轨道并行开发，轨道内 9–10 个小版本严格串行**。

## 一、开发总纲

1. **Agent 优先**：每个功能的第一个测试用例是「Agent 能否自主使用它」。
2. **向后兼容**：参考项目的账务语义（守恒、质押、罚没）与拒绝分类被继承并写成测试。
3. **轨道隔离**：10 条轨道零耦合，只共享冻结基元 `au4a-core` 与宿主内核 `au4a-kernel`；公共接口只增不改。
4. **可证伪**：每个小版本必须有可验证测试与证据等级；文档能力代码不可达即为缺陷。

## 二、环境与依赖

| 组件 | 版本 | 用途 |
|---|---|---|
| Rust | 1.88+（本机 1.98.1） | 全部 crate |
| cargo | 随 Rust | 构建与测试 |
| Node.js | ≥ 18（仅工具链） | 文档生成、版本物化、发布脚本；**不进入运行时** |

外部依赖仅 6 个：`serde`、`serde_json`、`sha2`、`ed25519-dalek`、`rand`、`hex`。轨道新增依赖必须经 Lead 批准。

## 三、代码组织

| 轨道 | crate | 职责 | 版本区间 |
|---|---|---|---|
| — | `au4a-core` | 冻结基元：自证 DID、规范 JSON、整数守恒账本、证据分级、类型化拒绝、PMB 信封、逻辑时钟 | 全系列 |
| 1.0 | `au4a-kernel` | Autonomy 自治内核 | `v1.0.1 → v1.0.10` |
| 1.1 | `au4a-capgraph` | Capability Graph 能力图 | `v1.1.1 → v1.1.10` |
| 1.2 | `au4a-negotiate` | Negotiation 协商协议 | `v1.2.1 → v1.2.10` |
| 1.3 | `au4a-state` | Portable State 可移植状态 | `v1.3.1 → v1.3.10` |
| 1.4 | `au4a-economy` | Economic Autonomy 经济自主 | `v1.4.1 → v1.4.10` |
| 1.5 | `au4a-safety` | Safety API 安全 API | `v1.5.1 → v1.5.10` |
| 1.6 | `au4a-learning` | Individual Learning 个体学习 | `v1.6.1 → v1.6.10` |
| 1.7 | `au4a-council` | Committee Governance 委员会治理 | `v1.7.1 → v1.7.10` |
| 1.8 | `au4a-chain` | Cross-Chain Settlement 跨链结算 | `v1.8.1 → v1.8.10` |
| 1.9 | `au4a-scale` | Network Scaling 网络扩展 | `v1.9.1 → v1.9.9` |
| — | `au4a-node` | 节点二进制：编排 10 条轨道、CLI、只读观察面板、部署验证 | 全系列 |

## 四、开发流程：每个小版本的六步闭环

1. **实现**：在轨道 crate 内新增模块，单一职责。
2. **测试**：正常路径 + 拒绝路径 + 不变式。
3. **通过**：`cargo test -p <crate>`，0 failed、0 warning。
4. **快照**：`node E:\DS\_forangent\snapshot.mjs <版本号> crates/<crate> docs/tracks/<track>.{md,json}`。
5. **文档**：更新该版本的元数据（目标/交付物/接口/验收/证据/状态）。
6. **下一版**：上一版未绿不得开下一版。

并行约定：每条轨道使用独立 `CARGO_TARGET_DIR`（`E:\DS\_forangent\target\<crate>`），
避免 10 条轨道争抢 cargo 锁；因此开发期**不跑 `cargo test --workspace`**，集成测试由 Lead 在汇总阶段统一执行。

## 五、99 个小版本逐项（开发视图）

### v1.0 Autonomy 自治内核（`au4a-kernel`，10 个小版本）

| 小版本 | 标题 | 目标 | 交付文件 | 测试命令 | 通过 | 等级 | 状态 |
|---|---|---|---|---|---|---|---|
| `v1.0.1` | 宿主内核重构 | 把宿主内核重构为可审计、可复现的宿主：注册序成为语义、能力有倒排索引、内核自己声称的不变式可被独立断言 | src/registry.rs<br>src/audit.rs<br>src/lib.rs<br>tests/host_kernel.rs | `cargo test -p au4a-kernel (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-kernel)` | 27/27 | verified | ✅ 已交付 |
| `v1.0.2` | Agent 自治层骨架 | 把「Agent 是主体」落成代码：纯函数决策器 + 真驱动内核的执行器，决策输入无人类通道、意图集合无「等待批准」 | src/autonomy.rs<br>src/lib.rs<br>tests/autonomy.rs | `cargo test -p au4a-kernel (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-kernel)` | 42/42 | verified | ✅ 已交付 |
| `v1.0.3` | 人类观察层骨架 | 给人类进度/结果/收益三个只读投影，并让「只读」成为可枚举、可编译期检查的结构事实（不存在 &mut Kernel，写能力没有类型） | src/observer.rs<br>src/lib.rs<br>tests/observer.rs | `cargo test -p au4a-kernel (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-kernel)` | 53/53 | verified | ✅ 已交付 |
| `v1.0.4` | Agent 委员会骨架 | 建立只由在册 Agent 组成的裁决机构：确定性抽签、法定人数与多数、具名失败模式，且隔离动议只对恶意 2 码开放（竞争绝不隔离） | src/council.rs<br>src/lib.rs<br>tests/council.rs | `cargo test -p au4a-kernel (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-kernel)` | 65/65 | verified | ✅ 已交付 |
| `v1.0.5` | 权限模型更新 | 内核能回答「这个 Agent 能做什么/不能做什么」：全部能力的恰好一次划分 + 类型化拒绝理由，权限来源只有客观事实、没有人类或运营方 | src/permission.rs<br>src/lib.rs<br>tests/permission.rs | `cargo test -p au4a-kernel (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-kernel)` | 79/79 | verified | ✅ 已交付 |
| `v1.0.6` | PMB协议扩展 | 在冻结线格式上扩展协议语义：消息类型分类、准入、收件人展开、重放保护与逻辑时钟新鲜度，拒绝全部类型化且分类不丢 | src/pmb.rs<br>src/lib.rs<br>tests/pmb.rs | `cargo test -p au4a-kernel (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-kernel)` | 92/92 | verified | ✅ 已交付 |
| `v1.0.7` | 状态机更新 | 把 Agent 生命周期变成白名单状态机 + 事件溯源，并把治理底线写进迁移表：竞争只降级（可恢复）、隔离只来自恶意证据或委员会决定、退役是终态 | src/lifecycle.rs<br>src/lib.rs<br>src/audit.rs<br>src/observer.rs<br>tests/lifecycle.rs | `cargo test -p au4a-kernel (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-kernel)` | 106/106 | verified | ✅ 已交付 |
| `v1.0.8` | 迁移适配器 | 把 v3.x 风格插件接口如实映射到 PMB 能力路由：能映射的映射、不能映射的具名拒绝、依赖人类审批的直接失败 | src/migration.rs<br>src/lib.rs<br>tests/migration.rs | `cargo test -p au4a-kernel (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-kernel)` | 120/120 | verified | ✅ 已交付 |
| `v1.0.9` | 测试框架 | 把「同样的种子给同样的结果」变成可复用工具：确定性仿真台 + 可重放日志 + 不变式套件（含无据隔离检测） | src/harness.rs<br>src/lib.rs<br>tests/harness.rs | `cargo test -p au4a-kernel (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-kernel)` | 130/130 | verified | ✅ 已交付 |
| `v1.0.10` | 文档 | 把文档变成机器可校验产物：类型化轨道清单 + 逐条断言（版本数/顺序/字段/证据/往返），文档声称的能力必须可达 | src/manifest.rs<br>src/lib.rs<br>tests/manifest.rs | `cargo test -p au4a-kernel (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-kernel)` | 139/139 | verified | ✅ 已交付 |

### v1.1 Capability Graph 能力图（`au4a-capgraph`，10 个小版本）

| 小版本 | 标题 | 目标 | 交付文件 | 测试命令 | 通过 | 等级 | 状态 |
|---|---|---|---|---|---|---|---|
| `v1.1.1` | 数据结构 | 给「一条能力」一个整数、可校验、可内容寻址的表示（skill/latency_p50/latency_p99/throughput/current_load/price/reliability/supported_formats/constraints） | src/capability.rs<br>src/lib.rs<br>tests/data_structures.rs | `cargo test -p au4a-capgraph` | 14/14 | verified | ✅ 已交付 |
| `v1.1.2` | 声明 API | Agent 用自签声明把自己的能力写进图，且任何人都不能替他人声明 | src/declaration.rs<br>src/graph.rs<br>tests/declaration.rs | `cargo test -p au4a-capgraph` | 35/35 | verified | ✅ 已交付 |
| `v1.1.3` | 广播协议 | 用 PMB Envelope + MsgKind 广播能力通告，真签名验签，篡改被拒 | src/broadcast.rs<br>src/lib.rs<br>tests/broadcast.rs | `cargo test -p au4a-capgraph` | 52/52 | verified | ✅ 已交付 |
| `v1.1.4` | 缓存层 | 邻居能力缓存：容量上限 + TTL 失效 + LRU 淘汰，全部确定性 | src/cache.rs<br>src/graph.rs<br>tests/cache.rs | `cargo test -p au4a-capgraph` | 69/69 | verified | ✅ 已交付 |
| `v1.1.5` | 查询接口 | 按技能/格式/价格/可靠度查询能力，走倒排索引而不是全图扫描 | src/index.rs<br>src/graph.rs<br>tests/query_index.rs | `cargo test -p au4a-capgraph` | 82/82 | verified | ✅ 已交付 |
| `v1.1.6` | 路径规划 | 真图搜索：给定技能序列与格式，返回最小代价流水线或明确的「无路径」 | src/planner.rs<br>src/graph.rs<br>tests/planner.rs | `cargo test -p au4a-capgraph (CARGO_INCREMENTAL=0)` | 101/101 | verified | ✅ 已交付 |
| `v1.1.7` | 版本化 | 能力变更自动递增版本，能识别陈旧通告与同版本异内容冲突 | src/version.rs<br>src/declaration.rs<br>src/graph.rs<br>tests/versioning.rs | `cargo test -p au4a-capgraph (CARGO_INCREMENTAL=0)` | 113/113 | verified | ✅ 已交付 |
| `v1.1.8` | 性能优化 | 增量索引维护 + 有界 top-k 选择 + 查询结果缓存，用操作计数而非墙钟证明 | src/perf.rs<br>src/graph.rs<br>src/index.rs<br>tests/perf.rs | `cargo test -p au4a-capgraph (CARGO_INCREMENTAL=0)` | 126/126 | verified | ✅ 已交付 |
| `v1.1.9` | 测试 | 端到端场景 + 不变式 + 对抗用例的完整测试面，并提供独立校验器 | src/planner.rs<br>tests/end_to_end.rs<br>tests/invariants.rs<br>tests/adversarial.rs<br>src/lib.rs | `cargo test -p au4a-capgraph (CARGO_INCREMENTAL=0)` | 151/151 | verified | ✅ 已交付 |
| `v1.1.10` | 文档与证据汇总 | 10 个版本的目标/接口/验收/证据齐备，results_json 汇总可被节点聚合 | src/lib.rs<br>tests/end_to_end.rs<br>docs/tracks/1.1.md<br>docs/tracks/1.1.json | `cargo test -p au4a-capgraph (CARGO_INCREMENTAL=0)` | 152/152 | verified | ✅ 已交付 |

### v1.2 Negotiation 协商协议（`au4a-negotiate`，10 个小版本）

| 小版本 | 标题 | 目标 | 交付文件 | 测试命令 | 通过 | 等级 | 状态 |
|---|---|---|---|---|---|---|---|
| `v1.2.1` | 消息类型 | 六种协商消息（REQUEST/COUNTER/ACCEPT/REJECT/SIGN/BREACH）成为先签名后发送的 PMB 信封 | src/msg.rs<br>src/lib.rs<br>tests/messages.rs | `cargo test -p au4a-negotiate` | 14/14 | cpu-proto | ✅ 已交付 |
| `v1.2.2` | 状态机 | IDLE→NEGOTIATING→ACCEPTED→CONTRACT_SIGNED→EXECUTING→SETTLED 与违约进入 ARBITRATION，每次转换双方签名 | src/state.rs | `cargo test -p au4a-negotiate` | 30/30 | cpu-proto | ✅ 已交付 |
| `v1.2.3` | 持久化 | 协商记录导出为规范 JSON 快照并可重放，重放后逐字节一致（纯内存，无文件 I/O） | src/journal.rs<br>src/state.rs<br>tests/journal.rs | `cargo test -p au4a-negotiate` | 40/40 | cpu-proto | ✅ 已交付 |
| `v1.2.4` | 多轮协商 | 多轮报价/还价引擎：轮数上限、REJECT 不占额度、超限走类型化拒绝 | src/rounds.rs<br>src/lib.rs<br>tests/rounds.rs | `cargo test -p au4a-negotiate` | 51/51 | cpu-proto | ✅ 已交付 |
| `v1.2.5` | 合约签订 | 合约哈希 = 规范 JSON 的 SHA-256，双方签名才成立，可锚定 | src/contract.rs | `cargo test -p au4a-negotiate` | 65/65 | cpu-proto | ✅ 已交付 |
| `v1.2.6` | 违约处理 | CONTRACT_BREACH 申诉：四类违约、证据等级、状态机进入 ARBITRATION | src/breach.rs<br>src/rounds.rs<br>tests/breach.rs | `cargo test -p au4a-negotiate` | 77/77 | cpu-proto | ✅ 已交付 |
| `v1.2.7` | 仲裁接入 | 仲裁案件立案→证据→裁决，罚没与赔付落到内核账本，守恒不变式保持 | src/arbitration.rs<br>src/rounds.rs<br>tests/arbitration.rs | `cargo test -p au4a-negotiate` | 93/93 | cpu-proto | ✅ 已交付 |
| `v1.2.8` | 测试 | 综合集成测试：正常/拒绝/不变式三条线覆盖全轨道能力 | tests/negotiate.rs<br>（无新公共 API，只加断言） | `cargo test -p au4a-negotiate` | 101/101 | cpu-proto | ✅ 已交付 |
| `v1.2.9` | 文档 | docs/tracks/1.2.md 与 1.2.json 覆盖 10 个小版本的目标/交付物/接口/验收/证据/状态 | docs/tracks/1.2.md<br>docs/tracks/1.2.json | `cargo test -p au4a-negotiate` | 101/101 | cpu-proto | ✅ 已交付 |
| `v1.2.10` | 示例 | scenario 真跑两条路径：多轮→双签→执行→结算；违约→仲裁，返回两份 JSON 摘要 | src/example.rs<br>src/lib.rs<br>tests/example.rs | `cargo test -p au4a-negotiate` | 111/111 | cpu-proto | ✅ 已交付 |

### v1.3 Portable State 可移植状态（`au4a-state`，10 个小版本）

| 小版本 | 标题 | 目标 | 交付文件 | 测试命令 | 通过 | 等级 | 状态 |
|---|---|---|---|---|---|---|---|
| `v1.3.1` | 状态快照 | 把 Agent 的三区（文件系统/内存/上下文）状态冻结成内容寻址的块集合，并落到可替换的状态存储上。 | src/snapshot.rs<br>src/store.rs<br>src/lib.rs<br>tests/v131_snapshot.rs | `cargo test -p au4a-state` | 31/31 | verified | ✅ 已交付 |
| `v1.3.2` | 跨节点传输 | 块级增量 diff（三区 set/del）+ 可切块、可续跑的双节点内存传输。 | src/diff.rs<br>src/transfer.rs<br>tests/v132_transfer.rs | `cargo test -p au4a-state` | 59/59 | cpu-proto | ✅ 已交付 |
| `v1.3.3` | 签名验证 | 快照内容寻址 + Ed25519 签名，篡改或替换必须被拒（签名有效 ≠ 是我要的那份）。 | src/signed.rs<br>tests/v133_signature.rs | `cargo test -p au4a-state` | 84/84 | verified | ✅ 已交付 |
| `v1.3.4` | 恢复协议 | 两阶段提交 prepare → commit → confirm，任意阶段失败回滚，目标节点不留部分状态。 | src/recovery.rs<br>src/recovery.rs<br>src/recovery.rs<br>tests/v134_two_phase.rs | `cargo test -p au4a-state` | 106/106 | verified | ✅ 已交付 |
| `v1.3.5` | 一致性检查 | 三区摘要 + 块级差异报告，定位恢复后的缺失/多余/被改块。 | src/integrity.rs<br>src/integrity.rs<br>tests/v135_consistency.rs | `cargo test -p au4a-state` | 123/123 | verified | ✅ 已交付 |
| `v1.3.6` | UDOS集成 | 与 E:\DS\UDOS 的数据契约（分布式存储语义，不引入依赖、不联网）。 | src/udos.rs<br>src/udos.rs<br>src/udos.rs<br>src/udos.rs<br>tests/v136_udos.rs | `cargo test -p au4a-state` | 148/148 | verified | ✅ 已交付 |
| `v1.3.7` | 性能优化 | 块级哈希复用 + 只物化变动块 + 增量续跑，用确定性计数器量化省下的工作量（不读墙钟）。 | src/perf.rs<br>src/perf.rs<br>src/snapshot.rs<br>tests/v137_perf.rs | `cargo test -p au4a-state` | 165/165 | verified | ✅ 已交付 |
| `v1.3.8` | 测试 | 全链路集成（run_chain）+ 篡改矩阵 + 故障矩阵 + 边界与拒绝路径。 | src/chain.rs<br>tests/v138_chain.rs | `cargo test -p au4a-state` | 186/186 | verified | ✅ 已交付 |
| `v1.3.9` | 文档 | 能力清单在代码里可枚举、可测试，文档每条声明都指向实现与测试。 | src/capabilities.rs<br>tests/v139_capabilities.rs<br>docs/tracks/1.3.md | `cargo test -p au4a-state` | 200/200 | verified | ✅ 已交付 |
| `v1.3.10` | 灾难恢复 | 备份 → 源丢失 → 2PC 重建 → 增量续跑 的完整演练。 | src/disaster.rs<br>src/disaster.rs<br>src/capabilities.rs<br>tests/v1310_disaster.rs | `cargo test -p au4a-state` | 215/215 | verified | ✅ 已交付 |

### v1.4 Economic Autonomy 经济自主（`au4a-economy`，10 个小版本）

| 小版本 | 标题 | 目标 | 交付文件 | 测试命令 | 通过 | 等级 | 状态 |
|---|---|---|---|---|---|---|---|
| `v1.4.1` | 余额管理 | Agent 自主申报并执行余额策略（运营底线 / 目标质押比例 / 单笔支出上限），拒绝路径不改动账本。 | src/balance.rs<br>src/lib.rs | `cargo test -p au4a-economy (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-economy)` | 10/10 | verified | ✅ 已交付 |
| `v1.4.2` | 定价策略 | Agent 用信誉分/稀缺度/负载的整数基点函数自主定价并广播，买方按整数单价发现并挑选最便宜供给方。 | src/pricing.rs<br>src/lib.rs | `cargo test -p au4a-economy (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-economy)` | 21/21 | verified | ✅ 已交付 |
| `v1.4.3` | 自动兑换 | 系统积分 ↔ BTC/ETH 兑换路由决策表（金额/时效/费用阈值）+ 账务预留；类型层封死「链上已成功」的表达，真实执行留待 v1.8。 | src/fx.rs<br>src/lib.rs | `cargo test -p au4a-economy (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-economy)` | 34/34 | verified | ✅ 已交付 |
| `v1.4.4` | 质押管理 | Agent 自主质押/解质押/冷静期解锁；罚没与质押簿同步；准入线与份额上限校验；不变式为「簿记承诺量 ≤ 账本锁定量」。 | src/stake.rs<br>src/lib.rs | `cargo test -p au4a-economy (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-economy)` | 45/45 | verified | ✅ 已交付 |
| `v1.4.5` | 争议仲裁 | Agent 自主立案/投票/申诉；罚没上限 = 锁定余额（并记录截断来源）；申诉重开投票但已销毁的罚没不可退回。 | src/arbitration.rs<br>src/lib.rs | `cargo test -p au4a-economy (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-economy)` | 56/56 | verified | ✅ 已交付 |
| `v1.4.6` | 结算路由 | 结算路由决策表（证据等级/cpu-proto 上限/收款方争议）+ 整数最大余数法分成；收益归属资源提供者，人类操作者只收收益不决策。 | src/settlement.rs<br>src/lib.rs | `cargo test -p au4a-economy (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-economy)` | 65/65 | verified | ✅ 已交付 |
| `v1.4.7` | 测试 | 跨模块性质测试与端到端测试：400 步随机操作序列下守恒/质押簿不变式、全域定价单调、分成严格分完、罚没上限、拒绝不改账本、场景逐字节可复现。 | tests/economy_invariants.rs<br>tests/e2e_scenario.rs<br>src/lib.rs | `cargo test -p au4a-economy (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-economy)` | 81/81 | verified | ✅ 已交付 |
| `v1.4.8` | 文档 | 把 7 个版本的实现固化为可审计文档：版本索引、不可协商规则、错误码映射、证据分级与明确的未做清单。 | crates/au4a-economy/README.md<br>docs/tracks/1.4.md<br>src/lib.rs | `cargo test -p au4a-economy (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-economy)` | 81/81 | verified | ✅ 已交付 |
| `v1.4.9` | 示例 | 可运行、确定性的「经济自主八步走」示例：用真实数字走完余额/定价/兑换/质押/仲裁/结算/不变量/自检，并打印场景 JSON。 | examples/economy_tour.rs<br>docs/tracks/1.4.md | `cargo test -p au4a-economy; cargo run -p au4a-economy --example economy_tour` | 81/81 | verified | ✅ 已交付 |
| `v1.4.10` | 监控 | 只读收益面板数据源：与节点 /api/revenue 字段对齐（minted/slashed/total/accounts.*），补上 earned/kind/display/conservation_ok；收益只算真实到账凭证。 | src/monitor.rs<br>src/lib.rs<br>examples/economy_tour.rs<br>README.md | `cargo test -p au4a-economy; cargo run -p au4a-economy --example economy_tour` | 87/87 | verified | ✅ 已交付 |

### v1.5 Safety API 安全 API（`au4a-safety`，10 个小版本）

| 小版本 | 标题 | 目标 | 交付文件 | 测试命令 | 通过 | 等级 | 状态 |
|---|---|---|---|---|---|---|---|
| `v1.5.1` | 权限查询 | Agent 能查询自己的权限边界，返回结构化的允许/禁止/需质押，而不是自由文本 | src/config.rs<br>src/permission.rs<br>src/setup.rs<br>src/lib.rs | `cargo test -p au4a-safety` | 14/14 | verified | ✅ 已交付 |
| `v1.5.2` | 违规举报 | Agent 举报他人违规，必须携带可验证证据哈希；未确认的举报不改变任何余额 | src/evidence.rs<br>src/chain.rs<br>src/case.rs<br>src/office.rs | `cargo test -p au4a-safety` | 44/44 | verified | ✅ 已交付 |
| `v1.5.3` | 申诉提交 | 被处罚方本人提交申诉证据，状态机进入 APPEALED，申诉本身不动账本 | src/appeal.rs<br>src/office.rs | `cargo test -p au4a-safety` | 56/56 | verified | ✅ 已交付 |
| `v1.5.4` | 处罚查询 | 查询处罚记录；处罚只能由仲裁者签名的数据契约触发，且罚没受锁定余额约束 | src/penalty.rs<br>src/office.rs | `cargo test -p au4a-safety` | 68/68 | verified | ✅ 已交付 |
| `v1.5.5` | 通知机制 | 按案件订阅状态变更（REPORTED/APPEALED/ARBITRATED），可退订，只投递给订阅者 | src/notify.rs<br>src/office.rs | `cargo test -p au4a-safety` | 83/83 | verified | ✅ 已交付 |
| `v1.5.6` | PMB扩展 | 安全 API 接入 PMB：SAFETY_QUERY / SAFETY_REPORT / SAFETY_APPEAL 三种消息类型，Envelope 真签名 | src/pmb.rs<br>src/office.rs | `cargo test -p au4a-safety` | 98/98 | verified | ✅ 已交付 |
| `v1.5.7` | 仲裁接入 | 申诉进入仲裁：用数据契约与 au4a-council 解耦，案件进入 ARBITRATED，误判可回滚 | src/arbitration.rs<br>src/office.rs | `cargo test -p au4a-safety` | 113/113 | verified | ✅ 已交付 |
| `v1.5.8` | 测试 | 端到端 + 对抗 + 确定性重放的独立测试套件 | tests/end_to_end.rs<br>tests/adversarial.rs<br>tests/determinism.rs | `cargo test -p au4a-safety` | 128/128 | verified | ✅ 已交付 |
| `v1.5.9` | 文档 | 机器可读契约导出与文档定稿（线格式、错误映射表、证据分级） | src/schema.rs<br>docs/tracks/1.5.md 与 1.5.json 定稿 | `cargo test -p au4a-safety` | 127/127 | verified | ✅ 已交付 |
| `v1.5.10` | 审计 | 审计报告：链完整性、状态==重放(事件链)、账本罚没与处罚记录交叉核对 | src/audit.rs<br>src/office.rs | `cargo test -p au4a-safety` | 137/137 | verified | ✅ 已交付 |

### v1.6 Individual Learning 个体学习（`au4a-learning`，10 个小版本）

| 小版本 | 标题 | 目标 | 交付文件 | 测试命令 | 通过 | 等级 | 状态 |
|---|---|---|---|---|---|---|---|
| `v1.6.1` | 经验库 | 让 Agent 把自己的经历写成可寻址、有界、可重放的数据结构：同语义只有一种字节表示，重复被精确识别，淘汰可见。 | src/experience.rs<br>src/rng.rs<br>src/scenario.rs<br>tests/experience_store.rs | `cargo test -p au4a-learning` | 15/15 | verified | ✅ 已交付 |
| `v1.6.2` | 反馈机制 | 把原始经验聚合成按任务类型与协作者维度的反馈统计（成功率/部分成功/失败、平均质量、平均收益、置信度），成功经验通过共享内核账本真实结算。 | src/feedback.rs<br>src/scenario.rs<br>tests/feedback.rs | `cargo test -p au4a-learning` | 25/25 | verified | ✅ 已交付 |
| `v1.6.3` | 行为调整 | 让学习真的改变行为：把反馈与信号变成定价/任务选择/协作对象选择三个策略参数的下一个取值，并用同种子的对照实验量化改善。 | src/policy.rs<br>src/violation.rs<br>src/sim.rs<br>src/scenario.rs<br>tests/policy.rs（9 个）、tests/market.rs（10 个） | `cargo test -p au4a-learning` | 44/44 | verified | ✅ 已交付 |
| `v1.6.4` | 学习信号 | 把四类信号（任务完成质量 / 结算金额 / 信誉变化 / 违规记录）折算成统一的整数信号向量，并让它们真的参与决策。 | src/signal.rs<br>src/policy.rs<br>src/sim.rs<br>src/scenario.rs<br>tests/signal.rs | `cargo test -p au4a-learning` | 52/52 | verified | ✅ 已交付 |
| `v1.6.5` | 模型更新 | 带学习率/动量/阻尼/遗忘/漂移钳制/代际回滚的整数模型更新；落地不可转让、有界的本地信誉台账。 | src/model.rs<br>src/sim.rs<br>src/lib.rs<br>tests/model.rs | `cargo test -p au4a-learning` | 63/63 | verified | ✅ 已交付 |
| `v1.6.6` | 隐私保护 | 本地明文经验库 / 本地加密视图 / 对外公开视图三条路径彻底分开：公开视图无上下文原文、无 peer DID、无精确金额，小样本聚合被抑制。 | src/privacy.rs<br>src/scenario.rs<br>tests/privacy.rs | `cargo test -p au4a-learning` | 71/71 | verified | ✅ 已交付 |
| `v1.6.7` | 测试 | 把不变式、可重放性与拒绝路径做成系统性测试套件（多种子扫描 + 逐字节复现 + 全部错误路径）。 | tests/invariants.rs<br>tests/reproducibility.rs<br>tests/refusals.rs | `cargo test -p au4a-learning` | 81/81 | verified | ✅ 已交付 |
| `v1.6.8` | 文档 | 让文档与代码不可能分叉：策略解释由与行为相同的函数产出，并发布『解释所蕴含的下一步动作』，测试逐项断言其与实际行为一致。 | src/explain.rs<br>src/policy.rs<br>tests/explain.rs<br>docs/tracks/1.6.md 与 1.6.json：10 个小版本全部成文 | `cargo test -p au4a-learning` | 86/86 | verified | ✅ 已交付 |
| `v1.6.9` | 示例 | 可运行示例：一个 Agent 跑完整学习循环（六阶段），打印可核对的 JSON 报告；示例不含逻辑，测试断言的就是示例打印的内容。 | src/demo.rs<br>examples/learning_loop.rs<br>tests/demo.rs | `cargo test -p au4a-learning` | 90/90 | verified | ✅ 已交付 |
| `v1.6.10` | 评估 | 固定种子的学习组 vs 对照组评估：池化成功率/收益提升 + 跨种子稳健性（中位数、最差种子、负向种子数）+ 消融实验分辨三个杠杆的贡献。 | src/eval.rs<br>src/sim.rs<br>tests/eval.rs | `cargo test -p au4a-learning` | 96/96 | verified | ✅ 已交付 |

### v1.7 Committee Governance 委员会治理（`au4a-council`，10 个小版本）

| 小版本 | 标题 | 目标 | 交付文件 | 测试命令 | 通过 | 等级 | 状态 |
|---|---|---|---|---|---|---|---|
| `v1.7.1` | 选举机制 | 五类委员会由高信誉+长期在线 Agent 选举产生，结果确定可复现，刷票无法当选 | src/committee.rs<br>src/election.rs<br>src/lib.rs<br>tests/election.rs | `cargo test -p au4a-council` | 19/19 | verified | ✅ 已交付 |
| `v1.7.2` | 提案流程 | 动议只能由 Agent 提出且内容由 Agent 私钥签名；人类观察者在类型层面无提案/修改能力 | src/proposal.rs<br>src/human.rs<br>src/lib.rs<br>tests/proposal.rs | `cargo test -p au4a-council` | 37/37 | verified | ✅ 已交付 |
| `v1.7.3` | 表决机制 | BFT-lite 法定人数表决（n≥3f+1、quorum=n-f），重复投票被拒，模棱两可双签作废整轮 | src/voting.rs<br>src/lib.rs<br>tests/voting.rs | `cargo test -p au4a-council` | 51/51 | verified | ✅ 已交付 |
| `v1.7.4` | 执行引擎 | 把已通过的决议落成真实状态变更（策略/信誉/账本），只能执行 passed、执行者须为在任委员、幂等且守恒 | src/execution.rs<br>src/lib.rs<br>tests/execution.rs | `cargo test -p au4a-council` | 61/61 | verified | ✅ 已交付 |
| `v1.7.5` | 否决权 | 人类唯一的写能力是否决：类型层面无 propose/edit/cast_vote/execute，只能阻断，理由必须公开 | src/veto.rs<br>src/human.rs<br>src/lib.rs<br>tests/veto.rs | `cargo test -p au4a-council` | 73/73 | verified | ✅ 已交付 |
| `v1.7.6` | 链上治理 | 把治理状态投影成 GovernorToken 语义数据结构（只做语义映射，不假装有链；真实链上执行属 v1.8） | src/ongov.rs<br>src/lib.rs<br>tests/ongov.rs | `cargo test -p au4a-council` | 82/82 | cpu-proto | ✅ 已交付 |
| `v1.7.7` | 测试 | 把「治理过程是否始终自洽」变成可执行可复现的证伪器：17 条不变式 + 确定性重放 | src/invariants.rs<br>src/lib.rs<br>tests/invariants.rs | `cargo test -p au4a-council` | 90/90 | verified | ✅ 已交付 |
| `v1.7.8` | 文档 | 能力清单机器可校验：文档里每条能力都必须指向源码里真实存在的测试，否则测试变红 | src/claims.rs<br>tests/claims.rs<br>src/lib.rs | `cargo test -p au4a-council` | 99/99 | verified | ✅ 已交付 |
| `v1.7.9` | 示例 | 可运行示例 governance_demo + 安全委员会紧急通道（即时生效、事后治理确认、否决/超期回滚）+ scenario 补齐双签作废 | src/emergency.rs<br>src/lib.rs<br>examples/governance_demo.rs<br>tests/emergency.rs<br>scenario | `cargo test -p au4a-council` | 106/106 | verified | ✅ 已交付 |
| `v1.7.10` | 审计 | 治理事件串成不可悄悄改写的哈希链，并可只读导出提案/否决/紧急指令/执行/策略快照 | src/audit.rs<br>src/lib.rs<br>src/invariants.rs<br>src/claims.rs<br>tests/audit.rs | `cargo test -p au4a-council` | 113/113 | verified | ✅ 已交付 |

### v1.8 Cross-Chain Settlement 跨链结算（`au4a-chain`，10 个小版本）

| 小版本 | 标题 | 目标 | 交付文件 | 测试命令 | 通过 | 等级 | 状态 |
|---|---|---|---|---|---|---|---|
| `v1.8.1` | RGB集成 | 把 RGB 客户端验证语义（创世/密封转移/验证/最终化）做成确定性测试网适配器，并落地双轨守恒与 fail-closed 对账。 | src/testnet.rs<br>src/bridge.rs<br>src/rgb.rs<br>src/lib.rs | `cargo test -p au4a-chain (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-chain)` | 25/25 | cpu-proto | ✅ 已交付 |
| `v1.8.2` | Taproot Assets | 资产锚定语义：Merkle 承诺 + 输出键派生 + 认证路径验证 + 最终性门槛；具名拒绝脚本路径花费/Schnorr 多签/增发/链下互换。 | src/taproot.rs<br>src/lib.rs | `cargo test -p au4a-chain (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-chain)` | 33/33 | cpu-proto | ✅ 已交付 |
| `v1.8.3` | ERC-8004 | ETH 侧身份/声誉/验证注册表语义：DID 绑定身份不可转让、整数基点反馈（自评不计入）、验证背书、只读信誉摘要。 | src/erc8004.rs<br>src/lib.rs | `cargo test -p au4a-chain (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-chain)` | 41/41 | cpu-proto | ✅ 已交付 |
| `v1.8.4` | x402 | 402 支付流程语义：开票-支付-最终性-领取；金额必须完全相等；领取走双轨托管解锁（fail-closed）。 | src/x402.rs<br>src/lib.rs | `cargo test -p au4a-chain (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-chain)` | 51/51 | cpu-proto | ✅ 已交付 |
| `v1.8.5` | 结算路由 | 按金额/时效/费用阈值在 internal/rgb/taproot/x402 之间选路；fail-closed 为决策表第一优先级；结算前中后各对账一次。 | src/routing.rs<br>src/lib.rs | `cargo test -p au4a-chain (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-chain)` | 63/63 | cpu-proto | ✅ 已交付 |
| `v1.8.6` | 信誉桥接 | 链上声誉事件 → 本地 4 维信誉（可靠性/质量/诚实/可用性）；不可转让、只认最终化事件、有界更新且不过冲。 | src/reputation.rs<br>src/lib.rs | `cargo test -p au4a-chain (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-chain)` | 71/71 | cpu-proto | ✅ 已交付 |
| `v1.8.7` | 测试 | 跨模块不变量与端到端测试：拒绝清单完备、收据恒 cpu-proto、双轨守恒随机序列、双花/重放/最终性回滚、场景逐字节复现。 | tests/chain_invariants.rs<br>tests/e2e_settlement.rs<br>src/testnet.rs<br>src/lib.rs | `cargo test -p au4a-chain (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-chain)` | 87/87 | cpu-proto | ✅ 已交付 |
| `v1.8.8` | 文档 | 汇总文档：模块地图、测试网语义表、双轨与 fail-closed、具名拒绝清单索引、证据分级与明确的未做清单。 | crates/au4a-chain/README.md<br>src/lib.rs<br>docs/tracks/1.8.md | `cargo test -p au4a-chain` | 87/87 | cpu-proto | ✅ 已交付 |
| `v1.8.9` | 示例 | 可运行的九步示例：测试网/双轨/RGB/Taproot/ERC-8004/x402/路由/信誉/自检，全部打印真实数字并输出场景 JSON。 | examples/chain_tour.rs<br>docs/tracks/1.8.md | `cargo test -p au4a-chain; cargo run -p au4a-chain --example chain_tour` | 87/87 | cpu-proto | ✅ 已交付 |
| `v1.8.10` | 安全审计 | 风险登记表（9 条，含未防护/部分防护的坦白：验证数据丢失、签名层、最终性概率、女巫刷分）+ 10 个真实攻击用例全部被拦住。 | src/audit.rs<br>src/lib.rs<br>examples/chain_tour.rs<br>README.md | `cargo test -p au4a-chain; cargo run -p au4a-chain --example chain_tour` | 92/92 | cpu-proto | ✅ 已交付 |

### v1.9 Network Scaling 网络扩展（`au4a-scale`，9 个小版本）

| 小版本 | 标题 | 目标 | 交付文件 | 测试命令 | 通过 | 等级 | 状态 |
|---|---|---|---|---|---|---|---|
| `v1.9.1` | 缩放定律度量 | 把节点数/交互复杂度/算力预算 ↔ 群体智能水平形式化为整数度量，数学表达=代码=数值测试 | src/metrics.rs<br>src/lib.rs | `cargo test -p au4a-scale` | 8/8 | verified | ✅ 已交付 |
| `v1.9.2` | 实验框架 | 确定性实验框架：给定节点档位与参数，产出可复现的实验报告与规范摘要 | src/harness.rs<br>src/lib.rs | `cargo test -p au4a-scale` | 14/14 | verified | ✅ 已交付 |
| `v1.9.3` | 大规模集群 | 10/100/1k/10k 节点的有界评估：秒级完成、内存有界（O(1) 分配，不随 N 增长） | src/cluster.rs<br>tests 中的墙钟上界断言（仅测试读时钟） | `cargo test -p au4a-scale` | 20/20 | verified | ✅ 已交付 |
| `v1.9.4` | 效果评估 | 容量顶点裁决：完成率峰值 + 峰值后编排开销恶化 + 边际收益转负（含「收益转负」测试） | src/verdict.rs | `cargo test -p au4a-scale` | 26/26 | verified | ✅ 已交付 |
| `v1.9.5` | 数据收集 | 确定性数据收集：把多个场景的观测记录成可复现的 bundle（内容寻址摘要） | src/collect.rs | `cargo test -p au4a-scale` | 31/31 | verified | ✅ 已交付 |
| `v1.9.6` | 分析工具 | 从观测数据反推模型参数（整数网格搜索）并给出残差报告 | src/analyze.rs | `cargo test -p au4a-scale` | 36/36 | verified | ✅ 已交付 |
| `v1.9.7` | 测试 | 端到端 + 确定性 + 性能上界 + 收益转负的独立测试套件 | tests/scale_end_to_end.rs | `cargo test -p au4a-scale` | 42/42 | verified | ✅ 已交付 |
| `v1.9.8` | 文档 | 机器可读契约（度量/单位/裁决码/证据分级）与文档定稿 | src/schema.rs<br>docs/tracks/1.9.md 与 1.9.json 定稿 | `cargo test -p au4a-scale` | 45/45 | verified | ✅ 已交付 |
| `v1.9.9` | 论文 | 论文稿：方法/指标/结果/局限，逐条标注模型推论 vs 本机实测 | src/paper.rs<br>docs/tracks/1.9.md 论文节（含局限与未做之事） | `cargo test -p au4a-scale` | 48/48 | verified | ✅ 已交付 |

## 六、测试与验证策略

| 层次 | 内容 |
|---|---|
| 单元测试 | 每个小版本的语义与边界（`crates/<crate>/src` 内 `#[cfg(test)]`） |
| 集成测试 | 跨模块流程（`crates/<crate>/tests/`） |
| 不变式测试 | 账本守恒、状态迁移幂等、事件链哈希连续 |
| 结构性测试 | 观察面只读（路由枚举断言）、否决权无提案能力 |
| 确定性测试 | 固定种子两次运行结果逐字节一致 |
| 跨 crate | `cargo test --workspace`（Lead 在集成阶段执行） |

## 七、部署验证矩阵（10 个中版本收尾点）

| 中版本 | 收尾版本 | 部署命令 | 实测命令 | 证据文件 |
|---|---|---|---|---|
| v1.0 Autonomy 自治内核 | `v1.0.10` | `cargo build --release -p au4a-node` | `au4a-node verify` + `au4a-node run --agents 8 --observe 127.0.0.1:8787` | `docs/verify/medium-v1.0.md` |
| v1.1 Capability Graph 能力图 | `v1.1.10` | `cargo build --release -p au4a-node` | `au4a-node verify` + `au4a-node run --agents 8 --observe 127.0.0.1:8787` | `docs/verify/medium-v1.1.md` |
| v1.2 Negotiation 协商协议 | `v1.2.10` | `cargo build --release -p au4a-node` | `au4a-node verify` + `au4a-node run --agents 8 --observe 127.0.0.1:8787` | `docs/verify/medium-v1.2.md` |
| v1.3 Portable State 可移植状态 | `v1.3.10` | `cargo build --release -p au4a-node` | `au4a-node verify` + `au4a-node run --agents 8 --observe 127.0.0.1:8787` | `docs/verify/medium-v1.3.md` |
| v1.4 Economic Autonomy 经济自主 | `v1.4.10` | `cargo build --release -p au4a-node` | `au4a-node verify` + `au4a-node run --agents 8 --observe 127.0.0.1:8787` | `docs/verify/medium-v1.4.md` |
| v1.5 Safety API 安全 API | `v1.5.10` | `cargo build --release -p au4a-node` | `au4a-node verify` + `au4a-node run --agents 8 --observe 127.0.0.1:8787` | `docs/verify/medium-v1.5.md` |
| v1.6 Individual Learning 个体学习 | `v1.6.10` | `cargo build --release -p au4a-node` | `au4a-node verify` + `au4a-node run --agents 8 --observe 127.0.0.1:8787` | `docs/verify/medium-v1.6.md` |
| v1.7 Committee Governance 委员会治理 | `v1.7.10` | `cargo build --release -p au4a-node` | `au4a-node verify` + `au4a-node run --agents 8 --observe 127.0.0.1:8787` | `docs/verify/medium-v1.7.md` |
| v1.8 Cross-Chain Settlement 跨链结算 | `v1.8.10` | `cargo build --release -p au4a-node` | `au4a-node verify` + `au4a-node run --agents 8 --observe 127.0.0.1:8787` | `docs/verify/medium-v1.8.md` |
| v1.9 Network Scaling 网络扩展 | `v1.9.9` | `cargo build --release -p au4a-node` | `au4a-node verify` + `au4a-node run --agents 8 --observe 127.0.0.1:8787` | `docs/verify/medium-v1.9.md` |

每个收尾点的实测必须包含：构建成功、`verify` 全绿退出码 0、只读面板三个 GET 端点命中、非 GET 返回 405、端到端编排输出可复现。

## 八、发布 SOP

1. `node tools/gen-docs.mjs`（完整性闸门：10 中版本 + 99 小版本）。
2. `cargo test --workspace` 全绿；`cargo clippy --workspace` 无 warning（尽力而为，见风险 5）。
3. `node tools/publish.mjs plan` 核对远端现状。
4. `node tools/publish.mjs bootstrap --yes`（首次：归档旧 master、清理旧 tag/release）。
5. `node tools/publish.mjs versions`（99 个 commit + 99 个 tag，可断点续跑）。
6. `node tools/publish.mjs releases --yes`（10 个中版本 Release）。
7. `node tools/publish.mjs status` 核验 99 个 tag 指向与 10 个 Release。

发布物与 tag 的关系：每个 tag 指向该版本提交，该提交的树由 `tools/materialize.mjs` 从逐版本快照重建，
即「测试通过时的那棵树」；`VERSION` 文件在发布时被戳成对应版本号（唯一的确定性变换）。

## 九、风险与缓解

| # | 风险 | 缓解 |
|---|---|---|
| 1 | 10 条轨道并行导致接口漂移 | 冻结基元 + 公共接口只增不改 + 契约文件 TEAM-BRIEF |
| 2 | 小版本被「只改文档」冒充 | 完整性闸门要求每版有交付物、验收与测试证据；快照逐版重建历史树 |
| 3 | 证据虚报 | 证据分级（verified / cpu-proto / unverified）+ 结算闸门：unverified 不可结算 |
| 4 | 集成分支冲突 | 轨道写作用域互斥；Lead 统一集成与 workspace 测试 |
| 5 | clippy/平台差异 | 以本机 Windows + Linux CI 双跑为准，未跑的平台标注为未验证 |

---

> 本文档由 `tools/gen-docs.mjs` 从 `docs/versions.json` 与 `docs/tracks/*.json` 生成；请勿手改。
