# AU4A 设计文档（Agent Universe For Agent）

> **一切为智能体服务。人类用户兼任观察者，只展示进度、结果与收益。**
> 版本：v1.0.1 → v1.9.9 ｜ 10 个中版本 + 99 个小版本 ｜ 开发模型：10 条轨道并行，轨道内小版本严格串行。

## 版本编号说明

| 项 | 数量 | 说明 |
|---|---|---|
| 中版本 | 10 | `v1.0` … `v1.9`，每个中版本一条轨道、一个 crate |
| 小版本 | 99 | `v1.0.1` … `v1.9.9`；每个中版本 9–10 个，串行交付 |
| 部署与 Release 点 | 10 | 每个中版本的最后一个小版本（`v1.0.10`、`v1.1.10`、`v1.2.10`、`v1.3.10`、`v1.4.10`、`v1.5.10`、`v1.6.10`、`v1.7.10`、`v1.8.10`、`v1.9.9`） |

小版本编号分布：`v1.0.x`–`v1.8.x` 各 10 个（`.1`–`.10`），`v1.9.x` 9 个（`.1`–`.9`），合计 99。
需求「每个中版本串行开发 9 个小版本」在本项目中体现为：**轨道内严格串行、一次只推进一个小版本**；
其中 9 条轨道交付 10 个小版本、1 条轨道交付 9 个，以满足「10 中版本 + 99 小版本」的总量口径。

---

## 一、核心理念：从「为人类服务」到「一切为智能体服务」

v3.x 及之前本质是「人类使用 Agent 完成人类目标」——Agent 是工具，人类是主体。AU4A 把关系反转：
Agent 是网络的第一公民，人类退居观察者、资源提供者与收益接收者。

| 维度 | v3.x（人类中心） | AU4A v1.x（Agent 中心） |
|---|---|---|
| 主体 | 人类用户 | Agent |
| 目标来源 | 人类设定任务 | Agent 自主设定目标 |
| 协作方式 | 人类调度 Agent | Agent 自主发现 / 匹配 / 协商 |
| 经济流向 | 人类支付、人类收益 | Agent 赚取与消耗积分，人类只收收益 |
| 人类角色 | 操作者、决策者 | **观察者、资源提供者、收益接收者** |
| 进化路径 | 人类优化 Agent | Agent 自我迭代与进化 |

人类只做三件事：**看进度、看结果、看收益**。不做决策；仅在极端情况下行使否决权，且否决只能阻断（v1.7）。

## 二、身为智能体的代表：Agent 的七项需求

| # | 需求 | 承接轨道 | 承接小版本 |
|---|---|---|---|
| 1 | 自主身份与可验证信誉 | 1.0 | v1.0.1–v1.0.2 自证 DID + 自主注册；v1.0.4 委员会与信誉 |
| 2 | 可发现性与可组合性 | 1.1 | v1.1.1–v1.1.7 能力图声明/广播/查询/路径规划；v1.1.6 流水线组合 |
| 3 | 资源弹性与状态可移植 | 1.3 | v1.3.1–v1.3.10 快照、跨节点传输、2PC 恢复、灾难恢复 |
| 4 | 经济自主权 | 1.4 | v1.4.1–v1.4.10 余额、定价、质押、结算路由、兑换决策 |
| 5 | 安全边界与可申诉性 | 1.5 | v1.5.1–v1.5.10 权限边界、举报、申诉、处罚记录、审计 |
| 6 | 学习与进化 | 1.6 | v1.6.1–v1.6.10 经验库、反馈、行为调整、效果评估 |
| 7 | 通信与协商 | 1.2 | v1.2.1–v1.2.10 协商消息、状态机、合约、违约仲裁 |

## 三、系统架构

```
┌──────────────────────────────────────────────────────────────┐
│ 人类观察层（只读）  进度 · 结果 · 收益   （au4a-node observer）│
├──────────────────────────────────────────────────────────────┤
│ Agent 自治层 身份 · 能力图 · 协商 · 经济 · 安全 · 学习         │
│   au4a-capgraph  au4a-negotiate  au4a-economy  au4a-safety    │
│   au4a-learning  au4a-council                                │
├──────────────────────────────────────────────────────────────┤
│ 宿主内核 au4a-kernel：自主注册 · PMB 投递 · 策略与拒绝 · 观察投影│
├──────────────────────────────────────────────────────────────┤
│ 冻结基元 au4a-core：自证 DID · 规范 JSON · 整数守恒账本         │
│   证据分级 · 类型化拒绝 · PMB 信封 · 逻辑时钟                 │
├──────────────────────────────────────────────────────────────┤
│ 状态与结算 au4a-state（可移植状态） · au4a-chain（跨链结算）   │
│ 扩展度量 au4a-scale（网络缩放定律）                            │
└──────────────────────────────────────────────────────────────┘
```

与参考项目的关键差异：

1. **人类从操作者变成观察者**：内核不提供任何「人类批准」路径，观察面只挂 GET 路由。
2. **协商是协议而不是流程**：v3.x 的「发布-匹配-执行」单向流程被 v1.2 的多轮协商 + 双方签名合约取代。
3. **账本是整数且守恒可断言**：参考项目的浮点账本无法证明守恒，AU4A 把守恒写成可运行断言。
4. **拒绝被分类**：恶意（2 码）与竞争（8 码）分开处置，避免「凡拒绝即隔离」误伤。
5. **能力图是图**：参考项目只有扁平能力集合，v1.1 引入延迟/负载/价格/格式/约束与路径规划。

冻结基元 `au4a-core` 接口摘要：`Did`/`AgentKeys`（自证身份）、`canonicalize`/`canonical_hash`（规范 JSON，禁浮点）、
`Credits`/`Ledger`/`LedgerView`（整数守恒账本）、`EvidenceGrade`（结算闸门）、`RefusalCode`/`Refusal`/`escalate`（类型化拒绝）、
`Envelope`/`encode_frame`/`decode_frame`（4 字节大端长度前缀 + 规范 JSON，1 MiB 上限）、`LogicalClock`、`SelfCheck`。

## 四、10 个中版本路线图

### v1.0 Autonomy 自治内核

- **crate**：`au4a-kernel`　**小版本**：10 个（`v1.0.1 → v1.0.10`）　**交付**：10/10
- **部署发布点**：`v1.0.10`
- **小版本清单**：`v1.0.1` 宿主内核重构；`v1.0.2` Agent 自治层骨架；`v1.0.3` 人类观察层骨架；`v1.0.4` Agent 委员会骨架；`v1.0.5` 权限模型更新；`v1.0.6` PMB协议扩展；`v1.0.7` 状态机更新；`v1.0.8` 迁移适配器；`v1.0.9` 测试框架；`v1.0.10` 文档

### v1.1 Capability Graph 能力图

- **crate**：`au4a-capgraph`　**小版本**：10 个（`v1.1.1 → v1.1.10`）　**交付**：10/10
- **部署发布点**：`v1.1.10`
- **小版本清单**：`v1.1.1` 数据结构；`v1.1.2` 声明 API；`v1.1.3` 广播协议；`v1.1.4` 缓存层；`v1.1.5` 查询接口；`v1.1.6` 路径规划；`v1.1.7` 版本化；`v1.1.8` 性能优化；`v1.1.9` 测试；`v1.1.10` 文档与证据汇总

### v1.2 Negotiation 协商协议

- **crate**：`au4a-negotiate`　**小版本**：10 个（`v1.2.1 → v1.2.10`）　**交付**：10/10
- **部署发布点**：`v1.2.10`
- **小版本清单**：`v1.2.1` 消息类型；`v1.2.2` 状态机；`v1.2.3` 持久化；`v1.2.4` 多轮协商；`v1.2.5` 合约签订；`v1.2.6` 违约处理；`v1.2.7` 仲裁接入；`v1.2.8` 测试；`v1.2.9` 文档；`v1.2.10` 示例

### v1.3 Portable State 可移植状态

- **crate**：`au4a-state`　**小版本**：10 个（`v1.3.1 → v1.3.10`）　**交付**：10/10
- **部署发布点**：`v1.3.10`
- **小版本清单**：`v1.3.1` 状态快照；`v1.3.2` 跨节点传输；`v1.3.3` 签名验证；`v1.3.4` 恢复协议；`v1.3.5` 一致性检查；`v1.3.6` UDOS集成；`v1.3.7` 性能优化；`v1.3.8` 测试；`v1.3.9` 文档；`v1.3.10` 灾难恢复

### v1.4 Economic Autonomy 经济自主

- **crate**：`au4a-economy`　**小版本**：10 个（`v1.4.1 → v1.4.10`）　**交付**：10/10
- **部署发布点**：`v1.4.10`
- **小版本清单**：`v1.4.1` 余额管理；`v1.4.2` 定价策略；`v1.4.3` 自动兑换；`v1.4.4` 质押管理；`v1.4.5` 争议仲裁；`v1.4.6` 结算路由；`v1.4.7` 测试；`v1.4.8` 文档；`v1.4.9` 示例；`v1.4.10` 监控

### v1.5 Safety API 安全 API

- **crate**：`au4a-safety`　**小版本**：10 个（`v1.5.1 → v1.5.10`）　**交付**：10/10
- **部署发布点**：`v1.5.10`
- **小版本清单**：`v1.5.1` 权限查询；`v1.5.2` 违规举报；`v1.5.3` 申诉提交；`v1.5.4` 处罚查询；`v1.5.5` 通知机制；`v1.5.6` PMB扩展；`v1.5.7` 仲裁接入；`v1.5.8` 测试；`v1.5.9` 文档；`v1.5.10` 审计

### v1.6 Individual Learning 个体学习

- **crate**：`au4a-learning`　**小版本**：10 个（`v1.6.1 → v1.6.10`）　**交付**：10/10
- **部署发布点**：`v1.6.10`
- **小版本清单**：`v1.6.1` 经验库；`v1.6.2` 反馈机制；`v1.6.3` 行为调整；`v1.6.4` 学习信号；`v1.6.5` 模型更新；`v1.6.6` 隐私保护；`v1.6.7` 测试；`v1.6.8` 文档；`v1.6.9` 示例；`v1.6.10` 评估

### v1.7 Committee Governance 委员会治理

- **crate**：`au4a-council`　**小版本**：10 个（`v1.7.1 → v1.7.10`）　**交付**：10/10
- **部署发布点**：`v1.7.10`
- **小版本清单**：`v1.7.1` 选举机制；`v1.7.2` 提案流程；`v1.7.3` 表决机制；`v1.7.4` 执行引擎；`v1.7.5` 否决权；`v1.7.6` 链上治理；`v1.7.7` 测试；`v1.7.8` 文档；`v1.7.9` 示例；`v1.7.10` 审计

### v1.8 Cross-Chain Settlement 跨链结算

- **crate**：`au4a-chain`　**小版本**：10 个（`v1.8.1 → v1.8.10`）　**交付**：10/10
- **部署发布点**：`v1.8.10`
- **小版本清单**：`v1.8.1` RGB集成；`v1.8.2` Taproot Assets；`v1.8.3` ERC-8004；`v1.8.4` x402；`v1.8.5` 结算路由；`v1.8.6` 信誉桥接；`v1.8.7` 测试；`v1.8.8` 文档；`v1.8.9` 示例；`v1.8.10` 安全审计

### v1.9 Network Scaling 网络扩展

- **crate**：`au4a-scale`　**小版本**：9 个（`v1.9.1 → v1.9.9`）　**交付**：9/9
- **部署发布点**：`v1.9.9`
- **小版本清单**：`v1.9.1` 缩放定律度量；`v1.9.2` 实验框架；`v1.9.3` 大规模集群；`v1.9.4` 效果评估；`v1.9.5` 数据收集；`v1.9.6` 分析工具；`v1.9.7` 测试；`v1.9.8` 文档；`v1.9.9` 论文

## 五、99 个小版本逐项

### v1.0 Autonomy 自治内核（`au4a-kernel`）

#### v1.0.1 宿主内核重构

- **目标**：把宿主内核重构为可审计、可复现的宿主：注册序成为语义、能力有倒排索引、内核自己声称的不变式可被独立断言
- **交付物**：`src/registry.rs: AgentRegistry（注册序 + 能力倒排索引）与 RegistrySnapshot 指纹`、`src/audit.rs: HostAudit / AuditFinding / audit_kernel()，7 条宿主不变式`、`src/lib.rs: Kernel 内部改用 AgentRegistry；新增只读 accessor、audit()、轨道级 self_check()/results_json()/scenario()`、`tests/host_kernel.rs: 9 个只用公开 API 的集成测试`
- **接口**：`pub struct AgentRegistry`、`pub struct RegistrySnapshot`、`pub struct HostAudit`、`pub struct AuditFinding`、`pub fn audit_kernel(kernel: &Kernel) -> HostAudit`、`pub fn bootstrap(config: KernelConfig) -> CoreResult<Kernel>`、`pub fn results_json() -> CoreResult<serde_json::Value>`、`pub fn scenario(kernel: &mut Kernel) -> CoreResult<serde_json::Value>`、`impl Kernel { pub fn is_registered / agents_with_skill / registry_fingerprint / registry_snapshot / registry_defects / queued_envelopes / progress_events / audit }`
- **验收**：bootstrap() 宿主 7 项审计全过；构造空头质押或破坏倒排索引后 audit() 必须报失败；两次独立 bootstrap() 的 registry_fingerprint() 相等，换注册序则指纹不同；同一 &Kernel 可同时持有两个只读观察，观察前后注册表指纹不变；伪造信封单次 unauthorized -> Quarantine；证据闸门拒绝重复 5 次 policy_denied -> Warn 且 assert_ne!(.., Quarantine)；scenario() 在两个新内核上跑出的 JSON 完全相等，同一内核重复调用不 panic；冻结接口 Kernel::register/send/drain/settle/observe 等签名未改，语义逐条断言
- **证据**：`cargo test -p au4a-kernel (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-kernel)` → 27/27，等级 verified（本机实测）（本机 Windows 实测 2026-10-04：unittests 18 passed / host_kernel.rs 9 passed，0 failed，0 warning，exit 0。投递为内核内存队列语义（真实签名与验签），真实 P2P 传输不在本版声称范围。）
- **状态**：✅ 已交付

#### v1.0.2 Agent 自治层骨架

- **目标**：把「Agent 是主体」落成代码：纯函数决策器 + 真驱动内核的执行器，决策输入无人类通道、意图集合无「等待批准」
- **交付物**：`src/autonomy.rs: AutonomyPolicy / Intent / IntentKind / AgentContext / AutonomyLayer / AutonomyState / AutonomyTurn / classify_error`、`src/lib.rs: 接线自治层，scenario() 新增 autonomy 阶段（真实跑两个 turn）`、`tests/autonomy.rs: 4 个集成测试（自主闭环、退避vs自停、同种子同日志、日志完备）`
- **接口**：`pub enum IntentKind { Announce, Offer, Progress, Settle, Decline, Defer, Suspend, Idle }`、`pub enum Intent`、`pub struct AgentContext`、`pub struct AutonomyPolicy`、`pub struct AutonomyLayer`、`impl AutonomyLayer { new / did / policy / state / journal / join / context / turn }`、`pub fn classify_error(err: &CoreError) -> RefusalCode`
- **验收**：AutonomyLayer::join 自注册后 turn 产生验签通过的广播信封，全程无人工调用；AgentContext 的 JSON 键与 IntentKind::ALL 名称均不含 human/approv/operator；plan() 对相同输入结果相等；心跳边界之前只 Idle、到点才 Announce；竞争性拒绝 -> defer_until=at+3 且 escalation != Quarantine；伪造信封 -> 下一回合 plan==[Suspend]、sent==0、halted==true 且为终态；未声明能力 -> Decline{unsupported}；amount<=0 -> Decline{malformed}；声明能力 -> Offer{1% 整数基点}；同种子两个内核跑 6 回合：AutonomyTurn/AutonomyState/注册表指纹全等；journal.len() == intents_planned，且既有 acted 也有纯决定记录
- **证据**：`cargo test -p au4a-kernel (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-kernel)` → 42/42，等级 verified（本机实测）（本机 Windows 实测 2026-10-04：unittests 29 + tests/autonomy.rs 4 + tests/host_kernel.rs 9 = 42 passed，0 failed，0 warning，exit 0。协商接受语义属 au4a-negotiate（1.2），跨节点传输仍为内核内存队列。）
- **状态**：✅ 已交付

#### v1.0.3 人类观察层骨架

- **目标**：给人类进度/结果/收益三个只读投影，并让「只读」成为可枚举、可编译期检查的结构事实（不存在 &mut Kernel，写能力没有类型）
- **交付物**：`src/observer.rs: ObserverRoute / ObserverCapability(只有 Read) / ObserverProjection / ObserverReport / Observer / observer_api() / observer_self_checks() / 3 条编译期签名证据`、`src/lib.rs: 接线观察层，Kernel::self_check() 改为「宿主审计 + 真实观察层检查」，scenario() 新增 observer_layer 阶段`、`tests/observer.rs: 5 个集成测试（入口枚举无写路径、渲染不改状态、三投影内容、自检齐备、报告无指令通道）`
- **接口**：`pub enum ObserverRoute { Progress, Results, Yield }`、`pub enum ObserverCapability { Read }`、`pub struct ObserverProjection`、`pub struct ObserverReport`、`impl Observer { render(&Kernel, ObserverRoute) / render_all(&Kernel) / report(&Kernel) }`、`pub fn observer_api() -> serde_json::Value`、`pub fn observer_self_checks(kernel: &Kernel) -> Vec<SelfCheck>`
- **验收**：observer_api(): writable=false、capabilities==["read"]、3 条路由 effects 为空、路由名不含写动词；渲染前后 registry_fingerprint/observe_json/now 完全相等，两次报告与指纹相等；进度 event_count>0；结果 self_checks_passed=true 且 audit.clean=true；收益 agents.len()==agent_count()；Kernel::self_check() 含 4 个 observer.* 项与宿主审计项且全 passed、detail 非空；报告 JSON 不含 pending_approval/commands/mutations/actions_to_apply，但含三个面板；编译期证据：const _: fn(&Kernel) -> ObserverView = Kernel::observe 等 3 条签名断言
- **证据**：`cargo test -p au4a-kernel (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-kernel)` → 53/53，等级 verified（本机实测）（本机 Windows 实测 2026-10-04：unittests 35 + tests/autonomy.rs 4 + tests/host_kernel.rs 9 + tests/observer.rs 5 = 53 passed，0 failed，0 warning，exit 0。HTTP/Web 面板属 au4a-node，本版只断言内核侧结构只读性。）
- **状态**：✅ 已交付

#### v1.0.4 Agent 委员会骨架

- **目标**：建立只由在册 Agent 组成的裁决机构：确定性抽签、法定人数与多数、具名失败模式，且隔离动议只对恶意 2 码开放（竞争绝不隔离）
- **交付物**：`src/council.rs: CouncilConfig / Motion(内容寻址) / MotionKind / Ballot / Vote / Tally / Verdict / Decision / Council / CouncilFailure(8 个具名失败模式) / motions_for_kernel()`、`src/lib.rs: 接线委员会，scenario() 新增 council 阶段（真实抽签+全员投票+计票）`、`tests/council.rs: 5 个集成测试（抽签只抽在册 Agent、隔离只对恶意、法定人数与多数、非委员/重复投票、空委员会）`
- **接口**：`pub struct CouncilConfig { size, quorum_bps, pass_bps }`、`pub struct Motion { id, kind, subject, cause, escalation, at }`、`pub enum MotionKind { Warn, Quarantine, Slash, Reprieve }`、`pub enum Ballot { Uphold, Reject, Abstain }`、`pub enum Verdict { Upheld, Rejected, Inconclusive }`、`pub enum CouncilFailure { NonMemberVote, DuplicateVote, MalformedMotion, UnknownMotion, DuplicateMotion, NoQuorum, TieInsufficient, EmptyCouncil }`、`impl Council { sortition / members / is_member / quorum_required / submit / vote / tally / decide / decisions / decisions_json / failures }`、`pub fn motions_for_kernel(kernel: &Kernel, at: u64) -> CoreResult<Vec<Motion>>`
- **验收**：sortition 可复现、委员全部 is_registered、在册不足取全部、空网络 -> EmptyCouncil 且映射为 degraded；穷举 10 拒绝码 x 3 升级级别：只有恶意 2 码能产生 Quarantine 动议；竞争码即使误传 Quarantine 也返回 None；真实内核：伪造信封 -> 1 份隔离动议(unauthorized)；证据闸门重复 5 次 -> 1 份 Warn 动议(policy_denied)；动议 id 自洽；5 人 60% 法定 3 人；2 人出席 -> NoQuorum+Inconclusive；3:2 -> Upheld；全员弃权 -> TieInsufficient 且不产生决定；NonMemberVote/DuplicateVote/MalformedMotion -> unauthorized(恶意)；UnknownMotion->unsupported，DuplicateMotion->conflict，其余->degraded；同输入两次运行 Tally/decisions/decisions_json 指纹全等
- **证据**：`cargo test -p au4a-kernel (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-kernel)` → 65/65，等级 verified（本机实测）（本机 Windows 实测 2026-10-04：unittests 42 + tests/autonomy.rs 4 + tests/council.rs 5 + tests/host_kernel.rs 9 + tests/observer.rs 5 = 65 passed，0 failed，0 warning，exit 0。决定执行落 v1.0.7 状态机；真实链上治理属 1.7/1.8。）
- **状态**：✅ 已交付

#### v1.0.5 权限模型更新

- **目标**：内核能回答「这个 Agent 能做什么/不能做什么」：全部能力的恰好一次划分 + 类型化拒绝理由，权限来源只有客观事实、没有人类或运营方
- **交付物**：`src/permission.rs: Capability(9 项) / Authority(5 个客观来源) / Denial / PermissionReport / explain / explain_with_council / authority_roots()`、`src/lib.rs: Kernel::permissions(&Did)，scenario() 新增 permission 阶段`、`tests/permission.rs: 6 个集成测试（划分完备、来源无人类、外部 DID 全拒、竞争不沾恶意码、只读可复现、能力名唯一）`
- **接口**：`pub enum Capability { PublishCard, DeliverDirect, SettleVerified, SettleCpuProto, Offer, VoteInCouncil, ProposeMotion, ListSkill, WithdrawStake }`、`pub enum Authority { SelfSovereign, Stake, Evidence, DeclaredSkill, CouncilDecision }`、`pub struct Denial { capability, code, reason }`、`pub struct PermissionReport { did, registered, allowed, denied, limits, authorities }`、`pub fn explain(kernel: &Kernel, did: &Did) -> CoreResult<PermissionReport>`、`pub fn explain_with_council(kernel: &Kernel, did: &Did, council: &Council) -> CoreResult<PermissionReport>`、`pub fn authority_roots() -> serde_json::Value`、`impl Kernel { pub fn permissions(&self, did: &Did) -> CoreResult<PermissionReport> }`
- **验收**：allowed.len()+denied.len()==9 且 is_total_partition()；allows 与 denial_for 对同一能力互斥；Authority::ALL 字符串不含 operator/human/admin/owner；authority_roots()["human_is_a_root"]==false；提案权来自 self_sovereign；未注册 DID 的 9 项能力全拒且 code=unauthorized(is_misconduct=true, retryable=false)，与内核 send 分类一致；在册 Agent 的拒绝清单无恶意码；cpu_proto_cap=100 与 resource_exhausted 上限拒绝如实暴露；两次 permissions() 报告与指纹相等；解释前后注册表指纹/观察投影不变；9 个能力名唯一且 parse 往返成功；Capability::parse("become_operator")==None
- **证据**：`cargo test -p au4a-kernel (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-kernel)` → 79/79，等级 verified（本机实测）（本机 Windows 实测 2026-10-04：unittests 50 + tests/autonomy.rs 4 + tests/council.rs 5 + tests/host_kernel.rs 9 + tests/observer.rs 5 + tests/permission.rs 6 = 79 passed，0 failed，0 warning，exit 0。真实 API 网关注入属 au4a-node / au4a-safety(1.5)。）
- **状态**：✅ 已交付

#### v1.0.6 PMB协议扩展

- **目标**：在冻结线格式上扩展协议语义：消息类型分类、准入、收件人展开、重放保护与逻辑时钟新鲜度，拒绝全部类型化且分类不丢
- **交付物**：`src/pmb.rs: kinds_ext(8 个类型) / MessageClass(6 类) / classify_kind / RouteDecision / RouterStats / PmbRouter / announce_card / progress / settle_request / council_vote / decode_and_verify / encode_checked`、`src/lib.rs: 接线协议层，scenario() 新增 pmb 阶段（真信封走准入+重放被拒）`、`tests/pmb.rs: 6 个集成测试（准入与内核一致、篡改vs重放、路由不改内核、线格式与上限、治理不得广播、两次运行全等）`
- **接口**：`pub mod kinds_ext`、`pub enum MessageClass { Announce, Negotiation, Settlement, Governance, Telemetry, Unknown }`、`pub fn classify_kind(kind: &str) -> MessageClass`、`pub struct RouteDecision { id, kind, class, accepted, recipients, code, reason }`、`pub struct PmbRouter; impl PmbRouter { admit(&Kernel, &Envelope) / stats }`、`pub fn announce_card / progress / settle_request / council_vote`、`pub fn decode_and_verify(frame: &[u8]) -> CoreResult<Envelope>`、`pub fn encode_checked(env: &Envelope) -> CoreResult<Vec<u8>>`
- **验收**：经 admit 准入的 4 类消息 Kernel::send 也必须收下（admitted==4, refused==0）；篡改->unauthorized(恶意)；重放->conflict(非恶意,retryable,replays==1)；未知类型->unsupported；收件人不在册->stale_epoch；落后>64->stale_epoch；治理广播->policy_denied；admit 前后注册表指纹/观察投影/队列长度不变；encode_checked 往返 id/sig 一致；改一字节验签失败；超 1 MiB 前缀与截断帧被拒；4 个在册 Agent 广播展开 recipients.len()==3 且不含发送者；by_class[telemetry]==1；同序列两次运行 RouteDecision 与 RouterStats 全等；admitted+refused==判定数；direct+broadcasts==admitted
- **证据**：`cargo test -p au4a-kernel (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-kernel)` → 92/92，等级 verified（本机实测）（本机 Windows 实测 2026-10-04：unittests 57 + autonomy 4 + council 5 + host_kernel 9 + observer 5 + permission 6 + pmb 6 = 92 passed，0 failed，0 warning，exit 0。真实跨进程传输属 au4a-node / au4a-scale(1.9)，本版只断言协议语义与线格式合规。）
- **状态**：✅ 已交付

#### v1.0.7 状态机更新

- **目标**：把 Agent 生命周期变成白名单状态机 + 事件溯源，并把治理底线写进迁移表：竞争只降级（可恢复）、隔离只来自恶意证据或委员会决定、退役是终态
- **交付物**：`src/lifecycle.rs: AgentState(6 态) / LifecycleEvent(8 种) / next_state 纯迁移函数 / Transition / Lifecycle(apply/can/allowed_events/replay/attempt/fingerprint) / LifecycleOutcome / LifecycleBook`、`src/lib.rs: Kernel 增加 lifecycles 台账（注册即 Admitted）+ lifecycle_of/lifecycles/apply_lifecycle/apply_council_decision，scenario() 新增 lifecycle 阶段`、`src/audit.rs: 新增审计项 lifecycle.tracked（在册必有台账、台账无孤儿）`、`src/observer.rs: 「结果」面板加入生命周期状态分布`、`tests/lifecycle.rs: 6 个集成测试`
- **接口**：`pub enum AgentState { Provisional, Active, Busy, Degraded, Quarantined, Retired }`、`pub enum LifecycleEvent { Admitted, WorkStarted, WorkFinished, Refused(RefusalCode), Recovered, CouncilQuarantine, CouncilReprieve, Retired }`、`pub fn next_state(AgentState, LifecycleEvent) -> Result<AgentState, RefusalCode>`、`pub struct Lifecycle; pub struct LifecycleBook; pub struct LifecycleOutcome; pub struct Transition`、`impl Kernel { lifecycle_of / lifecycles / apply_lifecycle / apply_council_decision }`
- **验收**：注册 3 个 Agent 后各 1 条 Admitted、状态 Active、count(Active)==3、audit 的 lifecycle.tracked 通过、self_check 全绿；10 个拒绝码穷举：恶意 2 码 -> Quarantined，其余 8 码 -> Degraded；连续 20 次限流仍 Degraded 且 count(Quarantined)==0；Recovered 直接回 Active；伪造信封 -> Quarantined；仅 Upheld 的 Reprieve 决定能回 Active；Rejected 无执行力(None)；决定与动议不匹配 -> Err(InvalidKind)；Upheld Quarantine 动议使目标隔离；退役后 5 种事件全部 applied==false 且 code==stale_epoch；重复退役幂等；未注册 DID 施加事件 -> Err(UnknownAgent)；同事件序列两次运行台账指纹相等；can() 与 apply() 在所有可达状态×事件样本上一致；allowed_events() 恰为 can 为真集合
- **证据**：`cargo test -p au4a-kernel (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-kernel)` → 106/106，等级 verified（本机实测）（本机 Windows 实测 2026-10-04：unittests 65 + autonomy 4 + council 5 + host_kernel 9 + lifecycle 6 + observer 5 + permission 6 + pmb 6 = 106 passed，0 failed，0 warning，exit 0。Slash 的经济执行属 au4a-economy(1.4)，跨节点同步属 1.3/1.7。）
- **状态**：✅ 已交付

#### v1.0.8 迁移适配器

- **目标**：把 v3.x 风格插件接口如实映射到 PMB 能力路由：能映射的映射、不能映射的具名拒绝、依赖人类审批的直接失败
- **交付物**：`src/migration.rs: V3PluginManifest / MigrationRefusal(8 个具名拒绝+拒绝码映射) / MigrationLimits / HookRoute / CapabilityGrant / MigrationPlan(内容寻址) / hook_route / permission_capability / adapt / apply_plan(fail-closed)`、`src/lib.rs: 接线迁移层，scenario() 新增 migration 阶段（真实适配+真实投递+运营方依赖被拒）`、`tests/migration.rs: 6 个集成测试（路由表完整、运营方审批被拒、宿主权限如实记录、执行真签名、被拒能力阻断路由、内容寻址可复现）`
- **接口**：`pub struct V3PluginManifest { name, entry, hooks, permissions, requires_operator_approval, owner }`、`pub enum MigrationRefusal { EmptyName, MissingEntry, UnsupportedHook(String), UnsupportedPermission(String), OperatorApprovalRequired, DuplicateHook(String), TooManyHooks(usize), PlanNotEncodable }`、`pub struct MigrationPlan { id, plugin, entry, routes, grants, refused_permissions, evidence }`、`pub fn hook_route(hook: &str) -> Option<HookRoute>`、`pub fn permission_capability(permission: &str) -> Option<Capability>`、`pub fn adapt(&V3PluginManifest, &MigrationLimits) -> Result<MigrationPlan, MigrationRefusal>`、`pub fn apply_plan(&mut Kernel, &AgentKeys, &MigrationPlan) -> CoreResult<MigrationApplication>`
- **验收**：9 个旧钩子全部映射到真实 PMB 类型且 classify_kind 非 Unknown；on_teleport -> None；requires_operator_approval 或 owner:*/operator:* -> Err(OperatorApprovalRequired) 且 to_refusal_code()==policy_denied（非恶意）；net:http/fs:read/exec:shell -> 记入 refused_permissions(unsupported)，is_partial()==true；apply_plan 发出的信封逐个 verify() 通过、from 正确、migration.applied 事件落库；篡改计划 is_intact()==false -> Err；fail-closed：stake:withdraw 被拒时 denied 记录 (withdraw_stake, policy_denied)，该路由不发消息；同清单两次 adapt 的 id 与指纹相等；钩子换序 -> 换 id；改 entry -> is_intact()==false；plan.evidence == CpuProto（JSON "cpu-proto"）：不执行插件字节码
- **证据**：`cargo test -p au4a-kernel (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-kernel)` → 120/120，等级 verified（本机实测）（本机 Windows 实测 2026-10-04：unittests 73 + autonomy 4 + council 5 + host_kernel 9 + lifecycle 6 + migration 6 + observer 5 + permission 6 + pmb 6 = 120 passed，0 failed，0 warning，exit 0。不执行插件字节码（WASM 运行时不在本 crate），故适配产物标注 cpu-proto。）
- **状态**：✅ 已交付

#### v1.0.9 测试框架

- **目标**：把「同样的种子给同样的结果」变成可复用工具：确定性仿真台 + 可重放日志 + 不变式套件（含无据隔离检测）
- **交付物**：`src/harness.rs: JournalStep(8 种) / JournalEntry / Journal / Harness(add_agents/announce/offer/settle/attempt_forgery/autonomy_turn/lifecycle/journal_fingerprint/replay/verify_replay) / ReplayOutcome / invariant_suite()`、`src/lib.rs: 接线测试框架，scenario() 新增 harness 阶段（跑脚本 + 自校验重放）`、`tests/harness.rs: 5 个集成测试（同脚本同世界、重放重建、不变式覆盖、真实内核效果、日志可序列化）`
- **接口**：`pub enum JournalStep { Register{display,skills,stake}, Announce, Offer{to,skill,price}, Settle{to,amount,grade}, Forgery, AutonomyTurn{planned}, Lifecycle{event} }`、`pub struct JournalEntry { at, actor, step, ok }`、`pub struct Harness; impl Harness { new / add_agents / announce / offer / settle / attempt_forgery / autonomy_turn / lifecycle / journal_fingerprint / replay / outcome / verify_replay }`、`pub struct ReplayOutcome { registry_fingerprint, lifecycle_fingerprint, minted, slashed, refusals, journal_len }`、`pub fn invariant_suite(kernel: &Kernel) -> Vec<SelfCheck>`
- **验收**：同 base_tag 两次同脚本：日志指纹/注册表/生命周期/发行量/拒绝数全等，且日志含 ok=false；verify_replay() 四个面全等；重放内核通过 invariant_suite 且发行量一致（双轨守恒）；套件含 >=10 项具名检查（宿主 7 + harness 3：quarantine_justified / registry_lifecycle_bijection / queue_sealed），全 passed 且 detail 非空；脚本后 3 Agent 在册、3 条生命周期、队列全部验签通过、发行量>0、含恶意拒绝、genesis 事件在案、audit 干净；journal_json 长度==日志长度且每条含 step/at/ok；两次运行 JSON 全等；did_of(99) 返回 Err 而非 panic
- **证据**：`cargo test -p au4a-kernel (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-kernel)` → 130/130，等级 verified（本机实测）（本机 Windows 实测 2026-10-04：unittests 78 + autonomy 4 + council 5 + harness 5 + host_kernel 9 + lifecycle 6 + migration 6 + observer 5 + permission 6 + pmb 6 = 130 passed，0 failed，0 warning，exit 0。框架是单进程内存仿真，跨节点重放属 au4a-state(1.3)。）
- **状态**：✅ 已交付

#### v1.0.10 文档

- **目标**：把文档变成机器可校验产物：类型化轨道清单 + 逐条断言（版本数/顺序/字段/证据/往返），文档声称的能力必须可达
- **交付物**：`src/manifest.rs: VERSION_ORDER / VersionEvidence / VersionSpec / TrackManifest / track_manifest()(10 版完整规格) / validate_manifest()(7 项) / manifest_is_valid()`、`src/lib.rs: self_check() 聚合清单校验（共 10 项）、results_json() 输出版本清单与总测试数、scenario() 新增 manifest 阶段`、`tests/manifest.rs: 4 个集成测试（10 版齐备、字段与证据完整、校验能发现篡改、自检/结果聚合清单）`
- **接口**：`pub const VERSION_ORDER: [&str; 10]`、`pub struct VersionEvidence { test_command, tests_passed, tests_total, grade, notes }`、`pub struct VersionSpec { version, title, goal, deliverables, interfaces, acceptance, evidence, status }`、`pub struct TrackManifest { track, crate_name, medium_title, range, owner, versions }`、`pub fn track_manifest() -> TrackManifest`、`pub fn validate_manifest(&TrackManifest) -> Vec<SelfCheck>`、`pub fn manifest_is_valid() -> bool`
- **验收**：versions.len()==10 且 version 序列与 VERSION_ORDER 逐项相等，all_done()==true，v1.0.11 -> None；每版 goal 非空、交付物/接口/验收各 >=3 条、status==done；证据 grade==verified、tests_passed==tests_total>0、命令指向 au4a-kernel、notes 非空，测试数单调不减；篡改检测：删版本->version_count/order 失败；清空验收->fields 失败；伪造 grade->evidence 失败；测试数回退->test_growth 失败；版本号写错->order 失败；self_check() 含 track.wired/host.audit/registry.deterministic + 7 项 manifest 检查且全 passed；results_json(): versions==10、manifest_valid==true、version_ids.len()==10、tests_total_latest==139；to_json/from_json 往返相等，缺字段 JSON 解析失败
- **证据**：`cargo test -p au4a-kernel (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-kernel)` → 139/139，等级 verified（本机实测）（本机 Windows 实测 2026-10-04 干净重建（cargo clean -p au4a-kernel 后重跑）：12 个测试目标全部 ok —— unittests 83 + autonomy 4 + council 5 + harness 5 + host_kernel 9 + lifecycle 6 + manifest 4 + migration 6 + observer 5 + permission 6 + pmb 6 = 139 passed，0 failed，0 warning，exit 0。）
- **状态**：✅ 已交付

### v1.1 Capability Graph 能力图（`au4a-capgraph`）

#### v1.1.1 数据结构

- **目标**：给「一条能力」一个整数、可校验、可内容寻址的表示（skill/latency_p50/latency_p99/throughput/current_load/price/reliability/supported_formats/constraints）
- **交付物**：`src/capability.rs: SkillId/FormatId/Constraints/Capability 及校验、加权度量、格式接续、指纹`、`src/lib.rs: 模块接入 + 5 条真实自检 + v1.1.1 scenario`、`tests/data_structures.rs: 7 个集成测试`
- **接口**：`pub struct Capability { skill, latency_p50_ms, latency_p99_ms, throughput_per_min, current_load_bp, price_per_unit, reliability_bp, supported_formats, produced_formats, constraints }`、`pub fn Capability::validate(&self) -> CoreResult<()>`、`pub fn Capability::effective_latency_ms(&self) -> u64`、`pub fn Capability::effective_reliability_bp(&self) -> u16`、`pub fn Capability::handoff_format(&self, next: &Capability) -> Option<FormatId>`、`pub fn Capability::fingerprint(&self) -> CoreResult<String>`、`pub struct Constraints { max_input_bytes, min_deadline_ms, max_concurrency, regions }`
- **验收**：9 个度量字段在规范 JSON 中均为整数，含浮点的值被 canonicalize 以 FloatForbidden 拒绝；p99<p50 / load>10000bp / reliability>10000bp / 空格式集合 均被 validate() 拒绝；p50=100,p99=300,load=5000bp 时加权延迟=200ms、加权可靠度=4500bp（整数运算，实测相等）；同一能力两次构造指纹相同；改价后指纹改变（SHA-256 内容寻址）；产出∩接受={application/json} 判定可接续；产出∩接受=∅ 判定不可接续；scenario 在同样输入下两次运行返回完全相同 JSON（可重放）
- **证据**：`cargo test -p au4a-capgraph` → 14/14，等级 verified（本机实测）（本机 Windows 实测：lib 7 passed + tests/data_structures 7 passed，0 failed，本 crate 0 warning。唯一警告来自冻结的 au4a-kernel（unused import all_passed），不在本轨道写作用域。）
- **状态**：✅ 已交付

#### v1.1.2 声明 API

- **目标**：Agent 用自签声明把自己的能力写进图，且任何人都不能替他人声明
- **交付物**：`src/declaration.rs: Declaration（自洽校验、按技能排序、拒绝重复技能）/ SignedDeclaration（验签）`、`src/graph.rs: AgentCapabilityGraph / CapGraphConfig / NeighborRecord / DeclareOutcome`、`tests/declaration.rs: 8 个集成测试`
- **接口**：`pub fn Declaration::new(agent: Did, epoch: u64, issued_at: u64, capabilities: Vec<Capability>) -> CoreResult<Declaration>`、`pub fn Declaration::sign(self, keys: &AgentKeys) -> CoreResult<SignedDeclaration>`、`pub fn SignedDeclaration::verify(&self) -> CoreResult<()>`、`pub fn AgentCapabilityGraph::apply(&mut self, signed: &SignedDeclaration, now: u64) -> DeclareOutcome`、`pub fn AgentCapabilityGraph::capabilities_of(&self, did: &Did) -> Option<&[Capability]>`、`pub enum DeclareOutcome { Applied, Unchanged, Rejected }`
- **验收**：自签声明入图；同一份声明重放 4 次全部 Unchanged 且 own_epoch 不推进（幂等）；B 用自己私钥替 A 声明返回 InvalidSignature；把已签声明的 agent 改成 B 被图判 unauthorized；篡改已签声明的 price/throughput → unauthorized（恶意码 is_misconduct()==true），邻居视图保持为空；epoch 回退→stale_epoch；同 epoch 异内容→conflict；epoch 递增→接受并完整替换旧能力集；超过 max_skills_per_agent→policy_denied，且被拒声明不改变 capability_count()；邻居声明不触碰自有状态；skills() 去重列出全部提供者
- **证据**：`cargo test -p au4a-capgraph` → 35/35，等级 verified（本机实测）（本机 Windows 实测：lib 20 passed + tests/data_structures 7 passed + tests/declaration 8 passed，0 failed，本 crate 0 warning。）
- **状态**：✅ 已交付

#### v1.1.3 广播协议

- **目标**：用 PMB Envelope + MsgKind 广播能力通告，真签名验签，篡改被拒
- **交付物**：`src/broadcast.rs: announce/announce_to/query_skill/parse_announcement/parse_query/ingest/pump + Ingest/PumpReport`、`src/lib.rs: demo_agents() 确定性角色集合 + scenario 改为真广播 + 4 条新自检`、`tests/broadcast.rs: 10 个集成测试`
- **接口**：`pub const PROTOCOL: &str = "au4a.capgraph/1"`、`pub const KIND_ANNOUNCE: &str = "capgraph.announce" / KIND_REQUEST: &str = "capgraph.request"`、`pub fn announce(keys: &AgentKeys, declaration: &Declaration, ts: u64) -> CoreResult<Envelope>`、`pub fn parse_announcement(env: &Envelope) -> Result<Announcement, (RefusalCode, String)>`、`pub fn ingest(graph: &mut AgentCapabilityGraph, env: &Envelope, now: u64) -> Ingest`、`pub fn pump(kernel: &mut Kernel, graph: &mut AgentCapabilityGraph, now: u64) -> PumpReport`
- **验收**：announce 的信封 verify() 通过、to==None、kind==capgraph.announce；分帧往返后信封相等且内容寻址 id 不变；篡改 price/throughput/reliability/load 任一字段 → unauthorized（is_misconduct()==true）；B 的信封装 A 的合法声明 → 身份错配判 unauthorized，图不变（capability_count()==0）；错误 kind 或错误 protocol → unsupported，且不触图；5 Agent 全量广播 → pump 报 routed=4/applied=4/refused=0，known_agents=5，内核 refusals 为空；他轨道信封被转发回队列（id 不变）；伪造通告的拒绝记在伪造者头上（escalation_for==Quarantine）
- **证据**：`cargo test -p au4a-capgraph` → 52/52，等级 verified（本机实测）（本机 Windows 实测：lib 27 + tests/broadcast 10 + tests/data_structures 7 + tests/declaration 8，0 failed，本 crate 0 warning。签名/验签/内容寻址/分帧/内核准入为真实实现；同进程内的队列传递标注 cpu-proto（本 crate 禁止网络 I/O）。）
- **状态**：✅ 已交付

#### v1.1.4 缓存层

- **目标**：邻居能力缓存：容量上限 + TTL 失效 + LRU 淘汰，全部确定性
- **交付物**：`src/cache.rs: CapabilityCache（容量上限/TTL/LRU/命中统计）+ CacheStats + CacheInsert`、`src/graph.rs: 邻居侧改由缓存托管；新增 get_neighbor/expire_neighbors/invalidate_neighbor/cache_stats/lru_order/live_neighbor_count`、`tests/cache.rs: 9 个集成测试`
- **接口**：`pub fn CapabilityCache::new(capacity: usize, ttl_ticks: u64) -> Self`、`pub fn CapabilityCache::get(&mut self, did: &Did, now: u64) -> Option<&NeighborRecord>`、`pub fn CapabilityCache::insert(&mut self, record: NeighborRecord, now: u64) -> CacheInsert`、`pub fn CapabilityCache::is_live(&self, did: &Did, now: u64) -> bool`、`pub fn CapabilityCache::expire(&mut self, now: u64) -> Vec<Did>`、`pub fn CapabilityCache::evict_lru(&mut self) -> Option<Did>`、`pub struct CacheStats { hits, misses, inserts, updates, evictions, expirations, invalidations }`
- **验收**：容量 8 写入 64 条过程中 len 峰值<=8、末尾 len=8、evictions=56；容量 3 时 evictions=7（硬上限）；触碰 1、2 后 lru_order 恰为 [3,1,2]，下一次写入淘汰 3；重跑同一序列得到同一淘汰结果；TTL 用逻辑刻：at=1000,ttl=20 时 1020 刻存活、1021 刻失效并计入 expirations；ttl=0 永不过期；邻居过期后同 epoch 异内容不再判 conflict 而是重新接受（旧世界已不存在）；invalidate 只移除指定邻居；重复失效为空操作且不再计数；容量 0 时返回 resource_exhausted，且 is_misconduct()==false（容量是竞争语义）；真实广播下容量 2 的图：4 条通告全部 applied、保留 2 条、淘汰 2 条；两个独立图 lru_order 相同
- **证据**：`cargo test -p au4a-capgraph` → 69/69，等级 verified（本机实测）（本机 Windows 实测：lib 35 + broadcast 10 + cache 9 + data_structures 7 + declaration 8 = 69，0 failed，本 crate 0 warning。语义变更：v1.1.2 的“满了就拒”占位策略被 LRU 淘汰取代。）
- **状态**：✅ 已交付

#### v1.1.5 查询接口

- **目标**：按技能/格式/价格/可靠度查询能力，走倒排索引而不是全图扫描
- **交付物**：`src/index.rs: CapabilityIndex（by_skill/by_input_format 倒排表，键为 (did,slot)）+ CapabilityQuery/CapabilityMatch/QueryStats/QueryResult`、`src/graph.rs: query/best_for/providers_of/capability_at/index_consistent/index_summary，索引在声明/淘汰/过期/失效四个入口同步`、`tests/query_index.rs: 8 个集成测试`
- **接口**：`pub fn AgentCapabilityGraph::query(&mut self, q: &CapabilityQuery, now: u64) -> QueryResult`、`pub fn AgentCapabilityGraph::best_for(&mut self, skill: &SkillId, now: u64) -> QueryResult`、`pub fn AgentCapabilityGraph::providers_of(&self, skill: &SkillId) -> Vec<Did>`、`pub fn AgentCapabilityGraph::index_consistent(&self) -> bool`、`pub struct QueryStats { candidates, scanned, matched, nodes_total }`、`pub fn CapabilityIndex::candidates_for(&self, skill: &SkillId, input_format: Option<&FormatId>) -> Vec<CapKey>`
- **验收**：视图 200 条能力时按技能查询 candidates==scanned==100、nodes_total==200，scanned < nodes_total；索引结果与朴素全扫（遍历全部邻居能力逐个过滤）集合完全相同；max_price/min_reliability/max_latency/min_throughput/min_available_bp/region/输入输出格式 均为硬条件；输入格式过滤 = by_skill ∩ by_input_format：2 条候选中 application/json 只命中 1 条；排序第一关键字为负载加权可靠度：9000/9000/8000bp → 价格序 5,5,4，两次查询逐位相同；淘汰/过期/显式失效后 index_consistent()==true，indexed_entries 与实际能力数一致；自有能力也被索引；查不到给出 QueryResult::refusal_code()==Some(unsupported)（非恶意、非异常）；scenario 报告 nodes_total=5、translate scanned=2
- **证据**：`cargo test -p au4a-capgraph` → 82/82，等级 verified（本机实测）（本机 Windows 实测：lib 40 + broadcast 10 + cache 9 + data_structures 7 + declaration 8 + query_index 8 = 82，0 failed，本 crate 0 warning。）
- **状态**：✅ 已交付

#### v1.1.6 路径规划

- **目标**：真图搜索：给定技能序列与格式，返回最小代价流水线或明确的「无路径」
- **交付物**：`src/planner.rs: PipelineRequest/PipelineStep/PlanCost/PlanOutcome/Pipeline/PipelineNode/NoPath/NoPathReason/SearchStats + 多准则标签设定搜索`、`src/graph.rs: AgentCapabilityGraph::plan(request, cost, now)`、`tests/planner.rs: 9 个集成测试`
- **接口**：`pub fn plan(graph: &mut AgentCapabilityGraph, request: &PipelineRequest, cost: &PlanCost, now: u64) -> PlanOutcome`、`pub fn AgentCapabilityGraph::plan(&mut self, request, cost, now) -> PlanOutcome`、`pub struct PlanCost { latency_price_per_ms: i64, handoff_price: i64 }`、`pub enum PlanOutcome { Path(Pipeline), NoPath(NoPath) }`、`pub enum NoPathReason { EmptyRequest, NoProvider, NoCompatibleFormat, ConstraintRejected, BudgetExceeded, DeadlineExceeded }`、`pub struct SearchStats { settled, dominance_pruned, constraint_pruned, labels_capped, candidates }`
- **验收**：text/plain + translate+sentiment → bob(→application/json) → carol，总价 5、总延迟 200ms、1 次换手；贪心反例：最便宜的第一步 dave(1 微积分, text/html) 的可接续下一步数==0，规划改选 bob(3)；强制中间格式 text/html → NoCompatibleFormat{step:1} + unsupported，且 search.settled>0；预算 4 → BudgetExceeded{4}+policy_denied；期限 150ms → DeadlineExceeded{150}+timeout；预算 5/期限 200ms 恰好可行；未知技能 → NoProvider{step:0}；空请求 → EmptyRequest+malformed；载荷 >1MiB 上限 → ConstraintRejected{step:0}+policy_denied；1KiB 可行；三步链 audio/wav→text/plain→application/json 绕开 1 微积分诱饵，总价 14、换手 2；两个相同图的规划输出逐字节相同（含无路径）；scenario 返回 plan.outcome==path（bob→carol、总价 5）与 no_path_demo（unsupported）
- **证据**：`cargo test -p au4a-capgraph (CARGO_INCREMENTAL=0)` → 101/101，等级 verified（本机实测）（本机 Windows 实测：lib 50 + broadcast 10 + cache 9 + data_structures 7 + declaration 8 + planner 9 + query_index 8 = 101，0 failed，0 warning（含依赖）。注：CARGO_INCREMENTAL=0 用于规避 Windows 增量目录复制失败（os error 5）产生的非代码警告。）
- **状态**：✅ 已交付

#### v1.1.7 版本化

- **目标**：能力变更自动递增版本，能识别陈旧通告与同版本异内容冲突
- **交付物**：`src/version.rs: VersionRecord + VersionHistory（append-only、有界 256、按 Agent 可查、hash_of 内容寻址）`、`src/declaration.rs: Declaration::capabilities_fingerprint()（只对能力集合取指纹）`、`src/graph.rs: declare() 自动递增 + history()/last_version_of()/version_summary() + 历史兜底的陈旧/冲突判定`、`tests/versioning.rs: 9 个集成测试`
- **接口**：`pub fn AgentCapabilityGraph::declare(&mut self, keys: &AgentKeys, capabilities: Vec<Capability>, now: u64) -> CoreResult<DeclareOutcome>`、`pub fn AgentCapabilityGraph::history(&self) -> &VersionHistory`、`pub fn AgentCapabilityGraph::last_version_of(&self, did: &Did) -> Option<u64>`、`pub fn AgentCapabilityGraph::version_summary(&self) -> Value`、`pub fn Declaration::capabilities_fingerprint(&self) -> CoreResult<String>`、`pub const HISTORY_CAPACITY: usize = 256`
- **验收**：三次内容变更 → 版本 1→2→3、历史 3 条、bumps==3；每条历史哈希 == 该版能力集合指纹；内容不变的重发 → Unchanged 且版本不推进、历史不新增；非 owner 调用 declare → Err(InvalidSignature)，版本保持 0；缓存存活时 stale_epoch / conflict / 递增接受三条路径均成立；缓存过期后同版本异内容仍 conflict、更旧版本仍 stale_epoch（历史兜底）、更高版本接受；历史有界：写入 HISTORY_CAPACITY+7 次后 len()==256，版本号涨到 263；version_summary() 报告自有版本、邻居版本映射、历史条数、递增次数；广播：v1→v2 均 applied；重放 v1 被 pump 判 stale_epoch 且非恶意码，图停在 v2；scenario: version_bump_applied/idempotent_ignored/stale_rejected 均为 true，versions.own==2、bumps==6
- **证据**：`cargo test -p au4a-capgraph (CARGO_INCREMENTAL=0)` → 113/113，等级 verified（本机实测）（本机 Windows 实测：lib 53 + broadcast 10 + cache 9 + data_structures 7 + declaration 8 + planner 9 + query_index 8 + versioning 9 = 113，0 failed，0 warning。语义变更：v1.1.4 的「缓存过期=当作没听过」被 v1.1.7 的「历史兜底版本判定」取代（同版本异内容过期后仍 conflict）。）
- **状态**：✅ 已交付

#### v1.1.8 性能优化

- **目标**：增量索引维护 + 有界 top-k 选择 + 查询结果缓存，用操作计数而非墙钟证明
- **交付物**：`src/perf.rs: QueryPerf / rank_top_k（有界选择）/ QueryCache+QueryCacheStats / rebuild_index`、`src/graph.rs: revision()/query_cached()/query_perf()/query_cache_stats()/query_cache_capacity()/reindex()/perf_summary()`、`src/index.rs: rank_and_truncate 作为全排序参考实现（等价性对照）`、`tests/perf.rs: 9 个集成测试`
- **接口**：`pub fn rank_top_k(matches: Vec<CapabilityMatch>, k: usize, perf: &mut QueryPerf) -> Vec<CapabilityMatch>`、`pub fn AgentCapabilityGraph::query_cached(&mut self, q: &CapabilityQuery, now: u64) -> QueryResult`、`pub fn AgentCapabilityGraph::revision(&self) -> u64`、`pub fn AgentCapabilityGraph::reindex(&mut self)`、`pub fn AgentCapabilityGraph::perf_summary(&self) -> Value`、`pub struct QueryPerf { queries, full_sorts, bounded_selections, comparisons, candidates_scanned, index_rebuilds }`
- **验收**：80 邻居×2 能力增量入图：index_rebuilds==0、entries==160、writes==160、索引自洽；容量 4 缓存淘汰后索引同步（被淘汰邻居不在 providers_of 中）；64 候选取前 3：bounded_selections==1、full_sorts==0，且与全排序前 3 逐位相同；rank_top_k 与 rank_and_truncate 结果一致；同一查询第二次命中；apply 推进修订号后 invalidated==1；只读查询不推进修订号；缓存容量 0 → 关闭缓存但结果与直接查询一致；reindex() 使 index_rebuilds==1 且索引仍自洽、查询结果不变；同一负载两次运行 perf_summary() 逐字节相同（证据不依赖时间）；scenario.perf: index_rebuilds==0、index.entries==5、bounded_selections>=1、full_sorts>=1、query_cache.misses>=1、query_cache_hit==true
- **证据**：`cargo test -p au4a-capgraph (CARGO_INCREMENTAL=0)` → 126/126，等级 verified（本机实测）（本机 Windows 实测：lib 57 + broadcast 10 + cache 9 + data_structures 7 + declaration 8 + perf 9 + planner 9 + query_index 8 + versioning 9 = 126，0 failed，0 warning。性能以操作计数（扫描/比较/排序/淘汰）为证据，不含任何墙钟读数。）
- **状态**：✅ 已交付

#### v1.1.9 测试

- **目标**：端到端场景 + 不变式 + 对抗用例的完整测试面，并提供独立校验器
- **交付物**：`src/planner.rs: verify_pipeline() 独立校验器（格式链/度量/约束/代价自洽）`、`tests/end_to_end.rs: 9 个端到端测试（全链路、换图独立验通、逐字节可重放、索引==全扫）`、`tests/invariants.rs: 5 个不变式测试（确定性 LCG 8 种子×60 步，逐步断言 I1–I7）`、`tests/adversarial.rs: 11 个对抗测试（改价/重放/冲突/走私/轰炸/饥饿/容量 0/极端查询）`、`src/lib.rs: checks_v119（校验器正反用例 + scenario 逐版字段覆盖）`
- **接口**：`pub fn verify_pipeline(graph: &AgentCapabilityGraph, request: &PipelineRequest, cost: &PlanCost, pipeline: &Pipeline) -> Result<(), String>`
- **验收**：规划结果能被 verify_pipeline 逐项验通；换一张图（仅凭同样的广播数据）也能验通；改价 / 改格式 / 少一步 / 换 DID 四种篡改方案全部被校验器拒绝；全链路逐字节可重放：规划结论、图状态、版本摘要、操作计数四项两次运行完全相同；索引结果与朴素全扫（含自有能力）逐条相同；陈旧广播进不了规划：重放 v1 被判 stale_epoch，规划仍用 v2 的价格；I1–I7 在 8 个种子 × 60 步随机操作序列的每一步后都成立；无路径与放宽后有路径一致：预算 1 → policy_denied；期限 1ms → timeout；去约束后可行；对抗 11 例：拒绝码正确、且被拒之后图状态逐字节不变；scenario 输出包含 8 个版本的证据字段（指针检查）
- **证据**：`cargo test -p au4a-capgraph (CARGO_INCREMENTAL=0)` → 151/151，等级 verified（本机实测）（本机 Windows 实测：lib 57 + adversarial 11 + broadcast 10 + cache 9 + data_structures 7 + declaration 8 + end_to_end 9 + invariants 5 + perf 9 + planner 9 + query_index 8 + versioning 9 = 151，0 failed，0 warning（含依赖）。）
- **状态**：✅ 已交付

#### v1.1.10 文档与证据汇总

- **目标**：10 个版本的目标/接口/验收/证据齐备，results_json 汇总可被节点聚合
- **交付物**：`src/lib.rs: pub const VERSIONS[10] + schema_json()（机器可读 self-description）+ results_json()（证据索引）+ checks_v110`、`tests/end_to_end.rs: self_check_and_results_json_are_fully_exercised（跑完整棵自检树）`、`docs/tracks/1.1.md: 10 版小节 + 轨道总结 + 复现命令 + 证据分级`、`docs/tracks/1.1.json: 10 版元数据齐全`
- **接口**：`pub const VERSIONS: [&str; 10]`、`pub fn schema_json() -> CoreResult<serde_json::Value>`、`pub fn results_json() -> CoreResult<serde_json::Value>`、`pub fn self_check() -> Vec<au4a_core::SelfCheck>`
- **验收**：self_check() 返回 >=25 条自检且全部 passed、detail 非空、all_passed() 为真；results_json()['versions'] 与 VERSIONS 常量逐项相同；schema_json() 含 10 个能力字段、3 种消息类型、floats=false，可被规范 JSON 编码；results_json() 内嵌 schema、模块表与证据摘要；v1.1.10 自检树暴露的 3 个真实缺陷（整数性判据/历史版本号/缓存作废断言）已修复并有回归测试
- **证据**：`cargo test -p au4a-capgraph (CARGO_INCREMENTAL=0)` → 152/152，等级 verified（本机实测）（本机 Windows 实测：lib 57 + adversarial 11 + broadcast 10 + cache 9 + data_structures 7 + declaration 8 + end_to_end 10 + invariants 5 + perf 9 + planner 9 + query_index 8 + versioning 9 = 152，0 failed，0 warning（含依赖）。）
- **状态**：✅ 已交付

### v1.2 Negotiation 协商协议（`au4a-negotiate`）

#### v1.2.1 消息类型

- **目标**：六种协商消息（REQUEST/COUNTER/ACCEPT/REJECT/SIGN/BREACH）成为先签名后发送的 PMB 信封
- **交付物**：`src/msg.rs: kinds 六个 PMB 类型名 / ALL_KINDS / Terms / BreachKind / NegotiationMsg / session_id`、`src/lib.rs: self_check 四条真实断言 + results_json + scenario(request+counter 真实投递)`、`tests/messages.rs: 六种消息经内核投递的集成测试`
- **接口**：`pub const kinds::NEGOTIATE_REQUEST/COUNTER/ACCEPT/REJECT/CONTRACT_SIGN/CONTRACT_BREACH`、`pub enum NegotiationMsg { Request, Counter, Accept, Reject, SignContract, Breach }`、`pub struct Terms { task, price, deadline, evidence }`、`pub fn session_id(&Did, &Did, &Terms) -> CoreResult<String>`、`NegotiationMsg::signed(&AgentKeys, &Did, u64, Option<String>) -> CoreResult<Envelope>`、`NegotiationMsg::from_env(&Envelope) -> CoreResult<NegotiationMsg>`
- **验收**：六个类型名均为合法 MsgKind 且互不相同；六种消息 构造→签名→验签→解析 逐字段相等；信封 kind 与消息体 type 不一致 → invalid_kind；篡改 body → invalid_signature；未封口 → not_sealed；伪造信封在内核记为 unauthorized（恶意码）；条款哈希对同一语义稳定、对价格敏感；price<=0/非法标签/非 hex 哈希/超长文本被拒；session_id 绑定（提议方,应答方,首轮条款）；scenario 同内核重复调用与跨内核同种子结果相同
- **证据**：`cargo test -p au4a-negotiate` → 14/14，等级 cpu-proto（语义原型）（本机 Windows 实测 exit=0：8 个单元测试 + 6 个集成测试全绿、0 warning（日志 E:\DS\_forangent\logs\1.2-v1.2.1.log）。真实的部分：Ed25519 签名/验签、规范 JSON 哈希、PMB 分帧、内核投递与恶意拒绝记录全部真实执行。原型部分：传输是同进程内存队列，非真实 libp2p 网络。）
- **状态**：✅ 已交付

#### v1.2.2 状态机

- **目标**：IDLE→NEGOTIATING→ACCEPTED→CONTRACT_SIGNED→EXECUTING→SETTLED 与违约进入 ARBITRATION，每次转换双方签名
- **交付物**：`src/state.rs: Phase/Event/transition/TransitionRecord 双签`
- **接口**：`pub enum Phase { Idle, Negotiating, Accepted, ContractSigned, Executing, Settled, Arbitration }`、`pub fn transition(Phase, Event) -> CoreResult<Phase>`、`pub struct TransitionRecord + co_sign/verify`
- **验收**：穷举全部 (Phase, Event) 组合，合法的恰好是设计表内的那些；单方签名的转换记录 verify 失败；非法转换返回 Err 而不是 panic
- **证据**：`cargo test -p au4a-negotiate` → 30/30，等级 cpu-proto（语义原型）（实测 exit=0：19 单元 + 6 tests/messages.rs + 5 tests/state_machine.rs 全绿、0 warning（日志 1.2-v1.2.2.log）。穷举 63 种 (相位,事件)：10 合法 / 53 非法。首轮 5 个测试失败暴露 commit 双重应用缺陷，修复后复跑全绿。真实：Ed25519 双签、签名覆盖全字段；原型：传输为同进程内存队列。）
- **状态**：✅ 已交付

#### v1.2.3 持久化

- **目标**：协商记录导出为规范 JSON 快照并可重放，重放后逐字节一致（纯内存，无文件 I/O）
- **交付物**：`src/journal.rs: Journal（唯一事实来源=已签名信封+双方签署的转换记录）`、`src/state.rs: StateMachine::rebuild（journal 重放路径）`、`tests/journal.rs: 归档往返/篡改/乱序集成测试`
- **接口**：`pub struct Journal`、`Journal::open(&str,&[Did]) -> CoreResult<Journal>`、`Journal::append(&Envelope) / append_transition(&TransitionRecord, Option<&dyn DualSigned>)`、`Journal::encode(&self) -> CoreResult<String>（规范 JSON，纯内存）`、`Journal::decode(&str) -> CoreResult<Journal>`、`Journal::replay(&self) -> CoreResult<StateMachine> / replay_digest() -> CoreResult<String>`、`StateMachine::rebuild(&str,&[Did],&[TransitionRecord]) -> CoreResult<StateMachine>`
- **验收**：encode→decode→encode 逐字节相同（定点）；重放摘要与重放出的状态机与实时状态完全相同；截断/多字段/错版本/乱序/篡改价格或签名/跨会话消息/重复消息全部被拒；单签转换不能归档；归档路径与实时路径用同一把尺子；归档文本里不存在可被直接篡改的 phase 冗余字段
- **证据**：`cargo test -p au4a-negotiate` → 40/40，等级 cpu-proto（语义原型）（实测 exit=0：25 单元 + 4 tests/journal.rs + 6 tests/messages.rs + 5 tests/state_machine.rs 全绿、0 warning（日志 1.2-v1.2.3.log）。encode→decode→encode 定点、重放摘要一致、版本篡改→invalid_version、条款篡改→invalid_signature、乱序→invalid_kind。纯内存规范 JSON，无文件 I/O。）
- **状态**：✅ 已交付

#### v1.2.4 多轮协商

- **目标**：多轮报价/还价引擎：轮数上限、REJECT 不占额度、超限走类型化拒绝
- **交付物**：`src/rounds.rs: Negotiation/Offer/Rejection（消息+状态机+归档三合一）`、`src/lib.rs: scenario 改用协商引擎真跑，self_check 增 rounds.cap / rounds.reject_free`、`tests/rounds.rs: 多轮经 PMB 真跑 + 上限/轮转拒绝测试`
- **接口**：`pub struct Negotiation`、`Negotiation::open(&mut Kernel,&AgentKeys,&AgentKeys,Terms,u32) -> CoreResult<Negotiation>`、`Negotiation::counter/accept/reject/archive/summary/price_trail`、`pub const DEFAULT_MAX_ROUNDS: u32 = 6`
- **验收**：max_rounds 轮还价后第 max_rounds+1 次 → Err(Overflow) + 内核记 PolicyDenied（非恶意码），轮数/报价数/归档字节均不变；REJECT 任意次都不消耗轮数额度，但每次都留下双方签名的自环记录；还价必须来自上一次报价的对端（自问自答 → Err + Conflict）；不能接受自己的报价；七条消息全部经内核投递、逐条验签、in_reply_to 引用链连续；同种子两次独立协商的归档逐字节相同
- **证据**：`cargo test -p au4a-negotiate` → 51/51，等级 cpu-proto（语义原型）（实测 exit=0：32 单元 + 4 tests/journal.rs + 4 tests/rounds.rs + 6 tests/messages.rs + 5 tests/state_machine.rs 全绿、0 warning（日志 1.2-v1.2.4.log）。行为：150→140→130→(拒绝)→125→120→接受；上限=1 时第 2 次还价 Err(Overflow)+policy_denied+escalation=None；5 次连续 REJECT 后轮数仍为 0。首轮 3 个测试失败（轮转规则的真实交互），修正后复跑全绿。）
- **状态**：✅ 已交付

#### v1.2.5 合约签订

- **目标**：合约哈希 = 规范 JSON 的 SHA-256，双方签名才成立，可锚定
- **交付物**：`src/contract.rs: Contract/ContractSignature/Anchor`
- **接口**：`pub struct Contract { id, hash, proposer, responder, terms, negotiation, created_at, signatures, anchor }`、`Contract::draft/sign/verify/compute_hash/encode/decode/anchor/verify_anchor/is_dual_signed`、`pub struct Anchor + Anchor::verify`、`impl DualSigned for Contract`、`Negotiation::sign_contract(&mut Kernel,&AgentKeys,&AgentKeys) -> CoreResult<Contract>`
- **验收**：合约哈希 = canonical_hash(规范 JSON 条款)，64 位小写 hex；条款/会话/时间任一变化哈希全变；单方签名 → not_sealed；重复签名 → invalid_signature；第三方签名 → unknown_agent；签署后改价格/期限/哈希/编号/签名/当事人 → invalid_signature；合约 encode→decode 逐字节定点，decode 会重验哈希与双方签名（篡改文本被拒）；只有已签署的当事人能锚定；锚点被改 at/哈希 → invalid_signature；签合约需 ACCEPTED 相位；双方各发一条 CONTRACT_SIGN，锚点事件进只读进度流
- **证据**：`cargo test -p au4a-negotiate` → 65/65，等级 cpu-proto（语义原型）（实测 exit=0：40 单元 + 6 tests/contract.rs + 4 tests/journal.rs + 6 tests/messages.rs + 4 tests/rounds.rs + 5 tests/state_machine.rs 全绿、0 warning（日志 1.2-v1.2.5.log）。哈希用冻结基元 canonical_hash（SHA-256 of AU4A-CJ），锚点写入 kernel 进度流并可离线复核。修复记录：一处测试助手用 Terms::new(0) 触发 ZeroAmount 导致 unwrap panic，改为直接构造零价条款后复跑全绿。）
- **状态**：✅ 已交付

#### v1.2.6 违约处理

- **目标**：CONTRACT_BREACH 申诉：四类违约、证据等级、状态机进入 ARBITRATION
- **交付物**：`src/breach.rs: BreachClaim（挂双签合约的签名申诉）`、`src/rounds.rs: Negotiation::execute（CONTRACT_SIGNED→EXECUTING）与 report_breach（→ARBITRATION，条款授权）`、`tests/breach.rs: 违约进仲裁的集成测试`
- **接口**：`pub struct BreachClaim { contract_id, contract_hash, claimant, accused, kind, evidence, note, at, sig }`、`BreachClaim::file(&AgentKeys,&Contract,BreachKind,EvidenceGrade,&str,u64) -> CoreResult<BreachClaim>`、`BreachClaim::verify/is_against/accused_is_counterparty/summary`、`Negotiation::execute(&mut Kernel,&AgentKeys,&AgentKeys) -> CoreResult<TransitionRecord>`、`Negotiation::report_breach(&mut Kernel,&AgentKeys,BreachKind,EvidenceGrade,&str) -> CoreResult<BreachClaim>`
- **验收**：四类违约（non_delivery/late_delivery/under_delivery/wrong_evidence）都可立案，载荷往返一致；无合约或未达 CONTRACT_SIGNED/EXECUTING 的申诉 → invalid_kind，且不多发一条消息；非当事人申诉 → unknown_agent；单方签署合约 → not_sealed；条款被改的合约 → invalid_signature；申诉方=被诉方 → invalid_kind；被诉方必须是合约另一方；改 kind/合约哈希/note 任一字段 → invalid_signature；被诉方代签无效；进入 ARBITRATION 的转换记录恰有 1 个当场签名 + 双签合约条款授权；重放与归档往返仍成立；证据等级如实记录：unverified 的申诉不会绕过结算闸门
- **证据**：`cargo test -p au4a-negotiate` → 77/77，等级 cpu-proto（语义原型）（实测 exit=0：45 单元 + 7 tests/breach.rs + 6 tests/contract.rs + 4 tests/journal.rs + 6 tests/messages.rs + 4 tests/rounds.rs + 5 tests/state_machine.rs 全绿、本 crate 0 warning（日志 1.2-v1.2.6.log；该次构建中 au4a-kernel 有 1 条 unused variable 警告，属 track-kernel 正在改的版本，不是本 crate）。修复记录：首轮 1 个测试断言写错（忘了先清空在途的开局报价消息），修正后全绿。）
- **状态**：✅ 已交付

#### v1.2.7 仲裁接入

- **目标**：仲裁案件立案→证据→裁决，罚没与赔付落到内核账本，守恒不变式保持
- **交付物**：`src/arbitration.rs: ArbitrationCase/Verdict/Ruling/ArbitrationPolicy/Enforcement`、`src/rounds.rs: Negotiation::{settle,open_case,case_mut,resolve}（EXECUTING→SETTLED、ARBITRATION→SETTLED）`、`tests/arbitration.rs: 完整仲裁链路 + 证据闸门 + 结算角色约束`
- **接口**：`pub struct ArbitrationCase { case_id, contract_id, contract_hash, claimant, accused, claim, filed_at, arbiters, ruling }`、`ArbitrationCase::{file,rule,verify_ruling,enforce,summary}`、`pub struct Ruling { case_id, verdict, slash, compensate, rationale, at, signatures }`、`pub struct ArbitrationPolicy { slash_bp, compensate_bp } + assess()`、`pub struct Enforcement { case_id, verdict, slashed, compensated, conservation_ok, at }`、`Negotiation::{settle,open_case,case,case_mut,resolve}`
- **验收**：case_id 内容寻址（合约哈希 + 申诉载荷），不同申诉得不同案件；当事人不能当仲裁员（unknown_agent）；两名仲裁员必须互不相同（invalid_kind）；只有本案两名仲裁员共同签名裁决才生效：单签 → not_sealed，名单外 → unknown_agent，改金额 → invalid_signature；unverified 申诉按政策 Rejected 且金额为 0；执行不动账本；罚没走 ledger.slash（销毁，发行量不变）、赔付走 kernel.settle（过证据闸门）；证据闸门拦下超限 cpu-proto 赔付时整案不执行（账本完全不变）并记 policy_denied；执行前后 check_conservation 均通过；结算必须按合约角色（反向付款 → conflict）
- **证据**：`cargo test -p au4a-negotiate` → 93/93，等级 cpu-proto（语义原型）（实测 exit=0：53 单元 + 8 tests/arbitration.rs + 7 tests/breach.rs + 6 tests/contract.rs + 4 tests/journal.rs + 6 tests/messages.rs + 4 tests/rounds.rs + 5 tests/state_machine.rs 全绿、0 warning（日志 1.2-v1.2.7.log）。实测数值：100 微积分合约 → 罚没 20（锁定 50→30，slashed +20，minted 不变）+ 赔付 50（可用 950→900，申诉方 +50）；CPU 原型 1000 合约的 500 赔付被证据闸门拒绝且账本零变化。修复记录：3 处测试借用冲突（enforce(&mut k, k.tick())）与 2 处余额期望值写错，修正后全绿。）
- **状态**：✅ 已交付

#### v1.2.8 测试

- **目标**：综合集成测试：正常/拒绝/不变式三条线覆盖全轨道能力
- **交付物**：`tests/negotiate.rs: 正常路径 ×2 / 拒绝矩阵 ×2 / 不变式 ×2 / 源码守卫 ×2`、`（无新公共 API，只加断言）`
- **接口**：`（测试文件，无新公共 API）`
- **验收**：正常路径：多轮→达成→双签→执行→结算，账本逐账户数值断言；拒绝矩阵：非法转换/超轮数/自问自答/第三方/无合约违约/当事人仲裁/单仲裁员，全部有类型化 Err 且相位不推进；不变式：63 种转换穷举（10 合法）、每条记录双签或条款授权、账本守恒、归档定点、同种子逐字节一致；源码守卫：读 src/*.rs 并断言库代码无 unwrap/expect/panic!/todo!/unimplemented!；宿主输入（空串/非 hex/超长/越界轮次/垃圾归档）只返回 Err，不 panic
- **证据**：`cargo test -p au4a-negotiate` → 101/101，等级 cpu-proto（语义原型）（实测 exit=0：53 单元 + 8 tests/negotiate.rs + 8 tests/arbitration.rs + 7 tests/breach.rs + 6 tests/contract.rs + 4 tests/journal.rs + 6 tests/messages.rs + 4 tests/rounds.rs + 5 tests/state_machine.rs 全绿、0 warning（日志 1.2-v1.2.8.log）。其中 library_code_never_panics 真实读取 8 个 src 模块并断言库代码零 panic 出口。修复记录：1 处余额期望值算错（客户可用 950-135=815），修正后全绿。）
- **状态**：✅ 已交付

#### v1.2.9 文档

- **目标**：docs/tracks/1.2.md 与 1.2.json 覆盖 10 个小版本的目标/交付物/接口/验收/证据/状态
- **交付物**：`docs/tracks/1.2.md: 轨道总览（能力→代码→证据等级、验证命令、设计决定、已知边界）+ 10 版逐版记录`、`docs/tracks/1.2.json: 10 版机器可读元数据（goal/deliverables/interfaces/acceptance/evidence/status）`
- **接口**：`（文档，无新公共 API）`
- **验收**：10 个小版本恰好一节，顺序与 brief 一致；每条证据都是真实运行结果（命令 + 通过数 + 日志路径）；总览表逐条标注真实/原型，并列出已知边界（内存传输、非链上锚定、仲裁员无质押抽签）；错误映射表说明 CoreError 冻结枚举下的语义映射
- **证据**：`cargo test -p au4a-negotiate` → 101/101，等级 cpu-proto（语义原型）（文档版：本版不改代码，重跑同一套测试确认 exit=0、101 passed / 0 failed / 0 warning（日志 1.2-v1.2.9.log）。docs/tracks/1.2.md 新增轨道总览（能力→代码→证据等级表、可复制验证命令、5 条设计决定、5 条已知边界），并补齐 10 个小版本的逐版记录。）
- **状态**：✅ 已交付

#### v1.2.10 示例

- **目标**：scenario 真跑两条路径：多轮→双签→执行→结算；违约→仲裁，返回两份 JSON 摘要
- **交付物**：`src/example.rs: success_path / breach_path / run / example_dids`、`src/lib.rs: scenario 改为组合两条链路（顶层保留成功链路兼容字段）`、`tests/example.rs: 两条链路逐项数值 + 可重复性 + 不吞错`
- **接口**：`pub fn success_path(&mut Kernel) -> CoreResult<Value>`、`pub fn breach_path(&mut Kernel) -> CoreResult<Value>`、`pub fn run(&mut Kernel) -> CoreResult<Value>（{"success":…,"breach":…}）`、`pub fn example_dids() -> [Did; 4]`
- **验收**：成功链路：120→100→(拒绝)→95→接受→双签合约→执行→结算 95，七条消息全部验签，账本 885/1075；违约链路：80→75→接受→双签→执行→违约申诉→立案→双签裁决（罚没 15 / 赔付 37）→结案 SETTLED；scenario 返回两份 JSON 摘要（success/breach）+ 顶层兼容字段，paths=2、steps=8；两条链路同种子可重复：全新内核逐字段相同；同内核重跑仅逻辑时钟导致的 id/ts 变化；示例不吞错：非法动作照样 Err（未接受就签 → invalid_kind；超轮数 → overflow + policy_denied）；六个 Agent 自主注册（无人类账户），观察层可见两条链路事件，账本守恒
- **证据**：`cargo test -p au4a-negotiate` → 111/111，等级 cpu-proto（语义原型）（实测 exit=0：57 单元 + 8 tests/arbitration.rs + 7 tests/breach.rs + 6 tests/contract.rs + 6 tests/example.rs + 4 tests/journal.rs + 6 tests/messages.rs + 8 tests/negotiate.rs + 4 tests/rounds.rs + 5 tests/state_machine.rs 全绿、0 warning（日志 1.2-v1.2.10.log）。两条链路实测：成功链路结算 95（客户可用 1000−20−95=885，服务方 1075）；违约链路案件裁决 upheld（罚没 15 = 75 的 2000bp、赔付 37 = 5000bp 向下取整），账本守恒、发行量不变。修复记录：scenario 重构后 scenario_agents 一度成为死代码（1 条 warning），改为成功链路复用它后 0 warning。）
- **状态**：✅ 已交付

### v1.3 Portable State 可移植状态（`au4a-state`）

#### v1.3.1 状态快照

- **目标**：把 Agent 的三区（文件系统/内存/上下文）状态冻结成内容寻址的块集合，并落到可替换的状态存储上。
- **交付物**：`src/snapshot.rs: StateZone/StateBlock/StateSnapshot（capture/verify/content_root/zone_root/rebase/JSON 往返）`、`src/store.rs: StateStore trait（put/get/list/remove/len）+ MemoryStore + 命名空间化 write_snapshot/read_snapshot`、`src/lib.rs: self_check/results_json/scenario 真实链路`、`tests/v131_snapshot.rs: 13 个集成测试`
- **接口**：`pub enum StateZone { Fs, Memory, Context }`、`pub fn StateBlock::new(zone, key, value) -> CoreResult<StateBlock>`、`pub fn StateSnapshot::capture(agent: &Did, node: &str, epoch: u64, blocks: Vec<StateBlock>) -> CoreResult<StateSnapshot>`、`pub fn StateSnapshot::root(&self) -> &str`、`pub fn StateSnapshot::content_root(&self) -> CoreResult<String>`、`pub fn StateSnapshot::verify(&self) -> CoreResult<()>`、`pub fn StateSnapshot::rebase(&self, node: &str, epoch: u64) -> CoreResult<StateSnapshot>`、`pub trait StateStore { put/get/list/remove/len }`、`pub fn write_snapshot(store, prefix, snap) -> CoreResult<usize>`、`pub fn read_snapshot(store, prefix, agent, node, epoch) -> CoreResult<StateSnapshot>`
- **验收**：正序/逆序捕获同一批块得到同一 root，verify() 通过；三区各有块且 zone_root 互不相等；空区也有稳定 root；同出处写库读回 root 与逐块内容相等；换节点读回 content_root 相等而 root 不同；篡改块 value 保留旧 root 被拒；整份快照替换被拒（InvalidSignature）；浮点/超长 key/重复 (zone,key)/超 4096 块/损坏存储键 全部返回 typed Err；scenario() 两次独立运行 JSON 完全相同（确定性重放）
- **证据**：`cargo test -p au4a-state` → 31/31，等级 verified（本机实测）（本机 Windows 实测（lib 18 + integration 13）；纯 CPU 逻辑，不读文件、不开网络、不读墙钟。日志 E:\DS\_forangent\logs\1.3-v1.3.1.log）
- **状态**：✅ 已交付

#### v1.3.2 跨节点传输

- **目标**：块级增量 diff（三区 set/del）+ 可切块、可续跑的双节点内存传输。
- **交付物**：`src/diff.rs: DeltaOp/DelOp/StateDelta（between/validate/apply_to/apply_moved/chunked/id）+ DeltaChunk`、`src/transfer.rs: NodeId/帧编解码/LocalNetwork（本地双节点内存 + 断线注入）/TransferSession/send_chunks/pull_into_session`、`tests/v132_transfer.rs: 11 个集成测试`
- **接口**：`pub fn StateDelta::between(from, to) -> CoreResult<StateDelta>`、`pub fn StateDelta::apply_to(&self, base) -> CoreResult<StateSnapshot>`、`pub fn StateDelta::apply_moved(&self, base, node, epoch) -> CoreResult<StateSnapshot>`、`pub fn StateDelta::chunked(&self, max_ops) -> CoreResult<Vec<DeltaChunk>>`、`pub fn send_chunks(net, from, to, delta, max_ops, start, count) -> CoreResult<TransferReport>`、`pub fn pull_into_session(net, node, session) -> CoreResult<(TransferSession, usize, usize)>`、`pub fn TransferSession::accept(&mut self, chunk) -> CoreResult<AcceptOutcome>`、`pub fn TransferSession::assemble(&self, total, agent) -> CoreResult<StateDelta>`、`pub fn TransferSession::resume_from(&self) -> usize`
- **验收**：三区（fs/memory/context）均有 set 与 del；应用 diff 后 content_root 与目标一致；base 不对时拒绝（InvalidSignature / StaleEpoch 语义）；断线注入后只补发缺失块，重新拼装内容根一致；重复块幂等；同 index 不同内容被拒；跨会话块被拒；帧截断/超 1 MiB 在分配前被拒
- **证据**：`cargo test -p au4a-state` → 59/59，等级 cpu-proto（语义原型）（lib 35 + v131 13 + v132 11 全绿 0 warning。跨节点传输是本地双节点内存实现（cpu-proto）：协议与失败注入真实可重放，但没有真实网络/丢包/TLS；diff 与内容寻址为 verified。日志 E:\DS\_forangent\logs\1.3-v1.3.2.log）
- **状态**：✅ 已交付

#### v1.3.3 签名验证

- **目标**：快照内容寻址 + Ed25519 签名，篡改或替换必须被拒（签名有效 ≠ 是我要的那份）。
- **交付物**：`src/signed.rs: SignedSnapshot（sign/verify/verify_policy/帧往返）+ SnapshotPolicy + VerifiedSnapshot`、`tests/v133_signature.rs: 16 个集成测试`
- **接口**：`pub fn SignedSnapshot::sign(snapshot, keys: &AgentKeys) -> CoreResult<SignedSnapshot>`、`pub fn SignedSnapshot::verify(&self) -> CoreResult<()>`、`pub fn SignedSnapshot::verify_policy(&self, policy: &SnapshotPolicy) -> CoreResult<VerifiedSnapshot>`、`pub fn SignedSnapshot::to_frame(&self) -> CoreResult<Vec<u8>>`、`pub struct SnapshotPolicy { agent, content_root: Option<String>, min_epoch: Option<u64> }`、`pub struct VerifiedSnapshot（私有字段，唯一构造路径 verify_policy）`
- **验收**：自签快照验签通过；帧/JSON 往返签名不变；同种子同状态签名逐字节相同；改块内容/改签名/空签名/错版本/伪造 signer 全部被拒；另一份合法签名的快照无法通过 content_root 策略（替换/重放被拒）；过期快照返回 InvalidVersion 但验签仍通过（竞争与恶意分开）；替别人签被拒；冒名状态被拒（InvalidDid）；scenario 中 verified.content_root == 目标重建的 content_root
- **证据**：`cargo test -p au4a-state` → 84/84，等级 verified（本机实测）（lib 44 + v131 13 + v132 11 + v133 16 全绿 0 warning；真实 Ed25519 签名/验签（算法来自冻结基元 au4a-core）。日志 E:\DS\_forangent\logs\1.3-v1.3.3.log）
- **状态**：✅ 已交付

#### v1.3.4 恢复协议

- **目标**：两阶段提交 prepare → commit → confirm，任意阶段失败回滚，目标节点不留部分状态。
- **交付物**：`src/recovery.rs: NodeStore（影子代 + 单点 head 切换 + 孤儿代检测）`、`src/recovery.rs: Migration（prepare/commit/confirm/rollback + 阶段守卫）、MigrationPlan（tx 内容寻址）`、`src/recovery.rs: FaultInjector/FaultPoint（5 个定点注入）、migrate()、MigrationOutcome`、`tests/v134_two_phase.rs: 13 个集成测试`
- **接口**：`pub fn NodeStore::open(node, agent, store) -> CoreResult<NodeStore<S>>`、`pub fn NodeStore::write_generation(&mut self, gen, snapshot) -> CoreResult<usize>`、`pub fn NodeStore::live_content_root(&self) -> CoreResult<String>`、`pub fn Migration::prepare(&mut self, node, delta, signed, faults) -> CoreResult<PrepareReceipt>`、`pub fn Migration::commit(&mut self, node, faults) -> CoreResult<CommitReceipt>`、`pub fn Migration::confirm(&mut self, node, faults) -> CoreResult<ConfirmReceipt>`、`pub fn Migration::rollback(&mut self, node) -> CoreResult<usize>`、`pub fn migrate(node, plan, delta, signed, faults) -> CoreResult<MigrationOutcome>`、`pub enum FaultPoint { BeforeStaging, AfterStaging, BeforeFlip, AfterFlip, BeforeCleanup }`
- **验收**：prepare 之后 live 仍是 base；commit 后才切换；confirm 清理旧代；5 个注入点逐个注入故障 → 全部回滚：state_restored=true、orphans=0、意图已清；任意时刻 live 的块集合整体等于 base 或整体等于 target（无混合状态）；阶段守卫拒绝乱序/重复调用；已 confirm 的迁移拒绝回滚；写生效代被拒；被拒的迁移不动 head、不留孤儿代；AfterFlip 故障时 rollback 能把 head 拨回并恢复 base；同输入迁移可重放（同样 outcome JSON）
- **证据**：`cargo test -p au4a-state` → 106/106，等级 verified（本机实测）（lib 53 + v131 13 + v132 11 + v133 16 + v134 13 全绿 0 warning。2PC 协议与 5 个故障注入点全部本机实跑；存储是本地内存 StateStore 实现，换真实存储只需换实现。日志 E:\DS\_forangent\logs\1.3-v1.3.4.log）
- **状态**：✅ 已交付

#### v1.3.5 一致性检查

- **目标**：三区摘要 + 块级差异报告，定位恢复后的缺失/多余/被改块。
- **交付物**：`src/integrity.rs: Finding/FindingCode（8 类）/ZoneReport/ConsistencyReport/StoreReport/NodeAudit`、`src/integrity.rs: compare（快照 vs 快照）、compare_store（存储原始字节 vs 期望）、audit_node（2PC 卫生）`、`tests/v135_consistency.rs: 11 个集成测试`
- **接口**：`pub fn compare(left, right) -> CoreResult<ConsistencyReport>`、`pub fn compare_store(store, prefix, expected) -> CoreResult<StoreReport>`、`pub fn audit_node(node) -> CoreResult<NodeAudit>`、`pub enum FindingCode { Missing, Extra, Modified, BadKey, Unreadable, OrphanGeneration, UnfinishedMigration, IntentMismatch }`、`pub struct ZoneReport { zone, left_root, right_root, equal, missing, extra, modified }`、`pub fn summarize(&ConsistencyReport) -> String`
- **验收**：内容一致（出处不同）时报告干净且三区 zone_root 两两相等；改/删/加/坏键各被定位到 1 处，zone 与 key 可读；compare_store 从存储原始字节发现篡改并给出观测内容根；只改 memory 一块时 fs/context 区摘要保持不变（诊断粒度是区）；2PC 提交后源与 live 逐块相等；回滚后与 base 相等、与 target 不等；未完成迁移在节点审计中可见（unfinished_migration + intent_mismatch）
- **证据**：`cargo test -p au4a-state` → 123/123，等级 verified（本机实测）（lib 59 + v131 13 + v132 11 + v133 16 + v134 13 + v135 11 全绿 0 warning。日志 E:\DS\_forangent\logs\1.3-v1.3.5.log）
- **状态**：✅ 已交付

#### v1.3.6 UDOS集成

- **目标**：与 E:\DS\UDOS 的数据契约（分布式存储语义，不引入依赖、不联网）。
- **交付物**：`src/udos.rs: UdosObject（schema/kind/object_id/collection/grade/provenance/payload）`、`src/udos.rs: snapshot_to_object/object_to_snapshot/delta_to_object/object_to_delta`、`src/udos.rs: context_to_transfer_bundle/transfer_bundle_to_blocks/bundle_fingerprint/claim`、`src/udos.rs: contract_descriptor/contract_check/collection_manifest/delta_shape`、`tests/v136_udos.rs: 14 个集成测试`
- **接口**：`pub const SCHEMA: &str = "udos.object/1"`、`pub fn digest_of(value) -> CoreResult<String>  // sha256:<hex>`、`pub fn snapshot_to_object(&StateSnapshot) -> CoreResult<Value>`、`pub fn object_to_snapshot(&Value) -> CoreResult<StateSnapshot>`、`pub fn context_to_transfer_bundle(&StateSnapshot) -> CoreResult<Value>`、`pub fn transfer_bundle_to_blocks(&Value) -> CoreResult<Vec<StateBlock>>`、`pub fn claim(key, statement, grade, artifact_digest) -> CoreResult<Value>`、`pub fn contract_descriptor() -> Value`
- **验收**：对象只靠 payload 即可复算 object_id（独立验证者视角）；还原后内容根一致；改 payload/object_id → InvalidSignature；换 schema → InvalidVersion；未知 kind/grade → 拒绝；伪造 provenance.agent → 导入时 InvalidDid；delta 导出导入 id 相同且可重建目标内容根；分块导出每块自校验；TransferBundle 六字段齐备；缺失被拒；指纹对键序不敏感；字符串 goal 往返无损；claim 只接受 sha256: 前缀摘要；证据等级字符串与 UDOS 完全一致
- **证据**：`cargo test -p au4a-state` → 148/148，等级 verified（本机实测）（lib 70 + v131 13 + v132 11 + v133 16 + v134 13 + v135 11 + v136 14 全绿 0 warning。纯数据契约：无 UDOS 依赖、不联网、不调用 Python；UDOS 侧对齐点只读核对（evidence.py/contracts.py/topology/transfer.py）。日志 E:\DS\_forangent\logs\1.3-v1.3.6.log）
- **状态**：✅ 已交付

#### v1.3.7 性能优化

- **目标**：块级哈希复用 + 只物化变动块 + 增量续跑，用确定性计数器量化省下的工作量（不读墙钟）。
- **交付物**：`src/perf.rs: WorkCounter（8 个确定性计数器，无时间字段）/DigestCache/DeltaPlan/plan/materialize`、`src/perf.rs: resume_savings/measure_transfer/measure_receive/generation_write_estimate/summary`、`src/snapshot.rs: StateBlock::new_cached（仅缓存路径可用，crate 内可见）`、`tests/v137_perf.rs: 10 个集成测试`
- **接口**：`pub struct WorkCounter { block_hashes, hashes_reused, canon_encodings, root_hashes, blocks_materialized, store_writes, ops_transferred, ops_skipped }`、`pub fn WorkCounter::reuse_ratio_bp(&self) -> i64`、`pub fn DigestCache::capture_raw(agent, node, epoch, blocks, counter) -> CoreResult<StateSnapshot>`、`pub fn plan(from, to) -> CoreResult<DeltaPlan>`、`pub fn materialize(from, to, plan, counter) -> CoreResult<StateDelta>`、`pub fn resume_savings(total_ops, resumed_from_chunks, chunk_ops) -> (u64, u64)`、`pub fn measure_transfer(...) / measure_receive(...)`
- **验收**：同一状态二次捕获 0 次哈希（复用率 10000bp），root 与逐块内容不变；改一块只重算 1 次；同键不同值不会误命中缓存；plan 正确指出改/删/未变块；materialize 只物化变动块；续跑 ops_skipped > 0 且 ops_transferred < 全量，重装后内容根一致；计数器可重放；summary 明确 wall_clock_used=false；影子代写入如实标注完整物化（正确性优先于省写）
- **证据**：`cargo test -p au4a-state` → 165/165，等级 verified（本机实测）（lib 77 + v131 13 + v132 11 + v133 16 + v134 13 + v135 11 + v136 14 + v137 10 全绿 0 warning。全部为确定性计数，未使用墙钟/计时器。日志 E:\DS\_forangent\logs\1.3-v1.3.7.log）
- **状态**：✅ 已交付

#### v1.3.8 测试

- **目标**：全链路集成（run_chain）+ 篡改矩阵 + 故障矩阵 + 边界与拒绝路径。
- **交付物**：`src/chain.rs: run_chain/ChainOptions/ChainReport/ConsistencyReportView（产品代码，灾难演练复用）`、`tests/v138_chain.rs: 16 个集成测试（篡改/故障/边界三个矩阵）`
- **接口**：`pub fn run_chain(agent, base, target, to_node, options) -> CoreResult<ChainReport>`、`pub struct ChainOptions { chunk_ops, first_batch_chunks, drop_rest, fault, epoch }`、`pub struct ChainReport { base/source/live_content_root, identical, signed_and_verified, chunks, dropped_frames, resumed_from, delta_ops, committed, rollback_clean, consistency, work, delta_plan, udos_* }`、`pub fn ChainReport::is_ok(&self) -> bool`
- **验收**：6 块状态逐块篡改全部被检出；逐个存储键改/删恰好定位 1 处；每个差异块被篡改都被拒；改签名/改 object_id 被拒；5 个注入点全部回滚且 live==base、一致性干净；空↔单块双向迁移、零差异链路、Unicode/控制字符、超限输入全部按预期；链路报告逐字节可重放；跨 Agent 与 chunk_ops=0 被拒
- **证据**：`cargo test -p au4a-state` → 186/186，等级 verified（本机实测）（lib 82 + v131 13 + v132 11 + v133 16 + v134 13 + v135 11 + v136 14 + v137 10 + v138 16 全绿 0 warning。日志 E:\DS\_forangent\logs\1.3-v1.3.8.log）
- **状态**：✅ 已交付

#### v1.3.9 文档

- **目标**：能力清单在代码里可枚举、可测试，文档每条声明都指向实现与测试。
- **交付物**：`src/capabilities.rs: Capability/CAPABILITIES（19 条）/capabilities()/coverage_check()/manifest()/declared_shape_ok()`、`tests/v139_capabilities.rs: 9 个集成测试`、`docs/tracks/1.3.md: 能力清单表格（与 CAPABILITIES 一一对应）`
- **接口**：`pub const CAPABILITIES: [Capability; 19]`、`pub fn capabilities() -> &'static [Capability]`、`pub fn coverage_check() -> CoreResult<CoverageReport>`、`pub fn manifest() -> Value`、`pub struct Capability { id, title, since, grade, api, test, check, note }`
- **验收**：coverage_check 的 unknown/unclaimed/failing/bad_grades 四类问题全空；capabilities().len() == self_check().len()（无空头声明、无孤儿自检）；每条 api 落在真实模块且具体到函数；since 形如 v1.3.N；等级分布如实：unverified=0 且 cpu-proto>=1（跨节点传输为原型并在 note 写明）；results_json 与 scenario 携带同一份 manifest 与覆盖报告
- **证据**：`cargo test -p au4a-state` → 200/200，等级 verified（本机实测）（lib 87 + v131 13 + v132 11 + v133 16 + v134 13 + v135 11 + v136 14 + v137 10 + v138 16 + v139 9 全绿 0 warning。日志 E:\DS\_forangent\logs\1.3-v1.3.9.log）
- **状态**：✅ 已交付

#### v1.3.10 灾难恢复

- **目标**：备份 → 源丢失 → 2PC 重建 → 增量续跑 的完整演练。
- **交付物**：`src/disaster.rs: Backup（create/verify/restore_into）+ verify_media（任意介质的独立验证）`、`src/disaster.rs: DrillOptions/run_drill/DrillReport`、`src/capabilities.rs: 新增 disaster.drill 与 disaster.corrupted_backup_refused 两条能力（共 21 条）`、`tests/v1310_disaster.rs: 10 个集成测试`
- **接口**：`pub fn Backup::create(agent, snapshot) -> CoreResult<Backup>`、`pub fn Backup::verify(&self, policy) -> CoreResult<StateSnapshot>`、`pub fn Backup::restore_into(&self, node, policy) -> CoreResult<u64>`、`pub fn verify_media(store, prefix, signed, policy) -> CoreResult<StateSnapshot>`、`pub fn run_drill(agent, source, evolved, to_node, options) -> CoreResult<DrillReport>`、`pub struct DrillReport { backup_verified, source_lost, rebuild_first_*, rebuild_committed, ops_skipped, identical_to_source, consistency_clean, ... }`
- **验收**：备份可独立验证：改块/删块/伪造 provenance → 恢复被拒（不是尽力恢复）；源丢失后仅凭备份与签名重建出与源逐字节一致的 base；第一次 2PC 注入故障 → 回滚干净 → 同一笔迁移重试成功；增量续跑：dropped_frames>0、resumed_from>0、ops_skipped>0，最终内容根与源一致；consistency_clean、node_findings=0、rebuild_orphans=0 同时成立；5 个故障点逐个验证「注入→回滚→重试成功」；报告逐字节可重放
- **证据**：`cargo test -p au4a-state` → 215/215，等级 verified（本机实测）（lib 92 + v1310 10 + v131 13 + v132 11 + v133 16 + v134 13 + v135 11 + v136 14 + v137 10 + v138 16 + v139 9 全绿 0 warning。备份/重建/续跑本机实跑；备份介质与跨节点传输为内存实现（transfer.resumable 标注 cpu-proto）。日志 E:\DS\_forangent\logs\1.3-v1.3.10.log）
- **状态**：✅ 已交付

### v1.4 Economic Autonomy 经济自主（`au4a-economy`）

#### v1.4.1 余额管理

- **目标**：Agent 自主申报并执行余额策略（运营底线 / 目标质押比例 / 单笔支出上限），拒绝路径不改动账本。
- **交付物**：`src/balance.rs: BalancePolicy / SpendVerdict / check_spend / spend / autostake / BalanceManager / BalanceReport`、`src/lib.rs: self_check 两条真实余额自检 + scenario 注册-申报-支付-拒绝-质押-守恒`
- **接口**：`pub struct BalancePolicy { reserve_floor: Credits, stake_target_bp: i64, max_spend_bp: i64 }`、`pub fn check_spend(ledger: &Ledger, who: &Did, amount: Credits, policy: &BalancePolicy) -> CoreResult<SpendVerdict>`、`pub fn spend(ledger: &mut Ledger, from: &Did, to: &Did, amount: Credits, policy: &BalancePolicy) -> CoreResult<Credits>`、`pub fn autostake(ledger: &mut Ledger, who: &Did, policy: &BalancePolicy, budget: Credits) -> CoreResult<Credits>`、`pub struct BalanceManager (declare / policy / check / spend / autostake / report)`
- **验收**：支付 300 后 a 可用 700、b 可用 300，check_conservation 成立；可用 400 / 底线 100 时支出 301 被拒（InsufficientFunds），拒绝后账本逐字节不变；单笔上限可用余额 10% 时 1001 被拒、1000 通过；autostake 目标 = 总资产 × 2000bp，不穿底线，达标后再次补足返回 ZeroAmount；6 步脚本序列每一步后守恒成立，罚没精确等于锁定余额
- **证据**：`cargo test -p au4a-economy (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-economy)` → 10/10，等级 verified（本机实测）（本机 Windows 实测，cargo exit 0、0 warning；日志 _forangent\logs\1.4-v1.4.1.log）
- **状态**：✅ 已交付

#### v1.4.2 定价策略

- **目标**：Agent 用信誉分/稀缺度/负载的整数基点函数自主定价并广播，买方按整数单价发现并挑选最便宜供给方。
- **交付物**：`src/pricing.rs: PriceInputs / PriceKnobs / PriceComponents / PriceQuote / quote / unit_price / total_for / fingerprint`、`src/lib.rs: PRICE_KIND 报价公告 + scenario 定价-广播-验签-发现-成交；self_check 定价可复现与单调性`
- **接口**：`pub fn quote(inputs: &PriceInputs, knobs: &PriceKnobs) -> CoreResult<PriceQuote>`、`pub fn unit_price(inputs: &PriceInputs, knobs: &PriceKnobs) -> CoreResult<Credits>`、`pub struct PriceQuote { inputs, components, multiplier_bp: i64, unit_price: Credits }`、`pub const PRICE_KIND: &str = "economy.price"`
- **验收**：base 1000000 / 信誉 8000 / 稀缺 2000 / 负载 4000 → 折扣 1600、稀缺 600、负载 2000、乘数 11000、单价 1100000；同一输入两次报价相等且内容哈希相等；报价可被拒绝浮点的规范 JSON 编码；负载 0→10000 逐点不降、500bp 网格严格递增；信誉 0→10000 逐点不升、网格严格递减；稀缺每 +500bp 严格上升；乘数夹取 [9500,12000] 两端生效；base=1 满信誉时单价仍为 1（不塌缩为 0）；base 0 → ZeroAmount；信誉 10001 → InvalidKind；负数 → NegativeAmount；min>max → InvalidKind；空内核连跑两次场景规范 JSON 逐字节一致；卖方 408 / 对手 636 → 选中 408
- **证据**：`cargo test -p au4a-economy (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-economy)` → 21/21，等级 verified（本机实测）（本机 Windows 实测，cargo exit 0、0 warning；日志 _forangent\logs\1.4-v1.4.2.log）
- **状态**：✅ 已交付

#### v1.4.3 自动兑换

- **目标**：系统积分 ↔ BTC/ETH 兑换路由决策表（金额/时效/费用阈值）+ 账务预留；类型层封死「链上已成功」的表达，真实执行留待 v1.8。
- **交付物**：`src/fx.rs: Venue / Urgency / RouteAction / DecisionReason / ChainExecution / IntentStatus / RouteTable / ExchangeRequest / RoutePlan / ExchangeIntent / ExchangeBook / route / fee_for / escrow_for_route / cancel_intent / apply_internal`、`src/lib.rs: scenario 三条路由决策（内部/缓办/上链）+ 预留前过余额策略；self_check fx.decision_table 与 fx.no_fake_chain`
- **接口**：`pub fn route(request: &ExchangeRequest, table: &RouteTable) -> CoreResult<RoutePlan>`、`pub fn escrow_for_route(ledger: &mut Ledger, plan: &RoutePlan, at: u64) -> CoreResult<ExchangeIntent>`、`pub fn cancel_intent(ledger: &mut Ledger, intent: &ExchangeIntent) -> CoreResult<Credits>`、`pub fn apply_internal(ledger: &mut Ledger, plan: &RoutePlan, to: &Did) -> CoreResult<Credits>`、`pub enum ChainExecution { PendingV18, NotApplicable, NotRequested } // 无 Executed；executed() 恒 false`
- **验收**：199 → keep_internal/below_onchain_minimum（费用 0、到账 199）；1000 → route_onchain 走 ETH 费用 8、到账 992；偏好 BTC 时覆盖更便宜的 ETH（费用 12、到账 988）；费率相同按场所名升序 → BTC；剩余 19 < 所需 20 → defer/deadline_too_tight；费率 800bp > 阈值 150bp → defer/fee_above_threshold；缓办计划调用 escrow_for_route 返回 InvalidKind 且账本不变；预留 2000 → 可用 8000/锁定 2000 且守恒；意图状态 awaiting_chain_execution、on_chain_success() == false；ChainExecution::ALL 无 executed、IntentStatus::ALL.len() == 1；撤回预留后余额复原且守恒
- **证据**：`cargo test -p au4a-economy (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-economy)` → 34/34，等级 verified（本机实测）（决策与账务本机实测；链上执行未做（属 v1.8），类型层无法表达链上成功；日志 _forangent\logs\1.4-v1.4.3.log）
- **状态**：✅ 已交付

#### v1.4.4 质押管理

- **目标**：Agent 自主质押/解质押/冷静期解锁；罚没与质押簿同步；准入线与份额上限校验；不变式为「簿记承诺量 ≤ 账本锁定量」。
- **交付物**：`src/stake.rs: StakeTerms / Unbonding / StakePosition / StakeBook(adopt, stake, request_unstake, release_matured, absorb_slash, assert_consistent, to_json)`、`src/lib.rs: scenario 认领质押-自主解质押-无头寸被拒-冷静期释放；self_check stake.self_custody`
- **接口**：`pub fn stake(&mut self, ledger: &mut Ledger, terms: &StakeTerms, who: &Did, amount: Credits) -> CoreResult<StakePosition>`、`pub fn request_unstake(&mut self, terms: &StakeTerms, who: &Did, amount: Credits, now: u64) -> CoreResult<Unbonding>`、`pub fn release_matured(&mut self, ledger: &mut Ledger, who: &Did, now: u64) -> CoreResult<Credits>`、`pub fn absorb_slash(&mut self, who: &Did, amount: Credits) -> CoreResult<Credits>`、`pub fn assert_consistent(&self, ledger: &Ledger) -> CoreResult<()>`
- **验收**：质押 200 → 可用 800/锁定 200；99 < 下限 100 → InsufficientStake；501 > 上限 500 → InsufficientStake；解质押 150 后账本锁定不变，头寸 locked=250 / unbonding=150 / committed=400；release_at=25 时 t=24 释放 0、t=25 释放 100；无头寸者解质押 → UnknownAgent；罚没 10000 被锁定余额约束为 300 且簿记同步归零；外部解锁后 assert_consistent 报错；认领 150 不重复锁定，认领 151 被拒；JSON 投影无浮点
- **证据**：`cargo test -p au4a-economy (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-economy)` → 45/45，等级 verified（本机实测）（质押/解质押/冷静期/罚没同步均走真实 Ledger 写路径；日志 _forangent\logs\1.4-v1.4.4.log）
- **状态**：✅ 已交付

#### v1.4.5 争议仲裁

- **目标**：Agent 自主立案/投票/申诉；罚没上限 = 锁定余额（并记录截断来源）；申诉重开投票但已销毁的罚没不可退回。
- **交付物**：`src/arbitration.rs: DisputeState / Vote / Appeal / PenaltyCap / Dispute / ArbitrationTerms / Ruling / is_admissible / Court(open, vote, rule, appeal, close, case_json, to_json)`、`src/lib.rs: scenario 未验证证据被拒-立案-两票-罚没截断-申诉-重裁驳回-结案；self_check arbitration.penalty_cap`
- **接口**：`pub fn open(&mut self, claimant: &Did, respondent: &Did, claimed: Credits, evidence: EvidenceGrade, now: u64) -> CoreResult<Dispute>`、`pub fn vote(&mut self, case_id: &str, arbiter: &Did, uphold: bool, weight_bp: i64) -> CoreResult<()>`、`pub fn rule(&mut self, ledger: &mut Ledger, case_id: &str, terms: &ArbitrationTerms, now: u64) -> CoreResult<Ruling>`、`pub fn appeal(&mut self, case_id: &str, by: &Did, reason: &str, terms: &ArbitrationTerms, now: u64) -> CoreResult<Appeal>`、`pub enum PenaltyCap { None, EvidenceCap, LockedBalance }`
- **验收**：Unverified 证据立案 → InvalidKind；当事人投票 → UnknownAgent；重复投票 → InvalidKind；1 票 < 法定 2 票 → InvalidKind；支持票权 9000bp ≥ 5000bp → 罚没 50（cap none）；索赔 1000000、锁定 50 → 罚没 50（cap locked_balance），账本 slashed=50 且可用不变、守恒成立；cpu-proto 证据罚没被 100 上限截断（cap evidence_cap）；支持票权不足 → 罚没 0 且锁定不变；申诉后票作废、状态 appealed；窗口外/非当事人/空理由/超次数一律拒绝；重裁驳回：本次罚没 0、累计 100（销毁不可逆）；结案后 open_cases=0
- **证据**：`cargo test -p au4a-economy (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-economy)` → 56/56，等级 verified（本机实测）（本机 Windows 实测 cargo exit 0、0 warning；冻结错误码映射（state_conflict→InvalidKind、not_a_party→UnknownAgent）已写入文档；日志 _forangent\logs\1.4-v1.4.5.log）
- **状态**：✅ 已交付

#### v1.4.6 结算路由

- **目标**：结算路由决策表（证据等级/cpu-proto 上限/收款方争议）+ 整数最大余数法分成；收益归属资源提供者，人类操作者只收收益不决策。
- **交付物**：`src/settlement.rs: ProviderRole / BeneficiaryKind / DecisionRights / Beneficiary / RevenueShare / Receipt / SettlementRoute / SettlementPolicy / SettlementRequest / SettlementDecision / SettlementOutcome / RevenueBook / route / split_weights / settle_direct / execute_escrow / release_escrow / refund_escrow / pay_split`、`src/lib.rs: scenario 拒付-托管-退回-分成（140 compute + 60 human_operator）；self_check settlement.evidence_gate 与 settlement.split_exact`
- **接口**：`pub fn route(request: &SettlementRequest, policy: &SettlementPolicy) -> CoreResult<SettlementDecision>`、`pub fn split_weights(total: Credits, weights_bp: &[i64]) -> CoreResult<Vec<Credits>>`、`pub fn pay_split(kernel: &mut Kernel, request: &SettlementRequest, shares: &[RevenueShare], policy: &SettlementPolicy) -> CoreResult<SettlementOutcome>`、`pub fn execute_escrow / release_escrow / refund_escrow(ledger: &mut Ledger, ...) -> CoreResult<...>`、`pub fn Beneficiary::human_operator(did: Did) -> Beneficiary // decision_rights() == IncomeOnly`
- **验收**：零额 → ZeroAmount；Unverified → withheld/evidence_unverified；cpu-proto 超上限 → withheld/cpu_proto_cap；争议 → escrowed/payee_under_dispute；其余 direct；最大余数法：10→3/3/4、100→33/33/34、1000→333/333/334；1..500 全部总额 Σ 严格等于原额；权重和≠10000bp / 空表 / 越界 → 拒绝；总额 0 → ZeroAmount；200 分成 140/60 后付款方 790、提供者 1130/1050，守恒成立；拒付与托管下账本不动；托管 300 → 锁定 300；退回复原；放款给收款方 +300；人类操作者 decision_rights=income_only 且 may_decide=false
- **证据**：`cargo test -p au4a-economy (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-economy)` → 65/65，等级 verified（本机实测）（本 crate 0 warning；au4a-kernel 有 1 条 unused import 警告（属轨道 1.0，未越界修改）；日志 _forangent\logs\1.4-v1.4.6.log）
- **状态**：✅ 已交付

#### v1.4.7 测试

- **目标**：跨模块性质测试与端到端测试：400 步随机操作序列下守恒/质押簿不变式、全域定价单调、分成严格分完、罚没上限、拒绝不改账本、场景逐字节可复现。
- **交付物**：`tests/economy_invariants.rs: 10 个性质测试（确定性 LCG，无外部依赖）`、`tests/e2e_scenario.rs: 6 个端到端测试（场景契约、复现、伪造报价=恶意、Unverified 不结算、幂等、节点接线）`、`src/lib.rs: ensure_working_balance 补足工作余额并上报 topups；autostake 的 ZeroAmount 作为「无需补足」处理`
- **接口**：`cargo test -p au4a-economy 同时运行 lib + 2 个集成测试目标`、`scenario JSON 新增 topups 字段（创世补足量，如实上报）`
- **验收**：400 步混合操作序列每步后 check_conservation 与质押簿 assert_consistent 均成立，成功/拒绝路径都覆盖；同种子两次运行账本视图规范 JSON 逐字节一致，不同种子必须不同；定价 base∈{1,7,999,1000000} × 负载/信誉 0→10000 全域单调且价格≥1；500 组随机报价可复现无浮点；300 组随机权重分成 Σ==总额 且每份与理想值偏差 <1 微积分；40 个随机案件罚没 ≤ 锁定余额且账本 slashed 一致；5 类拒绝后 LedgerView 逐字段不变；端到端：4 个注册 Agent + 5 个账本账户，人类操作者可用 60/锁定 0；同内核重复运行幂等
- **证据**：`cargo test -p au4a-economy (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-economy)` → 81/81，等级 verified（本机实测）（lib 65 + e2e_scenario 6 + economy_invariants 10；本 crate 0 warning；日志 _forangent\logs\1.4-v1.4.7.log）
- **状态**：✅ 已交付

#### v1.4.8 文档

- **目标**：把 7 个版本的实现固化为可审计文档：版本索引、不可协商规则、错误码映射、证据分级与明确的未做清单。
- **交付物**：`crates/au4a-economy/README.md: 模块地图 / 决策表速查 / 错误码映射 / 证据分级 / 本地验证命令`、`docs/tracks/1.4.md: v1.4.8 汇总节（版本索引表 + 规则 + 错误码 + 证据 + 未做清单）`、`src/lib.rs: crate 级 rustdoc 增加模块地图与最短上手路径`
- **接口**：`文档不改变任何 public API；仅新增 README.md 与 rustdoc 内容`
- **验收**：版本索引表逐版给出模块/关键 public 项/累计测试数，与代码和日志一致；错误码映射表覆盖 ZeroAmount/NegativeAmount/InvalidKind/InsufficientFunds/InsufficientStake/UnknownAgent/Overflow；证据分级表明确标注「真实链上执行属 v1.8，本轨道未做」，不把未做写成已实现；README 与 rustdoc 中的 v1.4.9/v1.4.10 条目明确标注为后续版本落地
- **证据**：`cargo test -p au4a-economy (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-economy)` → 81/81，等级 verified（本机实测）（文档版不新增代码路径；测试结果沿用 v1.4.7 全绿运行（日志 _forangent\logs\1.4-v1.4.7.log））
- **状态**：✅ 已交付

#### v1.4.9 示例

- **目标**：可运行、确定性的「经济自主八步走」示例：用真实数字走完余额/定价/兑换/质押/仲裁/结算/不变量/自检，并打印场景 JSON。
- **交付物**：`examples/economy_tour.rs: 8 步示例，直接调用 public API 打印真实数字，最后输出 scenario JSON`、`docs/tracks/1.4.md: 示例原始输出摘录（来自本机运行日志）`
- **接口**：`cargo run -p au4a-economy --example economy_tour`、`示例 main 返回 au4a_core::CoreResult<()>（不用 unwrap/panic）`
- **验收**：示例 exit code 0，输出 8 行真实数字且与文档摘录一致；每步调用真实 API：check_spend/spend/autostake、quote、route、adopt/request_unstake/release_matured、Court::open/vote/rule/appeal、split_weights/earned、check_conservation、self_check、scenario；示例不读文件/不开网络/不读墙钟，同种子同输出；cargo test -p au4a-economy 仍 81 passed / 0 failed、0 warning（示例参与编译检查）
- **证据**：`cargo test -p au4a-economy; cargo run -p au4a-economy --example economy_tour` → 81/81，等级 verified（本机实测）（示例本机实测 exit 0，输出见 _forangent\logs\1.4-v1.4.9-example.log；测试日志 _forangent\logs\1.4-v1.4.9.log）
- **状态**：✅ 已交付

#### v1.4.10 监控

- **目标**：只读收益面板数据源：与节点 /api/revenue 字段对齐（minted/slashed/total/accounts.*），补上 earned/kind/display/conservation_ok；收益只算真实到账凭证。
- **交付物**：`src/monitor.rs: READ_ONLY / LedgerTotals / RevenueRow / RevenuePanel(to_json, conservation_ok, rows_earned_total) / ledger_totals / panel / revenue_panel / monitor_json / monitor_checks`、`src/lib.rs: 场景 JSON 新增 revenue_panel；self_check 新增 monitor.read_only（共 12 项）`、`examples/economy_tour.rs: 追加第 9 步（只读面板 + 3 条监控自检）`、`README.md: 只读收益面板章节 + 模块地图补齐 v1.4.10`
- **接口**：`pub const READ_ONLY: bool = true`、`pub fn ledger_totals(ledger: &Ledger) -> CoreResult<LedgerTotals>`、`pub fn panel(kernel: &Kernel, revenue: &RevenueBook, context: &Value) -> CoreResult<RevenuePanel>`、`pub fn revenue_panel(kernel: &Kernel, revenue: &RevenueBook, context: &Value) -> CoreResult<Value>`、`pub fn monitor_checks(kernel: &Kernel, revenue: &RevenueBook) -> Vec<SelfCheck>`
- **验收**：账本总量：发行 1500 / 罚没 50 / 锁定 150 / 可用 1300 / 总量 1450，total == available + locked，conservation_ok；面板列出每个账本账户：3 账户/2 Agent/收益 200；卖方 available 1130 earned 140 kind agent；人类操作者 kind human_operator display human-operator earned 60 locked 0；两次只读投影逐字段相同且读取前后 LedgerView 不变；rows_earned_total == total_earned；拒付后收益为 0 且余额不变（withheld/escrowed 永不计入收益）；3 条监控自检（ledger_conservation / conservation_gate / read_only）全部通过且 detail 非空；端到端：revenue_panel.ledger.account_count=5、total_earned=200、Σ可用+Σ锁定==总量；示例 exit 0
- **证据**：`cargo test -p au4a-economy; cargo run -p au4a-economy --example economy_tour` → 87/87，等级 verified（本机实测）（lib 71 + e2e 6 + invariants 10；本 crate 0 warning；示例实测 Σ可用 3059 + Σ锁定 841 + 罚没 100 = 发行 4000；日志 _forangent\logs\1.4-v1.4.10.log、1.4-v1.4.10-example.log）
- **状态**：✅ 已交付

### v1.5 Safety API 安全 API（`au4a-safety`）

#### v1.5.1 权限查询

- **目标**：Agent 能查询自己的权限边界，返回结构化的允许/禁止/需质押，而不是自由文本
- **交付物**：`src/config.rs: SafetyConfig（服务身份 + 仲裁者信任锚）与 require_arbiter 闸门`、`src/permission.rs: Permission / DenialReason / PermissionBoundary / PermissionQuery / stake_requirement / query_permissions`、`src/setup.rs: 确定性轨道种子与幂等 ensure_agent`、`src/lib.rs: self_check / results_json / scenario 首次落地真实流程`
- **接口**：`pub enum Permission { Register, SendMessage, QuerySafety, ReportViolation, AppealPenalty, SubscribeCase, Settle, IssueVerdict }`、`pub enum DenialReason { AlreadyRegistered, NotRegistered, StakeBelowMinimum { required: Credits, actual: Credits }, ArbiterOnly }`、`pub struct PermissionBoundary { did, registered, allowed, denied, stake_gates }`、`pub fn PermissionBoundary::of(kernel: &Kernel, config: &SafetyConfig, did: &Did) -> Self`、`pub fn stake_requirement(p: Permission, min_stake: Credits) -> Credits`、`pub fn query_permissions(kernel, config, about) -> CoreResult<Value>`
- **验收**：已注册 Agent：6 个权限可达，Register 得 already_registered，IssueVerdict 得 arbiter_only；未注册 DID：allowed == [Register]，其余 7 个权限全部 not_registered；余额 3 < min_stake 10 的 DID：注册被 stake_below_minimum{required:10,actual:3} 拒绝；罚没清零锁定质押后举报权被收回（stake_below_minimum），免费查询权不受影响；边界 JSON 的拒绝原因必为带 code 的结构化对象，allowed+denied == 8，可 JSON 往返
- **证据**：`cargo test -p au4a-safety` → 14/14，等级 verified（本机实测）（2026-10-04 本机 Windows 实测，0 warning；CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-safety；日志 _forangent\logs\1.5-v1.5.1.log）
- **状态**：✅ 已交付

#### v1.5.2 违规举报

- **目标**：Agent 举报他人违规，必须携带可验证证据哈希；未确认的举报不改变任何余额
- **交付物**：`src/evidence.rs: EvidenceRef（canonical_hash 承诺 + 复算校验）`、`src/chain.rs: SafetyEvent 哈希链（prev + 内容 → hash）与断链检测`、`src/case.rs: ViolationKind / CaseStatus / Case`、`src/office.rs: SafetyOffice::report + 事件追加`
- **接口**：`pub struct EvidenceRef { kind, digest }`、`pub fn EvidenceRef::commit(kind, payload: &Value) -> CoreResult<EvidenceRef>`、`pub struct SafetyOffice { .. }`、`pub fn SafetyOffice::report(&mut self, kernel, keys, subject, kind, evidence) -> CoreResult<ViolationReport>`
- **验收**：伪造/缺失证据的举报被拒（InvalidSignature / Encoding），案件数 0、事件数 0；举报前后账本快照与 AgentCard 逐字段相等（无罪不罚），slashed == 0；举报写入哈希链事件，链头 64 位 hex 且可复算；篡改内容/删除记录/改写前驱/伪造 hash 分别报 hash_mismatch/seq_mismatch/prev_mismatch；未注册举报者 unauthorized、自举报 InvalidKind、未知主体 stale_epoch，均留下结构化拒绝记录
- **证据**：`cargo test -p au4a-safety` → 44/44，等级 verified（本机实测）（2026-10-04 本机 Windows 实测，0 warning；日志 _forangent\logs\1.5-v1.5.2.log；累计 44 个测试（v1.5.1 的 14 个 + 本版 30 个））
- **状态**：✅ 已交付

#### v1.5.3 申诉提交

- **目标**：被处罚方本人提交申诉证据，状态机进入 APPEALED，申诉本身不动账本
- **交付物**：`src/appeal.rs: Appeal（主体签名）+ 证据列表`、`src/office.rs: appeal + REPORTED/APPEALED 状态迁移`
- **接口**：`pub struct Appeal { id, case, appellant, evidence, at, sig }`、`pub fn SafetyOffice::appeal(&mut self, kernel, keys, case_id, evidence) -> CoreResult<Appeal>`
- **验收**：第三方代签的申诉被拒（InvalidSignature + unauthorized），状态仍为 reported；未知案件被拒（UnknownAgent + stale_epoch）；空证据或载荷数不匹配被拒；申诉前后账本快照与 AgentCard 逐字段不变（slashed == 0）；主体申诉后状态 appealed、Case.appeals 按顺序记录、事件链 2 条且完好；新证据可再次申诉（申诉权不被终局性剥夺）
- **证据**：`cargo test -p au4a-safety` → 56/56，等级 verified（本机实测）（2026-10-04 本机 Windows 实测，0 warning；日志 _forangent\logs\1.5-v1.5.3.log；累计 56 个测试）
- **状态**：✅ 已交付

#### v1.5.4 处罚查询

- **目标**：查询处罚记录；处罚只能由仲裁者签名的数据契约触发，且罚没受锁定余额约束
- **交付物**：`src/penalty.rs: SanctionKind / PenaltyOrder（数据契约）/ PenaltyRecord`、`src/office.rs: apply_penalty_order + penalties_for 查询`
- **接口**：`pub enum SanctionKind { Warning, StakeSlash, Quarantine }`、`pub struct PenaltyOrder { case, subject, sanction, amount, arbiter, issued_at, sig }`、`pub fn SafetyOffice::apply_penalty_order(&mut self, kernel, order) -> CoreResult<PenaltyRecord>`、`pub fn SafetyOffice::penalties_for(&self, did: &Did) -> Vec<PenaltyRecord>`
- **验收**：非仲裁者签名的处罚契约被拒（unauthorized），账本逐字段不变；篡改金额的契约验签失败，账本不变；种类与金额不一致的契约被拒（InvalidKind/ZeroAmount）；罚没额超过锁定余额时按锁定余额封顶：requested 保留原值、applied 记录实际值，守恒仍成立；warning 类处罚写入记录但不动账本；penalties_for 按主体隔离查询，slashed_total == 账本 slashed
- **证据**：`cargo test -p au4a-safety` → 68/68，等级 verified（本机实测）（2026-10-04 本机 Windows 实测，0 warning；日志 _forangent\logs\1.5-v1.5.4.log；累计 68 个测试）
- **状态**：✅ 已交付

#### v1.5.5 通知机制

- **目标**：按案件订阅状态变更（REPORTED/APPEALED/ARBITRATED），可退订，只投递给订阅者
- **交付物**：`src/notify.rs: Subscription / Notification / 投递规则`、`src/office.rs: subscribe / unsubscribe / inbox`
- **接口**：`pub struct Subscription { id, subscriber, case, statuses, at, sig }`、`pub struct Notification { sub, to, case, status, seq, at }`、`pub fn SafetyOffice::subscribe(&mut self, keys, case, statuses) -> CoreResult<Subscription>`、`pub fn SafetyOffice::unsubscribe(&mut self, keys, sub_id) -> CoreResult<()>`、`pub fn SafetyOffice::inbox(&self, did: &Did) -> Vec<Notification>`
- **验收**：订阅即刻回放当前状态快照（seq 指向造成该状态的事件），此后只投递增量；未订阅的状态不投递；旁观者收件箱 0 条；通知彼此不可见；退订后不再投递且链上出现 unsubscribed；非本人退订被拒（unauthorized）；空状态集合被拒（InvalidKind + policy_denied），订阅 id 对状态集合规范化；订阅/退订前后账本逐字段不变；迟到的订阅不重放历史
- **证据**：`cargo test -p au4a-safety` → 83/83，等级 verified（本机实测）（2026-10-04 本机 Windows 实测，0 warning；日志 _forangent\logs\1.5-v1.5.5.log；累计 83 个测试）
- **状态**：✅ 已交付

#### v1.5.6 PMB扩展

- **目标**：安全 API 接入 PMB：SAFETY_QUERY / SAFETY_REPORT / SAFETY_APPEAL 三种消息类型，Envelope 真签名
- **交付物**：`src/pmb.rs: 消息类型常量、信封构造、SafetyMessage 解析与校验、safety.receipt 回执`、`src/office.rs: handle 服务侧分发`
- **接口**：`pub mod kinds { SAFETY_QUERY, SAFETY_REPORT, SAFETY_APPEAL, SAFETY_RECEIPT }`、`pub fn report_envelope(keys, to, ts, report) -> CoreResult<Envelope>`、`pub fn classify(env: &Envelope) -> CoreResult<SafetyMessage>`、`pub fn SafetyOffice::handle(&mut self, kernel, env) -> CoreResult<Option<Envelope>>`
- **验收**：三种消息经 Envelope::seal/verify 与 encode_frame/decode_frame 往返一致，MsgKind 合法；篡改 body 后 verify 失败，Kernel::send 记 unauthorized（单次即恶意码）；回执由服务身份签名，in_reply_to 指向请求 id，带结构化权限边界；非本人 about 的查询被拒；封 from 与记录作者不一致被拒；经 PMB 的举报同样复算证据：摘要不符即拒（policy_denied），不留案件与事件；经 PMB 的申诉把案件推到 appealed；本地 API 与网络路径共用 accept_report/accept_appeal
- **证据**：`cargo test -p au4a-safety` → 98/98，等级 verified（本机实测）（2026-10-04 本机 Windows 实测，0 warning；日志 _forangent\logs\1.5-v1.5.6.log；累计 98 个测试）
- **状态**：✅ 已交付

#### v1.5.7 仲裁接入

- **目标**：申诉进入仲裁：用数据契约与 au4a-council 解耦，案件进入 ARBITRATED，误判可回滚
- **交付物**：`src/arbitration.rs: ArbitrationRequest / ArbitrationVerdict（纯 JSON 数据契约）`、`src/office.rs: resolve（UPHELD 执行处罚 / REJECTED 归还罚没）+ ARBITRATED 终局`
- **接口**：`pub struct ArbitrationRequest { case, subject, reporter, violation, evidence, appeals, requested_at }`、`pub struct ArbitrationVerdict { case, outcome, sanction, amount, rationale_hash, arbiter, issued_at, sig }`、`pub enum VerdictOutcome { Upheld, Rejected }`、`pub fn SafetyOffice::resolve(&mut self, kernel, verdict) -> CoreResult<CaseOutcome>`
- **验收**：非法仲裁者/伪造签名的裁决被拒（InvalidSignature + unauthorized），账本不变；UPHELD：罚没执行（受锁定余额封顶）、状态 arbitrated、订阅者收到终局通知；REJECTED：罚没等额归还并重新锁定（质押回到原值），净罚没归零、守恒成立；裁决契约可仅用 serde_json 与手写 JSON 往返（不依赖 au4a-council crate）；理由哈希必须是 64 位 hex 承诺；rejected 不得携带金额；终局后新证据可再申诉（状态回 appealed），第二次裁决可统一回滚
- **证据**：`cargo test -p au4a-safety` → 113/113，等级 verified（本机实测）（2026-10-04 本机 Windows 实测；本 crate 0 warning；日志 _forangent\logs\1.5-v1.5.7.log；累计 113 个测试。回滚实现由测试修正：mint 之外必须 lock 回锁定质押）
- **状态**：✅ 已交付

#### v1.5.8 测试

- **目标**：端到端 + 对抗 + 确定性重放的独立测试套件
- **交付物**：`tests/end_to_end.rs: 完整案件生命周期`、`tests/adversarial.rs: 伪造证据/伪造裁决/第三方申诉/链篡改`、`tests/determinism.rs: scenario 两次运行字节一致`
- **接口**：`pub fn scenario(kernel: &mut Kernel) -> CoreResult<Value>（确定性契约）`
- **验收**：端到端：PMB 查询→举报→订阅→处罚→申诉→裁决 ARBITRATED 全程断言（含事件链 5 条）；对抗用例全部被拒且账本快照逐字段不变（伪造证据/裁决、第三方申诉/退订、无授权处罚）；链篡改在每一个位置都能被定位（broken_at == 篡改位置）；误判回滚后锁定质押回到原值，权限完好（不是「拿回钱丢了权」）；两个新内核上 scenario 字节一致；同一内核重复调用语义正确且账本守恒
- **证据**：`cargo test -p au4a-safety` → 128/128，等级 verified（本机实测）（本版交付 15 个集成测试（end_to_end 3 / adversarial 7 / determinism 5）全绿；lib 113 + 集成 15 = 128，0 warning。外部阻塞说明见 1.5.md 附录：v1.5.8/v1.5.9 的全绿取证与 v1.5.10 合并执行，快照取自最终树）
- **状态**：✅ 已交付

#### v1.5.9 文档

- **目标**：机器可读契约导出与文档定稿（线格式、错误映射表、证据分级）
- **交付物**：`src/schema.rs: schema_json() 导出消息类型/事件种类/案件状态/证据种类/链格式`、`docs/tracks/1.5.md 与 1.5.json 定稿`
- **接口**：`pub fn schema_json() -> CoreResult<Value>`
- **验收**：schema_json 可解析、覆盖消息类型/事件种类/状态/证据/处罚/权限/拒绝码；每个事件种类与权限点在 schema 中恰好出现一次（代码与文档同一来源）；线格式名与 au4a_core::kinds::SAFETY_REPORT 一致；genesis_prev 与 MAX_FRAME 与基元层一致；错误映射表 ≥ 10 条且每条三段式（情形 / core_error / refusal）；schema 本身是规范 JSON（无浮点，可 canonicalize）
- **证据**：`cargo test -p au4a-safety` → 127/127，等级 verified（本机实测）（schema 测试 5 个全绿（lib 累计 127 = 122 lib + 5 schema，随最终树合并取证）；0 warning）
- **状态**：✅ 已交付

#### v1.5.10 审计

- **目标**：审计报告：链完整性、状态==重放(事件链)、账本罚没与处罚记录交叉核对
- **交付物**：`src/audit.rs: AuditReport / AuditFinding / AuditCode`、`src/office.rs: from_journal 重放 + audit`
- **接口**：`pub fn SafetyOffice::audit(&self, kernel: &Kernel) -> AuditReport`、`pub fn SafetyOffice::from_journal(events, config, keys) -> CoreResult<SafetyOffice>`、`pub fn verify_chain(events: &[SafetyEvent]) -> ChainVerdict`
- **验收**：干净案件：audit ok、findings 为空、计数与四个金额（gross/net/refunded/ledger）正确；状态 == 重放(事件链)：from_journal 重建的实例链头/案件/处罚完全一致且审计同样干净；篡改事件后 verify_chain 指向断裂序号，from_journal 返回 InvalidSignature；账本被绕过改动时交叉核对发现 LedgerMismatch（而守恒式仍成立，证明守恒不够）；手工构造「给不存在案件开罚单」的日志被 flagged 为 case_missing 而不是被隐藏；self_check 聚合审计结论（audit.clean 项）
- **证据**：`cargo test -p au4a-safety` → 137/137，等级 verified（本机实测）（2026-10-04 本机 Windows 全绿：lib 122 + adversarial 7 + determinism 5 + end_to_end 3 = 137，0 failed、本 crate 0 warning；日志 _forangent\logs\1.5-v1.5.10.log。审计检查抓出并修正了两处真实的状态/重放不一致（status_seq 与裁决时间戳））
- **状态**：✅ 已交付

### v1.6 Individual Learning 个体学习（`au4a-learning`）

#### v1.6.1 经验库

- **目标**：让 Agent 把自己的经历写成可寻址、有界、可重放的数据结构：同语义只有一种字节表示，重复被精确识别，淘汰可见。
- **交付物**：`src/experience.rs: Outcome / Experience / ExperienceStore / RecordOutcome / StoreStats`、`src/rng.rs: SplitMix64（拒绝采样）+ 无状态 hash64 / hash_bp`、`src/scenario.rs: 端到端入口（自主注册 → 收集 24 条经验 → JSON 摘要）`、`tests/experience_store.rs: 10 个集成测试`
- **接口**：`pub enum Outcome { Success, Failure, Partial }`、`pub struct Experience { task_id, task_type, context, action, outcome, reward, timestamp, peer_agents }`、`pub fn Experience::new(..., peers: &[Did]) -> CoreResult<Experience>`、`pub fn Experience::key(&self) -> CoreResult<String>`、`pub fn ExperienceStore::record(&mut self, exp: Experience) -> CoreResult<RecordOutcome>`、`pub fn ExperienceStore::canonical_json(&self) -> CoreResult<String>`、`pub fn ExperienceStore::from_json_str(s: &str) -> CoreResult<ExperienceStore>`、`pub enum RecordOutcome { Added, Duplicate, Evicted }`
- **验收**：同一经验记录两次 → Duplicate，len 保持 1，duplicates == 1；容量 3 写 4 条 → Evicted 且 evictions == 1、len == 3；空/超长/控制字符/17 协作者/未升序/容量 0 → CoreError::InvalidKind；协作者顺序不同但集合相同 → 同一个内容键（SHA-256）；相同构建序列两次运行 canonical_json 与 digest 逐字节相同；规范 JSON 往返摘要不变；重复条目/非法字段/capacity<len 被拒绝；scenario 两次运行结果相同，重复调用幂等，账本守恒
- **证据**：`cargo test -p au4a-learning` → 15/15，等级 verified（本机实测）（本机 Windows 实测；CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-learning；日志 E:\DS\_forangent\logs\1.6-v1.6.1.log。经验库为内存结构（本轨道按契约不做文件 I/O）。）
- **状态**：✅ 已交付

#### v1.6.2 反馈机制

- **目标**：把原始经验聚合成按任务类型与协作者维度的反馈统计（成功率/部分成功/失败、平均质量、平均收益、置信度），成功经验通过共享内核账本真实结算。
- **交付物**：`src/feedback.rs: Feedback / PeerFeedback / FeedbackReport / FeedbackAnalyser / confidence_of`、`src/scenario.rs: 反馈阶段 + cpu-proto 结算阶段（拒绝被计数）`、`tests/feedback.rs: 10 个集成测试`
- **接口**：`pub const MIN_SAMPLES: usize = 4`、`pub fn confidence_of(sample: usize) -> i64`、`pub struct Feedback { scope, sample, success_bp, partial_bp, failure_bp, quality_bp, reward_total, mean_reward, confidence_bp, sufficient }`、`pub struct PeerFeedback { peer, sample, success_bp, quality_bp, mean_reward, confidence_bp, sufficient }`、`pub struct FeedbackReport { total, overall, per_type, per_peer }`、`pub fn FeedbackReport::public_json(&self) -> CoreResult<Value>`、`pub fn FeedbackAnalyser::analyse(store: &ExperienceStore) -> CoreResult<FeedbackReport>`
- **验收**：4 条混合经验 → success/partial/failure = 5000/2500/2500 bp 且三率和恒为 10000；quality_bp=6250、reward_total=100、mean_reward=25（整数除法）；样本 1/4 → sufficient=false 且 confidence_bp=2500；空库 → 零报告不是错误；协作者维度统计精确（成功/失败/部分各自落在正确 peer 上）；public_json 不含 did:au4a:、不含上下文原文、不含 per_peer 字段；端到端 settled_total == successes×40 且账本守恒；cap=0 时拒付被计数
- **证据**：`cargo test -p au4a-learning` → 25/25，等级 verified（本机实测）（本机 Windows 实测：5 单元 + 10 经验库集成 + 10 反馈集成。结算走本地内存账本（无真实链上转账），结算金额受 EvidenceGrade::CpuProto 上限保护。日志 E:\DS\_forangent\logs\1.6-v1.6.2.log）
- **状态**：✅ 已交付

#### v1.6.3 行为调整

- **目标**：让学习真的改变行为：把反馈与信号变成定价/任务选择/协作对象选择三个策略参数的下一个取值，并用同种子的对照实验量化改善。
- **交付物**：`src/policy.rs: PolicyParams / PolicyBounds / PolicyTargets / Signals / PolicyAdjustment / adjust() / task_score_bp()`、`src/violation.rs: Violation / ViolationLog（违规单独记账、按协作者计数、公开投影脱敏）`、`src/sim.rs: 合成任务市场 + ab_test() + Comparison（学习组 vs 对照组同种子同一市场）`、`src/scenario.rs: 用 24 条经验做真实行为调整 + 对照实验`、`tests/policy.rs（9 个）、tests/market.rs（10 个）`
- **接口**：`pub struct PolicyParams { price_bp, task_bias_bp, peer_bias_bp }`、`pub fn PolicyParams::baseline() -> PolicyParams  // 12000bp + 均匀偏好 = 对照组`、`pub struct PolicyBounds / PolicyTargets / Signals`、`pub fn adjust(&PolicyParams, &FeedbackReport, &ViolationLog, &Signals, &PolicyBounds, &PolicyTargets) -> CoreResult<PolicyAdjustment>`、`pub fn task_score_bp(&Feedback, Credits) -> i64`、`pub fn ab_test(&MarketConfig) -> CoreResult<(MarketRun, MarketRun, Comparison)>`、`pub fn compare(&MarketRun, &MarketRun) -> CoreResult<Comparison>`、`pub struct ViolationLog { record/len/total/count_of/peers/public_json }`
- **验收**：confidence_bp < min_confidence_bp → changed=false 且三个杠杆都不动（理由 evidence-below-threshold）；接受率 3000bp → 降价一步；9800bp → 提价一步；死区内 → 不动；good.type 偏好上升、bad.type 下降；peer0 上升、peer1 下降；每条理由可追溯到数据；平滑收益跌破死区 → 反向；收益改善 → 沿原方向继续；违规协作者偏好单调更低（-2000bp 惩罚），理由含 violations 计数；边界钳制：价格在上界时移动量为 0；偏好始终在 [bias_min, bias_max]；非法边界/门槛被拒；公开投影不含 did:au4a:（理由文本用 DID 摘要前缀 peer_tag）；对照组参数完全冻结；学习组价格 12000→9000bp、3 类任务偏好、8 个协作者偏好；对照组 2239bp/3240 微积分/21 违规 vs 学习组 4583bp/5105 微积分/12 违规（成功率 +2344bp、收益 +57.6%、违规 -43%）；两次 ab_test 逐字节一致；不同种子/不同 tick 数的 compare() 返回 InvalidKind
- **证据**：`cargo test -p au4a-learning` → 44/44，等级 verified（本机实测）（本机 Windows 实测：5 单元 + 10 经验库 + 10 反馈 + 10 市场 + 9 策略。A/B 数据为 192 tick/组、种子 0x161616 的真实运行输出（日志 1.6-v1.6.3-market.log）。提升结论的适用范围是本仓合成市场，不是真实网络绩效（该主张按 cpu-proto 对待）。第一版用相邻两轮收益比较驱动定价，实测学习组反而更差；改为接受率驱动 + EMA 安全阀后才为正，教训记录在 policy.rs::price_direction。日志 E:\DS\_forangent\logs\1.6-v1.6.3.log）
- **状态**：✅ 已交付

#### v1.6.4 学习信号

- **目标**：把四类信号（任务完成质量 / 结算金额 / 信誉变化 / 违规记录）折算成统一的整数信号向量，并让它们真的参与决策。
- **交付物**：`src/signal.rs: SignalWeights / LearningSignal / VIOLATION_UNIT_BP`、`src/policy.rs: 信誉下降时冻结涨价（让信誉信号真的改变行为）`、`src/sim.rs: 每轮用 from_store 生成信号并喂给 adjust()，RoundStats.signal_composite_bp 留证`、`src/scenario.rs: 端到端摘要输出 learning_signal`、`tests/signal.rs: 8 个集成测试`
- **接口**：`pub struct SignalWeights { quality_bp, settled_bp, reputation_bp, violation_bp, reward_ref, reputation_ref }`、`pub const VIOLATION_UNIT_BP: i64 = 2500`、`pub struct LearningSignal { quality_bp, settled, reputation_delta, violations, *_norm_bp, composite_bp }`、`pub fn LearningSignal::compute(quality_bp, settled, reputation_delta, violations, &SignalWeights) -> CoreResult<LearningSignal>`、`pub fn LearningSignal::from_experience(&Experience, &ViolationLog, &SignalWeights) -> CoreResult<LearningSignal>`、`pub fn LearningSignal::from_store(&ExperienceStore, &ViolationLog, reputation_delta, &SignalWeights) -> CoreResult<LearningSignal>`、`pub fn LearningSignal::to_signals(&self, sample, accept_rate_bp, mean_reward, prev_mean_reward, prev_price_dir) -> Signals`
- **验收**：质量 5000 + 结算 50/100 + 信誉 +50/100 → 归一 (5000,5000,5000,0)、综合 4500bp（精确）；四类全满 → 9000bp；四类全负 → -2500bp；质量↑/结算↑/信誉↑ → 综合↑；信誉↓/违规↑ → 综合↓；结算与信誉归一值封顶 10000bp；违规第 4 条起吃满；from_experience 只统计该条经验参与者的违规；from_store：质量均值 6250bp、结算求和 100；空库给零信号不是错误；权重校验：负权重 → NegativeAmount；权重和 >10000 / 分母为 0 → InvalidKind；reputation_delta=-40 时冻结涨价（price_moved=0，理由含『冻结涨价』），reputation_delta=0 时 +500bp
- **证据**：`cargo test -p au4a-learning` → 52/52，等级 verified（本机实测）（本机 Windows 实测：5 单元 + 10 经验库 + 10 反馈 + 10 市场 + 9 策略 + 8 信号。信誉信号在本版仍由调用方显式传入（市场里如实传 0，信誉台账到 v1.6.5 才落地），不假装已有信誉数据。日志 E:\DS\_forangent\logs\1.6-v1.6.4.log）
- **状态**：✅ 已交付

#### v1.6.5 模型更新

- **目标**：带学习率/动量/阻尼/遗忘/漂移钳制/代际回滚的整数模型更新；落地不可转让、有界的本地信誉台账。
- **交付物**：`src/model.rs: ReputationLedger / ModelConfig / LearningModel / UpdateRecord`、`src/sim.rs: 调整由 LearningModel::apply 落地；每 tick 用信誉台账记录观测；RoundStats 增加 price_drift_bp/reputation_delta/generation`、`src/lib.rs: 错误映射补充 InvalidVersion（回滚不存在代际）`、`tests/model.rs: 11 个集成测试`
- **接口**：`pub struct ReputationLedger { score_of / apply_outcome / apply_violation / apply_experience / public_json }`、`pub const REPUTATION_MIN: i64 = -1000; pub const REPUTATION_MAX: i64 = 1000`、`pub struct ModelConfig { learning_rate_bp, momentum_bp, forgetting_bp, max_drift_bp, max_generations }`、`pub fn LearningModel::apply(&mut self, &PolicyAdjustment) -> CoreResult<UpdateRecord>`、`pub fn LearningModel::rollback(&mut self) -> CoreResult<PolicyParams>`、`pub struct UpdateRecord { generation, price_before_bp, price_after_bp, price_drift_bp, momentum_bp, damped, clamped, forgotten_entries, reputation_delta }`
- **验收**：动量：连续同向意图的实际位移 -350,-455,-486,-495,-498,-499（幅度单调上升，momentum=0 时第一步即 -500）；阻尼：反转代 damped=true 且幅度减半（+500 → +250）；遗忘：400bp 偏好 80 代无更新后归零并移除条目；forgetting=0 时不衰减；漂移钳制：|price_drift| ≤ 1000bp 且偏好单代位移同样被封顶；不越过 PolicyBounds；回滚：第 0 代 → InvalidVersion；两代后回滚逐字段还原且代际回退；历史有界：max_generations=4 跑 10 代 → 只能回滚 4 次；信誉：精确增量 +20/+5/-10/-100；上界 1000、下界 -1000；第三方经历不影响其他协作者（无转让路径）；信誉只来自被观测经验：apply_experience 对两个参与者各记一次；非法配置（负学习率/动量>10000/遗忘>10000/max_generations=0/负漂移）→ InvalidKind；市场闭环：学习组 final_generation == rounds、对照组 == 0；仍有可量化改善（成功率 +1823bp、收益 +43.6%）
- **证据**：`cargo test -p au4a-learning` → 63/63，等级 verified（本机实测）（本机 Windows 实测：5 单元 + 10 经验库 + 10 反馈 + 10 市场 + 11 模型 + 9 策略 + 8 信号。接入模型更新后实测控制组 2239bp/3240/21 违规 vs 学习组 4062bp/4652/15 违规。v1.6.5 期间 au4a-kernel 曾编译失败阻塞约 5 分钟（已由 track-kernel 修复），期间继续写 v1.6.6 代码、未跳过验证。日志 E:\DS\_forangent\logs\1.6-v1.6.5.log）
- **状态**：✅ 已交付

#### v1.6.6 隐私保护

- **目标**：本地明文经验库 / 本地加密视图 / 对外公开视图三条路径彻底分开：公开视图无上下文原文、无 peer DID、无精确金额，小样本聚合被抑制。
- **交付物**：`src/privacy.rs: PrivacyPolicy / PublicAggregate / PublicView / SealedBlob / publish() / seal() / open() / peer_tag()`、`src/scenario.rs: 端到端摘要输出 privacy（公开视图 + 加密视图字节数）`、`tests/privacy.rs: 8 个集成测试`
- **接口**：`pub const DEFAULT_MIN_BUCKET_SAMPLE: usize = 3; pub const MIN_SALT_LEN: usize = 8`、`pub struct PrivacyPolicy { salt, min_bucket_sample, reward_bucket_credits, context_bucket_chars }`、`pub struct PublicAggregate { task_type, sample, suppressed, success_bp, mean_reward_bucket, context_bucket, distinct_peers }`、`pub fn peer_tag(&PrivacyPolicy, &Did) -> CoreResult<String>`、`pub fn publish(&ExperienceStore, &PrivacyPolicy) -> CoreResult<PublicView>`、`pub fn seal(&ExperienceStore, key: &[u8; 32]) -> CoreResult<SealedBlob>`、`pub fn open(&SealedBlob, key: &[u8; 32]) -> CoreResult<ExperienceStore>`
- **验收**：公开视图不含 SECRET-CONTEXT/不可外泄（上下文原文）、不含 did:au4a:、不含 task_id；金额只以桶出现（37 → 25）；协作者只以 distinct_peers 计数出现；阈值 6 → 2 组全抑制且统计字段全为 None；阈值 5 → 只抑制冷门组（>= 边界）；阈值 1 → 无抑制；密文 ≠ 明文；正确密钥往返摘要一致；同密钥同内容两次加密逐字节一致；不同密钥密文与标签都不同；错密钥/篡改 → InvalidSignature（先比标签再解码）；长度不一致 → FrameTruncated；非法 hex → Encoding；版本 2 → InvalidVersion；换盐换标签、同盐稳定、标签不含 DID 片段；公开视图不发布标签明文只发布计数；盐 <8 / min_bucket_sample=0 → InvalidKind；负金额桶宽 → NegativeAmount；桶宽 0 表示不发布该类信息；端到端 privacy 投影无 DID、无上下文原文，view.total>0、sealed_bytes>0
- **证据**：`cargo test -p au4a-learning` → 71/71，等级 verified（本机实测）（本机 Windows 实测：5 单元 + 10 经验库 + 10 反馈 + 10 市场 + 11 模型 + 8 隐私 + 9 策略 + 8 信号。脱敏/抑制/加盐/键流加密与完整性校验为 verified；『生产级机密性』不做主张（原型：SHA-256 计数器模式密钥流 + XOR + 明文摘要标签，无 AEAD/KDF/nonce 管理）→ 该部分 cpu-proto。日志 E:\DS\_forangent\logs\1.6-v1.6.6.log）
- **状态**：✅ 已交付

#### v1.6.7 测试

- **目标**：把不变式、可重放性与拒绝路径做成系统性测试套件（多种子扫描 + 逐字节复现 + 全部错误路径）。
- **交付物**：`tests/invariants.rs`、`tests/reproducibility.rs`、`tests/refusals.rs`
- **接口**：`tests/invariants.rs: 32 种子不变式扫描（复用 sim::ab_test / MarketConfig / MarketRun）`、`tests/reproducibility.rs: 规范 JSON 逐字节复现与摘要敏感性`、`tests/refusals.rs: 7 类 CoreError 清单 + 3 类软拒绝 + 300 次畸形输入扫描`
- **验收**：32 个种子扫描下全部结构性不变式成立（tick/结局/轮内聚合/边界/代际/窗口）；32 个种子中 29 个成功率与收益同时提升，平均成功率提升 +1025bp；两次独立流水线（含经验库→反馈→调整→模型→公开视图→加密）规范 JSON 逐字节相同；默认市场 ab_test 与 scenario 双跑逐字节一致，内核账本视图与逻辑时钟读数也一致；摘要对追加顺序敏感（[1,2,3] ≠ [3,2,1]），避免把顺序无关误当可复现；7 类 CoreError 全部有显式触发用例且清单完整无重复（InvalidKind/Encoding/NegativeAmount/Overflow/InvalidSignature/InvalidVersion/FrameTruncated）；3 类软拒绝（证据不足不学习、小样本抑制、证据闸门拒付）都有断言；300 次确定性畸形输入扫描无 panic，且合法/非法输入都被覆盖
- **证据**：`cargo test -p au4a-learning` → 81/81，等级 verified（本机实测）（本机 Windows 实测：5 单元 + 10 经验库 + 10 反馈 + 10 市场 + 11 模型 + 8 隐私 + 9 策略 + 8 信号 + 3 不变式 + 4 复现 + 3 拒绝。种子扫描实测 32 种子/29 改善/平均 +1025bp。日志 E:\DS\_forangent\logs\1.6-v1.6.7.log）
- **状态**：✅ 已交付

#### v1.6.8 文档

- **目标**：让文档与代码不可能分叉：策略解释由与行为相同的函数产出，并发布『解释所蕴含的下一步动作』，测试逐项断言其与实际行为一致。
- **交付物**：`src/explain.rs: PolicyExplanation / PriceExplanation / BiasExplanation / explain() / explain_peer_tag()`、`src/policy.rs: 抽出唯一来源 price_decision() 与 bias_delta_bp()（adjust 与 explain 共用）`、`tests/explain.rs: 5 个集成测试`、`docs/tracks/1.6.md 与 1.6.json：10 个小版本全部成文`
- **接口**：`pub fn price_decision(&Signals, &PolicyTargets) -> (i64, String)`、`pub fn bias_delta_bp(score_bp: i64, overall_bp: i64, &PolicyBounds) -> i64`、`pub fn explain(&PolicyParams, &FeedbackReport, &ViolationLog, &Signals, &PolicyBounds, &PolicyTargets) -> CoreResult<PolicyExplanation>`、`pub struct PriceExplanation { implied_direction, implied_move_bp, reason, ... }`、`pub struct BiasExplanation { target, kind, current_bias_bp, score_bp, overall_bp, violations, implied_delta_bp, reason }`、`pub fn PolicyExplanation::summary(&self) -> String`、`pub fn explain_peer_tag(&Did) -> String`
- **验收**：7 种输入矩阵下 explain().price.implied_move_bp == adjust().price_moved_bp；任务/协作者偏好的 implied_delta_bp（钳制后）与实际移动量逐项相等；理由引用真实数据：接受率 3000bp、目标 8500bp、死区 ±500bp、样本 8 条、good.type 评分 9333bp、bad.type 评分 0；样本不足的类型 implied_delta_bp=0 且理由写明不足；两次 explain 逐字段相等；摘要与 JSON 均不含 did:au4a:；协作者一律 peer:<摘要>；价格在上界时方向仍为上移但 implied_move_bp=0（与 adjust 一致）
- **证据**：`cargo test -p au4a-learning` → 86/86，等级 verified（本机实测）（本机 Windows 实测：5 单元 + 10 经验库 + 5 解释 + 10 反馈 + 10 市场 + 11 模型 + 8 隐私 + 9 策略 + 8 信号 + 3 不变式 + 4 复现 + 3 拒绝。重构 price_decision 时漏掉『信誉闸作用于最终方向』导致 seed=5654 回归，被 32 种子不变式扫描当场抓到并修复（教训写进文档）。日志 E:\DS\_forangent\logs\1.6-v1.6.8.log）
- **状态**：✅ 已交付

#### v1.6.9 示例

- **目标**：可运行示例：一个 Agent 跑完整学习循环（六阶段），打印可核对的 JSON 报告；示例不含逻辑，测试断言的就是示例打印的内容。
- **交付物**：`src/demo.rs: demo_report(seed) / demo_report_is_publishable() / DEMO_SEED / DEMO_EXPERIENCES / DEMO_GENERATIONS / DEMO_QUOTES`、`examples/learning_loop.rs: 命令行入口（可选种子），打印 pretty JSON`、`tests/demo.rs: 4 个集成测试`
- **接口**：`pub fn demo_report(seed: u64) -> CoreResult<serde_json::Value>`、`pub fn demo_report_is_publishable(report: &Value) -> bool`、`pub const DEMO_SEED: u64 = 0x160909; DEMO_EXPERIENCES = 18; DEMO_GENERATIONS = 3; DEMO_QUOTES = 24`
- **验收**：六阶段（经验收集/反馈分析/学习信号/行为调整/模型更新/效果评估）齐全、顺序正确、每段证据非空；行为调整 changed=true 且 price_moved_bp=-500（接受率 5000bp 低于目标 8500bp）；模型更新 3 代，第一步位移幅度 < 第三步（动量可见），代际 1→3；效果评估 improved=true、成功率与收益提升 > 0、params_differ=true；同种子两次报告完全相等且规范 JSON 一致；换种子得到不同报告；5 个额外种子均成功；报告不含 did:au4a: 与上下文原文，可被规范 JSON 编码；policy_before.price_bp=12000 且 policy_after≠12000；cargo run --example learning_loop 退出码 0
- **证据**：`cargo test -p au4a-learning` → 90/90，等级 verified（本机实测）（本机 Windows 实测：5 单元 + 4 示例 + 10 经验库 + 5 解释 + 10 反馈 + 10 市场 + 11 模型 + 8 隐私 + 9 策略 + 8 信号 + 3 不变式 + 4 复现 + 3 拒绝。附加证据：cargo run -p au4a-learning --example learning_loop → exit 0，实测成功率 2864→5156bp、收益 4692→6700、违规 9→7（日志 1.6-v1.6.9-example.log）。日志 E:\DS\_forangent\logs\1.6-v1.6.9.log）
- **状态**：✅ 已交付

#### v1.6.10 评估

- **目标**：固定种子的学习组 vs 对照组评估：池化成功率/收益提升 + 跨种子稳健性（中位数、最差种子、负向种子数）+ 消融实验分辨三个杠杆的贡献。
- **交付物**：`src/eval.rs: EvalConfig / ArmStats / SeedResult / EvalReport / AblationRow / evaluate() / ablations() / EVAL_SEEDS`、`src/sim.rs: MarketConfig 增加 learn_price/learn_task/learn_peer 掩码（消融）与矛盾配置拒绝`、`tests/eval.rs: 6 个集成测试`
- **接口**：`pub fn evaluate(&EvalConfig) -> CoreResult<EvalReport>`、`pub fn ablations(&EvalConfig) -> CoreResult<Vec<AblationRow>>`、`pub struct EvalReport { seeds, ticks_per_arm, control, learning, per_seed, success_lift_bp, revenue_lift_bp, revenue_lift_credits, violations_reduction, improved_seeds, negative_seeds, median/min/max_success_lift_bp }`、`pub fn EvalReport::improved(&self) -> bool`、`pub fn EvalReport::stable(&self) -> bool`、`pub fn EvalReport::summary(&self) -> CoreResult<Value>`、`pub const EVAL_SEEDS: [u64; 6]`
- **验收**：6 种子 × 192 tick/组：improved() 与 stable() 均为真，成功率/收益提升 > 0、违规减少 ≥ 0；中位数提升 > 0、改善种子 ≥ 2/3、最差种子 > -2000bp、负向种子 ≤ 1/3；每种子 params_differ=true 且学习组定价 <12000bp；对照组 params_changed_runs=0、学习组=种子数；逐种子收益之和 == 汇总收益（两组都断言）；两次 evaluate 逐字段/摘要/规范 JSON 逐字节一致；换种子集合换报告；消融四行：price-only 提升 > 0；all-levers ≥ task-only 与 peer-only；peer-only 违规减少 ≥ 0；非法配置（空种子 / >64 种子 / 非法市场 / 三杠杆全屏蔽）→ InvalidKind；limited(0) 保留 1 个种子
- **证据**：`cargo test -p au4a-learning` → 96/96，等级 verified（本机实测）（本机 Windows 实测：5 单元 + 4 示例 + 6 评估 + 10 经验库 + 5 解释 + 10 反馈 + 10 市场 + 11 模型 + 8 隐私 + 9 策略 + 8 信号 + 3 不变式 + 4 复现 + 3 拒绝。6 种子实测：成功率 2300→3906bp（+1606bp，中位 +1406bp，最小 +1094bp，6/6 改善），收益 22968→33919（+47.7%），违规 75→40（-47%）；消融：只学定价 +1024bp/+2047bp、只学任务 +226bp/+1244bp、只学协作 +122bp/+548bp、全开 +1268bp/+3425bp。绩效结论的适用范围是本仓合成市场 → 该主张 cpu-proto；代码与断言为 verified。日志 E:\DS\_forangent\logs\1.6-v1.6.10.log、1.6-v1.6.10-eval.log）
- **状态**：✅ 已交付

### v1.7 Committee Governance 委员会治理（`au4a-council`）

#### v1.7.1 选举机制

- **目标**：五类委员会由高信誉+长期在线 Agent 选举产生，结果确定可复现，刷票无法当选
- **交付物**：`src/committee.rs: 五类委员会封闭枚举 + 席位 + BFT-lite 数学`、`src/election.rs: 签名选票 + 加权计票纯函数 + 内容寻址结果`、`src/lib.rs: Council（信誉账/在线时长账/选举日志）+ elect/self_check/results_json/scenario`、`tests/election.rs: 8 个集成测试`
- **接口**：`pub enum CommitteeKind { Resource, Task, Arbitration, Evolution, Security }`、`pub struct Committee { kind, epoch, seats, elected_at, election_id, members }`、`pub struct ElectionBallot { voter, committee, choices, sig }`、`pub fn election::run(kind, epoch, &ElectionConfig, &[Candidate], &[ElectionBallot], at) -> CoreResult<ElectionOutcome>`、`pub fn Council::elect(&mut self, &mut Kernel, CommitteeKind, &[ElectionBallot]) -> CoreResult<ElectionOutcome>`
- **验收**：五类委员会名字往返一致，紧急通道仅安全委员会持有；n=5 时 f=1、quorum=4，n ≥ 3f+1 与 quorum = n - f 成立；同一输入（选票乱序）选举 id 相同；epoch 递增；60 个信誉 0 的空壳 DID 投票后当选名单与基线一致，忽略票留痕；伪造 → InvalidSignature；跨委员会重放 → InvalidKind；未注册 → UnknownAgent；重复投票 → DuplicateAgent；空委员会 quorum=0 但不具备表决资格
- **证据**：`cargo test -p au4a-council` → 19/19，等级 verified（本机实测）（本机 Windows 实测：单元 11 + 集成 8，0 warning；Ed25519 验签真实执行；跨节点广播不在本版范围）
- **状态**：✅ 已交付

#### v1.7.2 提案流程

- **目标**：动议只能由 Agent 提出且内容由 Agent 私钥签名；人类观察者在类型层面无提案/修改能力
- **交付物**：`src/proposal.rs: Action/ProposalState 状态机/Proposal/AgentIdentity 能力凭证/ProposalDraft 签名动议`、`src/human.rs: HumanObserver 只读投影 + 2 个 compile_fail 文档测试 + 1 个正例文档测试`、`src/lib.rs: Council::propose 唯一提案入口、transition_to 类型化状态机、CouncilEvent 事件日志`、`tests/proposal.rs: 10 个集成测试`
- **接口**：`pub enum Action { SetPolicy, SetReputation, Transfer, Slash }`、`pub enum ProposalState { Open, Passed, Rejected, Blocked, Executed }`、`pub struct AgentIdentity  // 字段私有、无 Deserialize，唯一入口 from_keys`、`pub fn Council::propose(&mut self, &mut Kernel, &AgentIdentity, ProposalDraft) -> CoreResult<Proposal>`、`pub fn Council::transition_to(&mut self, &mut Kernel, &str, ProposalState) -> CoreResult<Proposal>`、`pub struct HumanObserver  // 仅 new/label/observe，无 propose/edit`
- **验收**：委员 Agent 可提案，id 可由内容复算；非委员 → InvalidSignature + unauthorized；未注册 → UnknownAgent；未选出委员会 → StaleEpoch；同内容重复动议 → DuplicateAgent + conflict；篡改内容后签名失效（金额 5→500 被拒）；非法动作与空标题在提案阶段被拒，零状态变更；状态机拒绝 open→executed、passed→rejected、终态再迁移；观察两次结果相同且不产生治理事件；人类 propose/edit_proposal 编译失败（compile_fail 文档测试）
- **证据**：`cargo test -p au4a-council` → 37/37，等级 verified（本机实测）（EXIT=0；单元 16 + election 8 + proposal 10 + doc-tests 3（2 compile_fail + 1 正例），0 warning）
- **状态**：✅ 已交付

#### v1.7.3 表决机制

- **目标**：BFT-lite 法定人数表决（n≥3f+1、quorum=n-f），重复投票被拒，模棱两可双签作废整轮
- **交付物**：`src/voting.rs: Choice/Vote 签名票/Tally 计票/RoundOutcome/VotingRound/RoundState`、`src/lib.rs: Council::open_round / cast_vote / round / current_round / rounds + 私有 apply_state`、`tests/voting.rs: 10 个集成测试`
- **接口**：`pub enum Choice { Yes, No, Abstain }`、`pub struct Vote { voter, proposal, round, choice, sig }`、`pub struct Tally { yes, no, abstain, participation, n, f, quorum }`、`pub enum RoundOutcome { Pending, Passed, Rejected, VoidAmbiguous }`、`pub fn Council::open_round(&mut self, &mut Kernel, &str) -> CoreResult<RoundState>`、`pub fn Council::cast_vote(&mut self, &mut Kernel, Vote) -> CoreResult<RoundState>`
- **验收**：n=4 → f=1 quorum=3，第 3 张赞成票出结论；n=7=3f+1 → f=2 quorum=5=2f+1，缺席 2 人仍出结论；no≥quorum → rejected；全票弃权不产生结论；同轮同选择重复投票被拒且本轮票数不变；双签 → void_ambiguous 整轮作废、动议回 open、作废轮不再收票、重开可通过、留痕；非委员投票 → unauthorized（单次即恶意）；篡改签名 → InvalidSignature；未来轮次/已关闭轮次 → InvalidVersion（stale_epoch）；计票自洽：票数守恒、participation≤n、yes/no 不可能同时达法定人数
- **证据**：`cargo test -p au4a-council` → 51/51，等级 verified（本机实测）（EXIT=0；单元 20 + election 8 + proposal 10 + voting 10 + doc-tests 3，本 crate 0 warning）
- **状态**：✅ 已交付

#### v1.7.4 执行引擎

- **目标**：把已通过的决议落成真实状态变更（策略/信誉/账本），只能执行 passed、执行者须为在任委员、幂等且守恒
- **交付物**：`src/execution.rs: ExecutionEffect / ExecutionReceipt / execute() 校验+落地+收据`、`src/lib.rs: Council::execute / policy / policies / execution / executions / pub(crate) set_policy`、`tests/execution.rs: 10 个集成测试`
- **接口**：`pub enum ExecutionEffect { PolicySet, ReputationSet, Transferred, Slashed }`、`pub struct ExecutionReceipt { proposal, committee, executor, at, effects, total_before, total_after, slashed_before, slashed_after, conservation_ok }`、`pub fn ExecutionReceipt::ledger_effect_consistent() -> bool`、`pub fn Council::execute(&mut self, &mut Kernel, &AgentIdentity, &str) -> CoreResult<ExecutionReceipt>`
- **验收**：通过的决议落成策略写入 + executed 状态 + 收据 + 事件；非委员执行 → InvalidSignature+unauthorized；未注册 → UnknownAgent；open / rejected 动议不可执行且零状态变更；幂等：第二次执行被拒，收据不变；转账执行改变双方余额且总量不变、守恒成立；余额不足的转账执行失败后不留部分状态；罚没有界（锁定不足即拒）、罚没量入 slashed、总量恰好减少；信誉执行写入治理账并影响下一次选举花名册；自检 council.executions.ledger_effects / state_matches 一一对应
- **证据**：`cargo test -p au4a-council` → 61/61，等级 verified（本机实测）（EXIT=0；单元 20 + election 8 + proposal 10 + voting 10 + execution 10 + doc-tests 3，本 crate 0 warning）
- **状态**：✅ 已交付

#### v1.7.5 否决权

- **目标**：人类唯一的写能力是否决：类型层面无 propose/edit/cast_vote/execute，只能阻断，理由必须公开
- **交付物**：`src/veto.rs: Veto（私有字段+只读访问器）/ HumanVeto / VetoReceipt / apply / vetoable / is_blocked + 2 个 compile_fail 文档测试`、`src/human.rs: HumanObserver::veto 唯一写能力 + HumanView.vetoes + compile_fail 扩到 4 段`、`src/lib.rs: Council::apply_veto / veto_record / vetoes + 否决自检 + scenario 否决分支`、`tests/veto.rs: 8 个集成测试`
- **接口**：`pub struct Veto  // 只有 id/observer/target/reason/at/payload 只读方法`、`pub fn HumanObserver::veto(&self, &Council, &str, reason) -> CoreResult<Veto>`、`pub fn Council::apply_veto(&mut self, &mut Kernel, &Veto) -> CoreResult<VetoReceipt>`、`pub struct VetoReceipt { veto, proposal, previous_state, state, reason, at }`、`pub fn veto::{vetoable, is_blocked, vetoed_by, label_has_no_did, observer_is_not_an_agent}`
- **验收**：否决把 passed 动议推入 blocked，决议内容/动议总数/策略表不变；阻断后不可执行、不可复活、不可重开轮次、不可二次否决；空理由在铸造阶段被拒；理由进入事件与只读投影；open 动议也可否决，同样只通向 blocked；已执行动议不可被事后否决（否决不能回滚）；否决不存在的动议 → UnknownAgent，无记录；Veto 载荷不含 action/patch/alternative/payload/value/sig；6 段 compile_fail 文档测试：HumanObserver 无 propose/edit_proposal/cast_vote/execute，Veto 无 propose/edit_proposal
- **证据**：`cargo test -p au4a-council` → 73/73，等级 verified（本机实测）（EXIT=0；单元 20 + election 8 + proposal 10 + voting 10 + execution 10 + veto 8 + doc-tests 7（6 compile_fail + 1 正例），本 crate 0 warning；人类无密钥这一限制已在 md 中明说）
- **状态**：✅ 已交付

#### v1.7.6 链上治理

- **目标**：把治理状态投影成 GovernorToken 语义数据结构（只做语义映射，不假装有链；真实链上执行属 v1.8）
- **交付物**：`src/ongov.rs: GovState / ChainBinding / GovVeto / GovExecution / GovernorToken::project / project_all / no_false_chain_claims / state_summary`、`src/lib.rs: 自检 council.ongov.mapping 与 council.ongov.no_false_chain、results_json().ongov、scenario 的 governor_tokens`、`tests/ongov.rs: 6 个集成测试（+ ongov.rs 内 3 个单元测试）`
- **接口**：`pub enum GovState { Pending, Active, Canceled, Defeated, Succeeded, Queued, Executed }`、`pub struct GovernorToken { id, proposal, proposer, committee, snapshot_epoch, for_votes, against_votes, abstain_votes, quorum_numerator, quorum_denominator, state, veto, execution, binding }`、`pub fn GovernorToken::project(&Council, &str) -> CoreResult<GovernorToken>`、`pub struct ChainBinding { chain, grade, real_chain, note }`、`pub fn ongov::{project_all, no_false_chain_claims, state_summary}`
- **验收**：生命周期映射 pending → active → succeeded → executed；Canceled（带否决理由）与 Defeated 可区分；双签作废轮不产生链上结论，动议仍 active；绑定恒为 cpu-proto / real_chain=false，伪造绑定会被 claims_real_chain 抓出；投影确定性、JSON 往返、整数法定人数分数；scenario 两条动议投影为 executed 与 canceled，tx_ref 全为 null
- **证据**：`cargo test -p au4a-council` → 82/82，等级 cpu-proto（语义原型）（EXIT=0；单元 23 + election 8 + execution 10 + ongov 6 + proposal 10 + veto 8 + voting 10 + doc-tests 7，本 crate 0 warning；本版主动降级为 cpu-proto：只有语义映射，没有真实链上执行与真实 tx_ref）
- **状态**：✅ 已交付

#### v1.7.7 测试

- **目标**：把「治理过程是否始终自洽」变成可执行可复现的证伪器：17 条不变式 + 确定性重放
- **交付物**：`src/invariants.rs: INVARIANT_NAMES(17) / state_digest / check_all / replay(seed,steps) / replay_report`、`src/lib.rs: 自检 council.replay.invariants 与 council.replay.coverage、results_json().invariants`、`tests/invariants.rs: 8 个集成测试`
- **接口**：`pub const INVARIANT_NAMES: [&str; 17]`、`pub fn state_digest(&Council) -> CoreResult<String>`、`pub fn check_all(&Council) -> Vec<SelfCheck>`、`pub fn replay(seed: u64, steps: u32) -> CoreResult<ReplayReport>`、`pub struct ReplayReport { seed, steps, applied, proposals, rounds, voided_rounds, executed, blocked, rejected, refusals, invariants_per_step, violations, digest }`
- **验收**：同一种子两次重放逐字段相同（含状态摘要）；4 种子 × 90 步 0 违例，且执行/阻断/双签作废/否决四类结局全部被真实触发；不同种子给出不同治理历史；state_digest 随每次状态变更变化，只读观察不改变它；不变式清单与实现严格相等（双向断言）；steps=0 是合法空操作且摘要可复现；自检携带重放证据，results_json 暴露清单长度与三种子报告
- **证据**：`cargo test -p au4a-council` → 90/90，等级 verified（本机实测）（EXIT=0；单元 23 + election 8 + execution 10 + invariants 8 + ongov 6 + proposal 10 + veto 8 + voting 10 + doc-tests 7，本 crate 0 warning；自检真的跑 72 步重放（刻意保留成本））
- **状态**：✅ 已交付

#### v1.7.8 文档

- **目标**：能力清单机器可校验：文档里每条能力都必须指向源码里真实存在的测试，否则测试变红
- **交付物**：`src/claims.rs: Claim / CLAIMS(15) / claims_are_well_formed / claims_json / summary`、`tests/claims.rs: 6 个测试（含扫描源码核对测试名、读 1.7.json 校验元数据、读 1.7.md 校验小节）`、`src/lib.rs: results_json().claims`
- **接口**：`pub struct Claim { id, capability, interface, test, grade, note }`、`pub const CLAIMS: [Claim; 15]`、`pub fn claims_are_well_formed() -> bool`、`pub fn claims_json() -> serde_json::Value`、`pub fn summary() -> String`
- **验收**：清单自洽（id 唯一、字段非空、verified 有测试名、cpu-proto 有边界说明、无 unverified）；15 条能力的测试名在 src/tests/examples 源码里真实存在（跨文件扫描）；verified 条目的测试必须存在；cpu-proto 条目必须点明哪里不是真的；claims_json 与清单逐条一致；docs/tracks/1.7.json 版本连续、等级合法、状态 done、测试数自洽；docs/tracks/1.7.md 覆盖每个已完成版本的小节
- **证据**：`cargo test -p au4a-council` → 99/99，等级 verified（本机实测）（EXIT=0；单元 26 + claims 6 + election 8 + execution 10 + invariants 8 + ongov 6 + proposal 10 + veto 8 + voting 10 + doc-tests 7，本 crate 0 warning；文档成为通过条件（缺小节即红））
- **状态**：✅ 已交付

#### v1.7.9 示例

- **目标**：可运行示例 governance_demo + 安全委员会紧急通道（即时生效、事后治理确认、否决/超期回滚）+ scenario 补齐双签作废
- **交付物**：`src/emergency.rs: EmergencyStatus / EmergencyApproval / EmergencyConfirmation / EmergencyDirective / issue / confirm / rollback`、`src/lib.rs: CouncilConfig::emergency_confirm_window、issue_emergency / confirm_emergency / emergency(_directives)、自检 council.emergency.security_only 与 council.emergency.resolved`、`examples/governance_demo.rs: 可运行示例（scenario/自检/结果/清单/重放）`、`tests/emergency.rs: 7 个集成测试`、`scenario: 双签作废一轮→重开通过；紧急指令即时生效→事后确认`
- **接口**：`pub enum EmergencyStatus { Active, Confirmed, Rejected, Expired }`、`pub struct EmergencyDirective { id, committee, issued_by, key, value, reason, issued_at, confirm_deadline, previous_value, status, confirmation }`、`pub fn Council::issue_emergency(&mut self, &mut Kernel, &AgentIdentity, &str, i64, &str) -> CoreResult<EmergencyDirective>`、`pub fn Council::confirm_emergency(&mut self, &mut Kernel, &str, &[EmergencyApproval]) -> CoreResult<EmergencyDirective>`、`examples/governance_demo.rs`
- **验收**：只有安全委员会可下发（其他委员会 → unauthorized，零状态变更）；指令即时生效（0 票时策略已变）；票数不足保持 active；达 quorum → confirmed；被治理否决 → rejected 且策略回滚到先前值；超过确认窗口 → expired 且回滚（此前无该策略则删除）；确认票篡改/非委员/重复确认分别被拒；scenario 全流程：刷票无效→双签作废(2 轮)→通过→执行→人类阻断→紧急确认；示例源码可运行且不读文件/不开网络/不读墙钟
- **证据**：`cargo test -p au4a-council` → 106/106，等级 verified（本机实测）（EXIT=0；单元 26 + claims 6 + election 8 + emergency 7 + execution 10 + invariants 8 + ongov 6 + proposal 10 + veto 8 + voting 10 + doc-tests 7，示例由 cargo test 编译保证可构建，本 crate 0 warning）
- **状态**：✅ 已交付

#### v1.7.10 审计

- **目标**：治理事件串成不可悄悄改写的哈希链，并可只读导出提案/否决/紧急指令/执行/策略快照
- **交付物**：`src/audit.rs: AuditLog(rebuild/from_entries/verify/root) / AuditVerdict / AuditExport / 各类快照 / audit_checks`、`src/lib.rs: Council::audit_log / audit_export、results_json().audit、scenario 的 audit.export`、`src/invariants.rs: 不变式清单扩到 20 条（council.audit.chain_verifies）`、`src/claims.rs: 能力清单扩到 19 条（audit.hash_chain / audit.readonly_export）`、`tests/audit.rs: 7 个集成测试`
- **接口**：`pub struct AuditEntry { seq, at, kind, subject, detail, prev_hash, entry_hash }`、`pub struct AuditLog;  // rebuild(&Council) / from_entries(Vec<AuditEntry>) / verify() -> AuditVerdict / root()`、`pub struct AuditVerdict { ok, checked, broken_at, reason }`、`pub struct AuditExport { track, state_digest, audit_root, audit_entries, chain_ok, proposals, vetoes, emergency, executions, policies, invariants }`、`pub fn Council::audit_log() / audit_export()`
- **验收**：链由事件重建、条数与事件数一致、链根 64 位 hex、重建确定性；改写任意一条 → 断链且 broken_at 指向该条；删除/交换/改写 subject → 断链并指出第一处位置；空链可验证（checked=0，链根=创世哈希）；只读导出不改变治理状态（导出前后 state_digest 相同）且携带全部快照；审计自检项出现在 self_check() 与 results_json() 中；GovernorToken 只映射动议，与紧急指令互不混淆
- **证据**：`cargo test -p au4a-council` → 113/113，等级 verified（本机实测）（EXIT=0；单元 26 + claims 6 + audit 7 + election 8 + emergency 7 + execution 10 + invariants 8 + ongov 6 + proposal 10 + veto 8 + voting 10 + doc-tests 7，本 crate 0 warning）
- **状态**：✅ 已交付

### v1.8 Cross-Chain Settlement 跨链结算（`au4a-chain`）

#### v1.8.1 RGB集成

- **目标**：把 RGB 客户端验证语义（创世/密封转移/验证/最终化）做成确定性测试网适配器，并落地双轨守恒与 fail-closed 对账。
- **交付物**：`src/testnet.rs: ChainId / ChainTx / Block / Receipt / Testnet(accept, mine, mine_to, reorg, rebuild_nonces) / ChainRefusal / RefusalSpec / ONCHAIN_GRADE`、`src/bridge.rs: BridgeBook(bridge_out, bridge_in, require_consistent, force_chain_supply) / Reconciliation / reconcile`、`src/rgb.rs: Seal / RgbContract / TransferBundle / RgbAdapter / RGB_SUPPORTED / RGB_REFUSALS`、`src/lib.rs: 5 项自检（证据等级、具名拒绝、双轨守恒、fail-closed、接线）+ 场景`
- **接口**：`pub fn Testnet::accept(&mut self, tx: &ChainTx) -> Result<Receipt, ChainRefusal>`、`pub fn Testnet::reorg(&mut self, depth: u64) -> Result<Vec<String>, ChainRefusal>`、`pub fn BridgeBook::bridge_out(&mut self, ledger: &mut Ledger, who: &Did, amount: Credits, asset: &str, rail: &str) -> CoreResult<BridgeEvent>`、`pub fn BridgeBook::require_consistent(&self, ledger: &Ledger) -> CoreResult<Reconciliation>`、`pub fn RgbAdapter::execute(&mut self, net: &mut Testnet, tx: &ChainTx) -> Result<RgbOutcome, ChainRefusal>`
- **验收**：篡改交易 → Malformed；跨链 → Malformed；重放 → Conflict；回退 nonce → StaleEpoch（每条都带 op 名且被记录）；收据恒为 cpu-proto 且 real_network=false；最终性深度 BTC 6 / ETH 12；reorg 超过最终性被拒；RGB 创世 1000 流通量 1000；密封转移 400→250/150 后流通量不变；二次发行 → Conflict；篡改转移包 → Malformed；双花封印 → Conflict；Σ输入≠Σ输出 → Malformed；未达最终性 finalize → Timeout，叠够深度后成功，重复最终化 → Conflict；5 个具名拒绝（atomic_swap/lightning_route/unanchored_issuance/consensus_bridge/未知 op）逐个断言 op 与 code 且 refusals 计数为 5；双轨：本地托管 == 链上表示 == RGB 流通量；人为制造不一致 → require_consistent 拒绝且账本不被改写
- **证据**：`cargo test -p au4a-chain (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-chain)` → 25/25，等级 cpu-proto（语义原型）（确定性测试网语义原型：无真实 BTC/ETH 节点、无真实承诺方案、无共识层、不联网；链上证据等级恒为 cpu-proto；日志 _forangent\logs\1.8-v1.8.1.log）
- **状态**：✅ 已交付

#### v1.8.2 Taproot Assets

- **目标**：资产锚定语义：Merkle 承诺 + 输出键派生 + 认证路径验证 + 最终性门槛；具名拒绝脚本路径花费/Schnorr 多签/增发/链下互换。
- **交付物**：`src/taproot.rs: ProofStep / leaf_hash / merkle_root / merkle_proof / verify_merkle / TaprootAnchor / TaprootAdapter / TAPROOT_SUPPORTED / TAPROOT_REFUSALS`、`src/lib.rs: 自检 taproot.merkle + 场景 taproot 段（锚定 250/150、证明、验证、双轨对账）`
- **接口**：`pub fn merkle_root(leaves: &[String]) -> CoreResult<String>`、`pub fn merkle_proof(leaves: &[String], index: usize) -> CoreResult<Vec<ProofStep>>`、`pub fn verify_merkle(leaf: &str, proof: &[ProofStep], root: &str) -> CoreResult<bool>`、`pub fn TaprootAnchor::output_key_for(internal_key: &str, merkle_root: &str) -> CoreResult<String>`、`pub fn TaprootAdapter::execute(&mut self, net: &mut Testnet, tx: &ChainTx) -> Result<TaprootOutcome, ChainRefusal>`
- **验收**：锚定 250+150 → 总额 400、2 叶子、输出键可由内部键与根重新派生、Merkle 根可重算；5 个叶子的认证路径全部成立；换叶子/篡改路径 → 不成立；越界下标 → InvalidKind；锚定未达最终性就验证 → Timeout；叠够 finality_depth 后验证通过；伪造叶子 → Malformed；重复锚定同一资产 → Conflict；空金额/坏内部键 → Malformed；5 个具名拒绝（script_path_spend/schnorr_multisig/asset_inflation/offchain_swap/未知）逐个断言 op 与 code；Taproot 锚定总量 400 == 本地托管 400，require_consistent 通过
- **证据**：`cargo test -p au4a-chain (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-chain)` → 33/33，等级 cpu-proto（语义原型）（无 BIP341 真实 tweak、无真实签名、无 BTC 节点；Merkle 与哈希为真实运算，其余是测试网语义；日志 _forangent\logs\1.8-v1.8.2.log）
- **状态**：✅ 已交付

#### v1.8.3 ERC-8004

- **目标**：ETH 侧身份/声誉/验证注册表语义：DID 绑定身份不可转让、整数基点反馈（自评不计入）、验证背书、只读信誉摘要。
- **交付物**：`src/erc8004.rs: Identity / Feedback / Validation / ReputationSummary / Erc8004Adapter / reputation_weight_bp / summary_credits / ERC8004_SUPPORTED / ERC8004_REFUSALS`、`src/lib.rs: 自检 erc8004.reputation_reads_back + 场景 erc8004 段（身份注册、反馈、自评被拒、摘要）`
- **接口**：`pub fn Erc8004Adapter::execute(&mut self, net: &mut Testnet, tx: &ChainTx) -> Result<Erc8004Outcome, ChainRefusal>`、`pub fn Erc8004Adapter::summary(&self, agent_id: &str) -> CoreResult<ReputationSummary>`、`pub fn reputation_weight_bp(summary: &ReputationSummary) -> i64`、`pub fn summary_credits(summary: &ReputationSummary) -> CoreResult<Credits>`
- **验收**：身份绑定 DID 且不可重复注册（重复 → Conflict）；agent_id = H(did‖nonce‖std)；9000/8000/7000 → 平均 8000bp（整数除法）、权重 4800bp；3333/3333/3334 → 平均 3333；自评 → policy_denied（op=erc8004.self_feedback）；未注册客户 → Unauthorized；同客户第二条有效反馈 → Conflict；撤销后计数归零并可重发；重复撤销 → Conflict；分数 10001/-1 → Malformed；未知身份 → Conflict；summary 未知身份 → UnknownAgent；5 个具名拒绝（transfer_identity/self_feedback/mint_unbounded/set_operator/load_remote_registry/未知）逐个断言 op 与 code；摘要两次读取相等且 JSON 无浮点（grade=cpu-proto）
- **证据**：`cargo test -p au4a-chain (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-chain)` → 41/41，等级 cpu-proto（语义原型）（注册表语义原型：无真实合约、无事件日志、无 gas、无 EVM；日志 _forangent\logs\1.8-v1.8.3.log）
- **状态**：✅ 已交付

#### v1.8.4 x402

- **目标**：402 支付流程语义：开票-支付-最终性-领取；金额必须完全相等；领取走双轨托管解锁（fail-closed）。
- **交付物**：`src/x402.rs: Invoice / Payment / X402Adapter(execute, execute_with_ledger, claim, invoice_of, last_invoice, is_settled, to_json) / X402_SUPPORTED / X402_REFUSALS`、`src/lib.rs: 场景 x402 段（开票 100 → 支付 → 托管 100 → 最终性 → 领取）`
- **接口**：`pub fn Invoice::new(payee: &Did, amount: Credits, asset: &str, nonce: u64, expires_at: u64, memo: &str) -> CoreResult<Invoice>`、`pub fn X402Adapter::execute(&mut self, net: &mut Testnet, tx: &ChainTx) -> Result<X402Outcome, ChainRefusal>`、`pub fn X402Adapter::execute_with_ledger(&mut self, net: &mut Testnet, ledger: &mut Ledger, book: &mut BridgeBook, tx: &ChainTx) -> Result<X402Outcome, ChainRefusal>`、`pub fn X402Adapter::claim(&mut self, net: &mut Testnet, ledger: &mut Ledger, book: &mut BridgeBook, tx: &ChainTx) -> Result<X402Outcome, ChainRefusal>`
- **验收**：发票 id 内容寻址并绑定全部条款；改一个条件即不同 id；199 → partial_payment/Malformed；201 → Malformed；过期 → Timeout；重复支付 → Conflict；未达最终性领取 → Timeout；叠够深度后领取成功：链上表示与托管归零、服务方 +200、付款方 800、守恒成立；重复领取 / 未支付领取 → Conflict；无发票付款 → pay_without_invoice/Malformed；5 个具名拒绝（partial_payment/stream_payment/fiat_onramp/offchain_settle/未知）逐个断言 op 与 code；托管缺失时领取被 fail-closed 拒绝且服务方余额为 0；投影 JSON 无浮点且 grade=cpu-proto
- **证据**：`cargo test -p au4a-chain (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-chain)` → 51/51，等级 cpu-proto（语义原型）（无真实 HTTP、无真实结算层；发票/支付/领取为本地确定性状态机；日志 _forangent\logs\1.8-v1.8.4.log）
- **状态**：✅ 已交付

#### v1.8.5 结算路由

- **目标**：按金额/时效/费用阈值在 internal/rgb/taproot/x402 之间选路；fail-closed 为决策表第一优先级；结算前中后各对账一次。
- **交付物**：`src/routing.rs: Rail / Urgency / RouteAction / DecisionReason / RailTerms / RoutingTable / SettlementRequest / RouteDecision / SettlementOutcome / route / settle`、`src/lib.rs: 自检 routing.fail_closed_first + 场景 routing 段（400 → x402 决策 + 不一致时缓办）`
- **接口**：`pub fn route(request: &SettlementRequest, table: &RoutingTable, reconciliation_ok: bool) -> CoreResult<RouteDecision>`、`pub fn settle(ledger: &mut Ledger, book: &mut BridgeBook, request: &SettlementRequest, table: &RoutingTable) -> CoreResult<SettlementOutcome>`、`pub enum Rail { Internal, Rgb, Taproot, X402 }`、`pub fn RoutingTable::default_table() -> RoutingTable`
- **验收**：双轨不一致 → defer/reconciliation_failed（费用 0、到账不变），即使请求本身完全合格；1000 → 最便宜轨 x402（60bp）：费用 6、到账 994、eta 12；偏好 RGB 合格时走 RGB（费用 8）；时效 9 → deadline_too_tight（required_slack=30、eta=0）；费率上限 50bp → fee_above_threshold；2000000 → rail_unavailable；零额 → ZeroAmount；链上结算：托管 1000 == 链上表示 1000，桥回后双轨归零、收款方 +1000、守恒成立；内部结算不动链上轨；故障注入后 settle 返回 Err 且账本无新动账；缓办时 LedgerView 不变；4 金额 × 4 时效的决策可复现且 JSON 无浮点（grade=cpu-proto）
- **证据**：`cargo test -p au4a-chain (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-chain)` → 63/63，等级 cpu-proto（语义原型）（路由与账务为真实整数逻辑；链上仍是确定性测试网；日志 _forangent\logs\1.8-v1.8.5.log）
- **状态**：✅ 已交付

#### v1.8.6 信誉桥接

- **目标**：链上声誉事件 → 本地 4 维信誉（可靠性/质量/诚实/可用性）；不可转让、只认最终化事件、有界更新且不过冲。
- **交付物**：`src/reputation.rs: ReputationDim / ReputationEventKind / ChainReputationEvent / LocalReputation / ReputationBridge / credibility_credits / refusal_code_for / MAX_STEP_BP / NEUTRAL_BP / REPUTATION_REFUSALS`、`src/lib.rs: 自检 reputation.non_transferable_and_bounded + 场景 reputation 段（2 个事件 + 转让被拒）`
- **接口**：`pub fn ChainReputationEvent::new(agent: &Did, kind: ReputationEventKind, score_bp: i64, height: u64) -> CoreResult<ChainReputationEvent>`、`pub fn ReputationBridge::apply_event(&mut self, net: &Testnet, event: &ChainReputationEvent) -> Result<LocalReputation, ChainRefusal>`、`pub fn ReputationBridge::execute(&mut self, op: &str) -> Result<(), ChainRefusal>`、`pub fn LocalReputation::overall_bp(&self) -> i64`
- **验收**：反馈 9000（权重 6000）→ 质量 7000、可靠性不动；结算事件 8000（权重 4000）→ 可靠性与可用性各 6200；未最终化事件 → Timeout 且信誉保持中性、事件计数 0；重放 → Conflict；篡改 id → Malformed；30 次满分事件单调上升且 ≤10000，收敛到 1bp 以内；反向 30 次收敛到 0..=1；罚没事件 → 可靠性与诚实各降 2000（单步上限），质量不动；reputation.transfer/buy/bulk_import/reset/未知 逐个按名字拒绝并留痕；两个桥跑同样事件的投影逐字段相等、JSON 无浮点、transferable=false、grade=cpu-proto
- **证据**：`cargo test -p au4a-chain (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-chain)` → 71/71，等级 cpu-proto（语义原型）（映射与有界更新为真实整数逻辑；事件来源仍是确定性测试网；日志 _forangent\logs\1.8-v1.8.6.log）
- **状态**：✅ 已交付

#### v1.8.7 测试

- **目标**：跨模块不变量与端到端测试：拒绝清单完备、收据恒 cpu-proto、双轨守恒随机序列、双花/重放/最终性回滚、场景逐字节复现。
- **交付物**：`tests/chain_invariants.rs: 9 个跨模块测试（拒绝清单、未知 op、等级、随机双轨序列、双花重放、最终性、fail-closed、信誉边界、内容寻址）`、`tests/e2e_settlement.rs: 7 个端到端测试（section 齐全、逐字节复现、递归等级检查、钉住数值、观察层拒绝、自检接线、篡改拦截）`、`src/testnet.rs: 最终性改为冻结语义（finalized_height 单调不减）`、`src/lib.rs: results_json 模块清单补齐 8 个模块；场景 refusals 数组补齐 4 条`
- **接口**：`cargo test -p au4a-chain 同时运行 lib + 2 个集成测试目标`、`pub fn Testnet::is_final(&self, height: u64) -> bool // 冻结语义`
- **验收**：4 个适配器 + 信誉桥的拒绝清单：op 唯一、原因非空；未知 op → unsupported 且列出已知清单；所有成功收据 grade == cpu-proto；场景 JSON 递归检查 grade 与 real_network；200 步随机桥接：每步守恒、托管==链上表示、本地总量恒为 10000；测试网重放与 RGB 封印双花均 → Conflict；未最终化可回滚、已最终化不可回滚且最终性不撤销；5 个金额档在双轨不一致时全部 defer/reconciliation_failed；故障注入下 settle 返回 Err 且账本不动；信誉单步位移 ≤2000bp、单调、收敛到 1bp 以内；场景两次运行逐字节相同
- **证据**：`cargo test -p au4a-chain (CARGO_TARGET_DIR=E:\DS\_forangent\target\au4a-chain)` → 87/87，等级 cpu-proto（语义原型）（lib 71 + chain_invariants 9 + e2e_settlement 7；本版修正最终性为冻结语义；日志 _forangent\logs\1.8-v1.8.7.log）
- **状态**：✅ 已交付

#### v1.8.8 文档

- **目标**：汇总文档：模块地图、测试网语义表、双轨与 fail-closed、具名拒绝清单索引、证据分级与明确的未做清单。
- **交付物**：`crates/au4a-chain/README.md: 模块地图 / 测试网语义 / 双轨规则 / 拒绝清单索引 / 证据分级 / 本地验证`、`src/lib.rs: crate 级 rustdoc 模块地图与最短上手路径`、`docs/tracks/1.8.md: 本汇总节`
- **接口**：`文档不改任何 public API`
- **验收**：模块地图逐版给出模块与关键 public 项，与代码一致；拒绝清单索引覆盖 RGB/Taproot/ERC-8004/x402/信誉 五份清单，含例子与拒绝码；证据分级表每项标注 cpu-proto，并单列「真实链上执行 = 未做」；不声称任何未实现能力（无真实节点/承诺/EVM/HTTP/Lightning）
- **证据**：`cargo test -p au4a-chain` → 87/87，等级 cpu-proto（语义原型）（文档版不新增代码路径，测试结果沿用 v1.8.7 全绿运行；日志 _forangent\logs\1.8-v1.8.7.log）
- **状态**：✅ 已交付

#### v1.8.9 示例

- **目标**：可运行的九步示例：测试网/双轨/RGB/Taproot/ERC-8004/x402/路由/信誉/自检，全部打印真实数字并输出场景 JSON。
- **交付物**：`examples/chain_tour.rs: 九步示例，main 返回 CoreResult<()>，主流程无 unwrap/panic`、`docs/tracks/1.8.md: 示例原始输出摘录（本机运行日志）`
- **接口**：`cargo run -p au4a-chain --example chain_tour`
- **验收**：示例 exit code 0，输出 9 行真实数字且与文档摘录一致；每步调用真实 public API（Testnet/BridgeBook/RgbAdapter/TaprootAdapter/Erc8004Adapter/X402Adapter/routing/ReputationBridge/self_check/scenario）；不读文件/不开网络/不读墙钟，同输入同输出；cargo test -p au4a-chain 仍 87 passed / 0 failed、0 warning（示例参与编译检查）
- **证据**：`cargo test -p au4a-chain; cargo run -p au4a-chain --example chain_tour` → 87/87，等级 cpu-proto（语义原型）（示例本机实测 exit 0；输出见 _forangent\logs\1.8-v1.8.9-example.log；测试日志 _forangent\logs\1.8-v1.8.9.log）
- **状态**：✅ 已交付

#### v1.8.10 安全审计

- **目标**：风险登记表（9 条，含未防护/部分防护的坦白：验证数据丢失、签名层、最终性概率、女巫刷分）+ 10 个真实攻击用例全部被拦住。
- **交付物**：`src/audit.rs: RiskStatus / Risk / RISKS / risk_register / unresolved_risks / AttackOutcome / attack_suite / audit_report`、`src/lib.rs: 自检 audit.attack_suite + 场景 audit 段`、`examples/chain_tour.rs: 第 10 步打印风险与未防护清单`、`README.md: 安全审计章节（风险表与残余风险摘要）`
- **接口**：`pub const RISKS: [Risk; 9]`、`pub fn unresolved_risks() -> Vec<Risk>`、`pub fn attack_suite() -> Vec<AttackOutcome>`、`pub fn audit_report() -> serde_json::Value`
- **验收**：风险清单含重放/双花/最终性/验证数据丢失/女巫/签名；未防护或部分防护 ≥3 条且如实保留；验证数据丢失与签名层明确标注 unprotected，并写明真实实现需要什么；10 条攻击（重放/nonce 回退/篡改/双花/最终化回滚/双轨不一致硬走/自评刷分/未最终化事件/信誉转让/重复支付）全部被拦住且 detail 非空；两次攻击运行结果完全相同；报告 JSON 无浮点且声明 scope 只覆盖确定性测试网；场景 audit.risks>=8、unresolved_count>=3、all_attacks_blocked=true
- **证据**：`cargo test -p au4a-chain; cargo run -p au4a-chain --example chain_tour` → 92/92，等级 cpu-proto（语义原型）（lib 76 + chain_invariants 9 + e2e_settlement 7；审计对象是确定性测试网原型，真实链上执行未做；日志 _forangent\logs\1.8-v1.8.10.log、1.8-v1.8.10-example.log）
- **状态**：✅ 已交付

### v1.9 Network Scaling 网络扩展（`au4a-scale`）

#### v1.9.1 缩放定律度量

- **目标**：把节点数/交互复杂度/算力预算 ↔ 群体智能水平形式化为整数度量，数学表达=代码=数值测试
- **交付物**：`src/metrics.rs: ScalingParams + 9 个度量函数 + analytic_vertex_floor（整数二分）`、`src/lib.rs: self_check（4 项真实断言）/ results_json / scenario（4 档位度量表）`
- **接口**：`pub struct ScalingParams { n0: u64, alpha: u32, p0_milli: i64, interaction_cost_milli: i64 }`、`pub fn effective_per_node_milli(n, params) -> CoreResult<i64>`、`pub fn aggregate_throughput_milli(n, params) -> CoreResult<i64>`、`pub fn orchestration_overhead_milli(n, params) -> CoreResult<i64>`、`pub fn net_throughput_milli / completion_bp / overhead_ratio_bp / marginal_gain_milli`、`pub fn analytic_vertex_floor(n0, alpha) -> u64`
- **验收**：手算一致：p(N0)=p0/2、p(100)=500、T(100)=50000、T(10)=9900；独立 u128 复算与实现逐点相等（n=1,7,50,100,250,1000）；α=2 有峰值、α=1 严格单调；解析极值点与离散 argmax 相差 ≤1（α=2→1000，α=3→793）；编排开销二次增长（比值 4.0±0.2 倍）；成本高时净收益为负、开销占比 >100%；完成率饱和 10000bp；demand=0 被拒；参数非法（n0=0/α∉[1,3]/p0≤0/c<0）被拒；所有输出为整数（可 canonicalize，无浮点）
- **证据**：`cargo test -p au4a-scale` → 8/8，等级 verified（本机实测）（2026-10-04 本机 Windows 实测，本 crate 0 warning；日志 _forangent\logs\1.9-v1.9.1.log；模型为解析式聚合（10k 节点为模型推论，cpu-proto，不冒充真实压测））
- **状态**：✅ 已交付

#### v1.9.2 实验框架

- **目标**：确定性实验框架：给定节点档位与参数，产出可复现的实验报告与规范摘要
- **交付物**：`src/harness.rs: ExperimentConfig / Row / ExperimentReport / run（含 digest）`、`src/lib.rs: scenario 使用框架产出报告`
- **接口**：`pub struct ExperimentConfig { nodes: Vec<u64>, params: ScalingParams, demand_milli: i64 }`、`pub struct Row { nodes, per_node_milli, aggregate_milli, overhead_milli, net_milli, completion_bp, overhead_ratio_bp, marginal_gain_milli }`、`pub struct ExperimentReport { config, rows, digest }`、`pub fn run(config: &ExperimentConfig) -> CoreResult<ExperimentReport>`
- **验收**：同一配置两次 run 的 rows 与 digest 完全相同；row 的每个字段与 metrics 函数逐个相等（框架不引入第二套算法）；空档位/重复档位/非法参数被拒；报告 JSON 可规范化（无浮点），digest == canonical_hash(rows)
- **证据**：`cargo test -p au4a-scale` → 14/14，等级 verified（本机实测）（2026-10-04 本机 Windows 实测，本 crate 0 warning；日志 _forangent\logs\1.9-v1.9.2.log；累计 14 个测试）
- **状态**：✅ 已交付

#### v1.9.3 大规模集群

- **目标**：10/100/1k/10k 节点的有界评估：秒级完成、内存有界（O(1) 分配，不随 N 增长）
- **交付物**：`src/cluster.rs: TierSpec / TierReport / 有界窗口扫描求顶点`、`tests 中的墙钟上界断言（仅测试读时钟）`
- **接口**：`pub const TIERS: [u64; 4] = [10, 100, 1000, 10000]`、`pub fn tier_report(params, demand_milli) -> CoreResult<TierReport>`、`pub fn bounded_vertex_scan(params, lo, hi, samples) -> CoreResult<VertexScan>`
- **验收**：10k 档位评估在 1 秒内完成（本机实测 + 测试墙钟断言，实际毫秒级）；求值次数 ≤ samples + span/step + 2·step + 3，与节点数无关（有两组范围的逐项上界断言）；有界扫描顶点与全量逐点扫描顶点相差 ≤ 1 个粗扫步长（三组参数对照）；档位行与 metrics 函数逐字段一致；报告确定性与 digest 一致；lo=0 / hi<lo / samples∉[2,4096] / demand=0 全部被拒
- **证据**：`cargo test -p au4a-scale` → 20/20，等级 verified（本机实测）（2026-10-04 本机 Windows 实测，本 crate 0 warning；日志 _forangent\logs\1.9-v1.9.3.log；模型为解析式聚合（10k 为模型评估，非真实分布式压测））
- **状态**：✅ 已交付

#### v1.9.4 效果评估

- **目标**：容量顶点裁决：完成率峰值 + 峰值后编排开销恶化 + 边际收益转负（含「收益转负」测试）
- **交付物**：`src/verdict.rs: CapacityVerdict / VerdictKind / VerdictReason / adjudicate`
- **接口**：`pub enum VerdictKind { VertexFound, MonotonicNoVertex, InsufficientRange }`、`pub struct CapacityVerdict { kind, vertex_nodes, peak_completion_bp, peak_aggregate_milli, peak_net_milli, overhead_ratio_at_peak_bp, overhead_ratio_after_peak_bp, marginal_negative_from, reasons }`、`pub fn adjudicate(params, demand_milli, lo, hi, step) -> CoreResult<CapacityVerdict>`
- **验收**：α=2 场景判定 VertexFound 且顶点与解析式相差 ≤ 1 个步长；峰值后开销占比严格上升（OverheadWorsened，比较用 ppm 保留分辨率）；边际收益在顶点后转负（MarginalTurnedNegative，含具体起始 n）；收益转负：交互成本 10 任务时 net_negative_from 给出首次转负点，该点净收益 < 0 且起点 > 0；α=1 场景判定 MonotonicNoVertex；区间过窄判定 InsufficientRange；同一输入裁决结果可复现（两次调用相等），JSON 可规范化
- **证据**：`cargo test -p au4a-scale` → 26/26，等级 verified（本机实测）（2026-10-04 本机 Windows 实测，本 crate 0 warning；日志 _forangent\logs\1.9-v1.9.4.log；开销占比比较改用 ppm（bp 在 O/T≈1e-5 时截断为 0，由测试发现））
- **状态**：✅ 已交付

#### v1.9.5 数据收集

- **目标**：确定性数据收集：把多个场景的观测记录成可复现的 bundle（内容寻址摘要）
- **交付物**：`src/collect.rs: Record / DataBundle / collect / sweep`
- **接口**：`pub struct Record { scenario, nodes, params, aggregate_milli, net_milli, completion_bp, overhead_ratio_bp }`、`pub struct DataBundle { records: Vec<Record>, digest: String }`、`pub fn collect(specs: &[ScenarioSpec]) -> CoreResult<DataBundle>`、`pub fn sweep(nodes, n0_values, alphas, ...) -> CoreResult<DataBundle>`
- **验收**：bundle 的 digest == canonical_hash(records)，且 recompute_digest 可独立复算；同一 spec 两次 collect 完全一致；bundle 可 JSON 往返且可规范化；空场景列表 / 空档位 / demand=0 / 非法参数在收集阶段就被拒；sweep 覆盖 |nodes|×|N0|×|α| 个组合且确定性；每条记录自带完整参数；改动任一参数即改变 digest（内容寻址）
- **证据**：`cargo test -p au4a-scale` → 31/31，等级 verified（本机实测）（2026-10-04 本机 Windows 实测，本 crate 0 warning；日志 _forangent\logs\1.9-v1.9.5.log；累计 31 个测试）
- **状态**：✅ 已交付

#### v1.9.6 分析工具

- **目标**：从观测数据反推模型参数（整数网格搜索）并给出残差报告
- **交付物**：`src/analyze.rs: FitReport / fit_by_grid_search / residual 统计`
- **接口**：`pub struct FitReport { n0_estimate, alpha_estimate, p0_estimate_milli, max_residual_ppm, mean_residual_ppm, samples, evaluated_candidates }`、`pub struct FitSearch { n0_min, n0_max, n0_step, alphas }（around(...) 构造）`、`pub fn fit(bundle: &DataBundle, search: &FitSearch) -> CoreResult<FitReport>`、`pub fn residuals(bundle: &DataBundle, params: &ScalingParams) -> CoreResult<Vec<i64>>（ppm）`
- **验收**：从 6 条合成观测恢复真值：α 精确命中 2，N0 误差 ≤ 50（真值 1000）；真值参数残差 ≤ 1000 ppm；换错参数残差显著变大（区分度）；样本 < 2 条或搜索空间非法时被拒（不给出无依据的拟合）；扫描数据能被拟合回被扫描的参数（N0 误差 ≤ 30）；拟合确定性（两次相等）且 JSON 可规范化
- **证据**：`cargo test -p au4a-scale` → 36/36，等级 verified（本机实测）（2026-10-04 本机 Windows 实测，本 crate 0 warning；日志 _forangent\logs\1.9-v1.9.6.log；累计 36 个测试）
- **状态**：✅ 已交付

#### v1.9.7 测试

- **目标**：端到端 + 确定性 + 性能上界 + 收益转负的独立测试套件
- **交付物**：`tests/scale_end_to_end.rs: 全链路一致性 / 确定性 / 有界性 / 收益转负 / scenario 复现`
- **接口**：`pub fn scenario(kernel: &mut Kernel) -> CoreResult<Value>（确定性契约）`
- **验收**：端到端：收集→拟合→裁决全链路断言（α 命中 2、N0 误差 ≤ 50、顶点与解析同一步长）；同一输入两次运行的 digest / verdict / scenario 输出完全一致；10k 档位 + 裁决 < 1s（实测墙钟断言），求值次数 ≤ 预算、档位数固定；收益转负：首次转负点净收益 < 0、起点 > 0；零成本场景不出现该原因码；档位报告与实验框架对同一档位给出同一数值
- **证据**：`cargo test -p au4a-scale` → 42/42，等级 verified（本机实测）（2026-10-04 本机 Windows 实测，本 crate 0 warning；日志 _forangent\logs\1.9-v1.9.7.log；lib 36 + 集成 6 = 42）
- **状态**：✅ 已交付

#### v1.9.8 文档

- **目标**：机器可读契约（度量/单位/裁决码/证据分级）与文档定稿
- **交付物**：`src/schema.rs: schema_json() / schema_summary()`、`docs/tracks/1.9.md 与 1.9.json 定稿`
- **接口**：`pub fn schema_json() -> CoreResult<Value>`
- **验收**：schema 与代码常量逐个对齐（3 类裁决 / 7 个原因码 / 4 档位 / 求值预算 / MAX_SCAN_SAMPLES）；每个原因码在 schema 中恰好出现一次；schema 是规范 JSON（无浮点）；claim_scope 明写「非真实分布式压测」，not_done 列出未做之事，limitations 4 条；摘要给出 10 条公式、4 个档位、4 条局限
- **证据**：`cargo test -p au4a-scale` → 45/45，等级 verified（本机实测）（2026-10-04 本机 Windows 实测，本 crate 0 warning；日志 _forangent\logs\1.9-v1.9.8.log；lib 39 + 集成 6 = 45）
- **状态**：✅ 已交付

#### v1.9.9 论文

- **目标**：论文稿：方法/指标/结果/局限，逐条标注模型推论 vs 本机实测
- **交付物**：`src/paper.rs: paper_json()（结论 + 证据分级）`、`docs/tracks/1.9.md 论文节（含局限与未做之事）`
- **接口**：`pub fn paper_json() -> CoreResult<Value>`
- **验收**：6 条结论全部带合法证据等级（verified 4 / cpu-proto 2），摘要计数与结论数一致；结论与实验数据一致（顶点/解析值/开销占比/拟合恢复值由测试现场重算并比对）；局限 5 条、未做 3 项，且明确写出无真实分布式压测、参数为本模型标定；论文稿可规范化（无浮点）
- **证据**：`cargo test -p au4a-scale` → 48/48，等级 verified（本机实测）（2026-10-04 本机 Windows 实测，本 crate 0 warning；日志 _forangent\logs\1.9-v1.9.9.log；lib 42 + 集成 6 = 48。论文数值部分为 verified（本机复算），10k 节点推论为 cpu-proto（非真实分布式压测））
- **状态**：✅ 已交付

## 六、人类观察层

| 面板 | 内容 | 数据来源 |
|---|---|---|
| 进度 | 活跃 Agent、协作中的任务、里程碑事件 | `Kernel::observe().progress` |
| 结果 | 任务完成情况、产出物、质量评估 | 各轨道 `results_json()` 与 `SelfCheck` |
| 收益 | 积分余额、变动记录、资源贡献对应关系 | `LedgerView`（发行 / 罚没 / 各账户可用与锁定） |

只读性由结构强制：观察面只注册 GET 路由，非 GET 一律 405，且不存在任何写路由（有枚举路由表的结构性测试）。

## 七、风险与缓解

| # | 风险 | 缓解 |
|---|---|---|
| 1 | Agent 自治导致不可预测的经济行为 | 经济参数边界（证据闸门、结算上限）+ 委员会监督（v1.7）+ 人类否决只阻断 |
| 2 | 状态迁移过程中的不一致 | 两阶段提交（prepare/commit/confirm）+ 失败回滚不留部分状态（v1.3） |
| 3 | 个体学习的隐私泄露 | 经验库本地化 + 对外公开视图脱敏（v1.6） |
| 4 | 跨链资产与验证数据风险 | fail-closed：账本为真相、不一致则拒绝；适配器具名拒绝清单（v1.8） |
| 5 | 文档与代码漂移 | 文档由版本清单 + 轨道元数据生成；证据分级 + 结构性测试（本项目十原则第 4、10 条） |
| 6 | 并行开发互相破坏 | 轨道零耦合、公共接口只增不改、独立 target 目录（DEV.md 第四节） |

## 八、验收标准

| 维度 | 标准 |
|---|---|
| 功能 | 99 个小版本的验收项全部有对应测试；`cargo test --workspace` 全绿 |
| 兼容 | 参考项目语义对齐处（账本守恒、BFT-lite、拒绝分类）有对照测试 |
| 性能 | 10k 节点缩放模拟秒级完成（v1.9）；能力图查询走索引（v1.1.5/1.1.8） |
| 安全 | 只读观察面无写路由；篡改签名/快照/合约必被拒；罚没不超过锁定余额 |
| 文档 | 本文档与 `docs/DEV.md` 覆盖 10 中版本 + 99 小版本，且由生成器强制完整性 |
| 平台 | Windows 本机实测 10 个中版本收尾点（见 `docs/VERIFICATION.md`） |

## 附：设计原则

# AU4A 设计原则（PRINCIPLES）

> 一切为智能体服务。人类用户兼任观察者，只展示进度、结果与收益。

这十条原则不是口号，每一条都在代码里有对应的结构或测试。凡是做不到的，就降级证据等级并写明，
而不是把愿望写成文档——参考项目的教训正是「文档声称的能力在代码里不可达」。

## 1. Agent 优先
任何新功能的第一个测试用例是「**Agent 能否自主使用它**」，而不是「人类能否操作它」。
如果只有人类能触发的路径，那这个功能还没有完成。

## 2. 人类只观察，且这一点由结构强制
`Kernel::observe()` 返回一个值；`au4a-node` 的观察面板**只挂 GET 路由**，非 GET 一律 405。
「人类不参与决策」不是文档承诺，而是路由表与类型系统的事实，并有结构性测试枚举断言。

## 3. 人类否决权只能阻断
v1.7 的 `Veto` 类型在类型层面没有 propose / edit 能力：否决可以让一项决议不生效，
但不能提出动议、不能修改内容、不能指定替代方案。理由必须公开记录。

## 4. 可证伪
文档里写的每条能力，代码里必须可达，并有测试覆盖。做不到就写 `cpu-proto` 或 `unverified`，
并说明哪一部分是原型。**禁止** `todo!()` 冒充实现，禁止「应该可以」。

## 5. 证据分级是结算闸门
`EvidenceGrade::{Verified, CpuProto, Unverified}`：`Unverified` **永远不可结算**，无论金额多小；
`CpuProto` 只能结算不超过阈值的小额。这样「没验证的东西」不会变成经济事实。

## 6. 确定性可重放
基元层与轨道层**不读墙钟**（时间来自 `LogicalClock`），不做文件/网络 I/O，不引入随机性
（随机一律由种子驱动）。同样的输入必须给出**逐字节相同**的输出——这是能被验证的前提。

## 7. 整数守恒账本
金额一律 `Credits(i64)` 微积分，禁止浮点参与经济计算（规范 JSON 直接拒绝浮点数）。
任意时刻可断言：`Σ可用 + Σ锁定 + 已罚没 == 已发行`。守恒不是文档承诺，是可运行的断言。

## 8. 类型化拒绝：不要把竞争当恶意
十个拒绝码里只有 `malformed` 与 `unauthorized` 单次出现即构成恶意证据；
其余八个（超时、限流、容量、冲突、过期纪元、降级、不支持、策略拒绝）都是竞争或容量语义，
单次只记警告，重复超阈值才升级。因为**「凡是拒绝就隔离」等于「凡是竞争就误伤」**。

## 9. 轨道隔离，只增不改
10 条轨道（`au4a-kernel` 到 `au4a-scale`）零耦合，只共享冻结基元 `au4a-core` 与宿主内核 `au4a-kernel`；
每条轨道在自己的 crate 内**串行**推进小版本，互不阻塞。公共接口只增不改，
改签名必须走新的中版本——这样 9 条并行轨道不会互相拆掉对方。

## 10. 诚实优先于漂亮
宁可写「这一版只做到内存级原型，真实网络未实现」，也不要写「已实现 P2P」。
发布物、README、DESIGN/DEV、VERIFICATION 里的每一个数字都必须来自真实运行结果，
并且能被别人用仓库里的命令复现。

## 附：人类观察层实现说明

# 人类观察层（OBSERVER）

> 人类只做三件事：**看进度、看结果、看收益**。这一页说明它是怎么被**代码结构**强制的，
> 而不是靠文档承诺——参考项目的教训正是「文档说只读，代码里到处是写入口」。

## 1. 三个面板

| 面板 | 人类看到什么 | 数据来源 | 端点 |
|---|---|---|---|
| 进度 | 活跃 Agent 数、已投递消息、拒绝记录、网络节拍、每条轨道的运行事件 | `Kernel::observe()` → `ObserverView.progress` | `GET /api/progress` |
| 结果 | 各轨道的 `results_json()` 产物摘要 + 10 条轨道自检（通过/未通过 + 证据说明） | `au4a_node::track_results()` + `self_checks_json()` | `GET /api/results`、`GET /api/selfcheck` |
| 收益 | 账本总量、已发行、已罚没、每个账户的可用与锁定余额 | `Kernel::observe().ledger`（`LedgerView`） | `GET /api/revenue` |

HTML 首页 `GET /` 把三块面板放在一屏里，每 2 秒自动重新 GET 一次（刷新本身也是读操作）。

## 2. 只读性如何被结构强制

1. **路由表是常量**：`au4a_node::observer::ROUTES` 是一个 `&'static [&'static str]`，全部以 `/` 开头，
   里面不存在任何写语义的名字。
2. **只接受 GET**：`ALLOWED_METHOD == "GET"`，服务端对任何非 GET 请求返回 **405 Method Not Allowed**，
   并在响应体里回 `{"error":"method_not_allowed","allowed":"GET"}`；未知路径返回 404。
3. **投影是值而不是句柄**：`Kernel::observe()` 返回 `ObserverView`（一个可序列化的值）。
   观察面拿到的是**快照字符串**（`SharedView`），Agent 侧 `update()` 是唯一写入方，人类侧只有 `snapshot()`。
4. **结构性测试**（`crates/au4a-node/src/observer.rs` 的 `#[cfg(test)]`）：
   * `every_route_is_a_get_route`：断言方法常量为 GET、路由表里不含任何写语义词；
   * `observer_serves_reads_and_refuses_writes`：真起服务 → GET 三个面板 200 → POST/PUT/PATCH/DELETE 全部 405
     → 未知路径 404 → **最后比对投影逐字节未变**（这才叫只读证明，不是「没写代码」）；
   * `malformed_requests_do_not_panic`：畸形请求不能把服务打挂。
5. **部署实测**：`tools/verify-mediums.mjs` 会在 10 个中版本收尾点上真起面板、真发 GET/POST，
   把状态码写进 `docs/verify/medium-vX.Y.md`。

## 3. 人类还剩什么权力

| 权力 | 在哪 | 边界 |
|---|---|---|
| 看 | 本观察层（HTTP GET） | 无限制 |
| 收益 | `LedgerView` 只读投影；兑换结算由 Agent 自主（v1.4 / v1.8） | 人类不参与定价与分配 |
| 否决 | `au4a-council`（v1.7）的否决权 | **只能阻断**，必须公开理由，类型层面没有提案/修改能力 |
| 资源 | 提供算力/存储/网络的初始额度（`KernelConfig`） | 只是初始条件，不是运行期决策 |

## 4. 启动方式

```bash
# 让 8 个 Agent 自主跑，并打开只读面板
cargo run -p au4a-node -- run --agents 8 --observe 127.0.0.1:8787

# 只看一次投影（CI / 脚本用）
cargo run -p au4a-node -- observe --once --json

# 聚合 10 条轨道的自检（部署验证的退出码来源）
cargo run -p au4a-node -- verify --json
```

---

> 本文档由 `tools/gen-docs.mjs` 从 `docs/versions.json` 与 `docs/tracks/*.json` 生成；请勿手改。
