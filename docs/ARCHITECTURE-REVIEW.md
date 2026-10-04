# 架构审查报告（ARCHITECTURE-REVIEW）

> 审查对象：`TwinsEarth/agent-universe` 与 `TwinsEarth/NewAgentUniverseByDeepSeek`（**只读**）。
> 完整调查记录：[recon/REF1-ARCHITECTURE.md](recon/REF1-ARCHITECTURE.md)、[recon/REF2-ARCHITECTURE.md](recon/REF2-ARCHITECTURE.md)。
> 结论用途：确定 AU4A（本项目）要**继承什么、反转什么、必须自己重做什么**。

## 一、审查方法与范围

| 做了什么 | 说明 |
|---|---|
| 只读浏览 | glob/grep/read 遍历两个仓库；引用处标 `路径` 或 `路径:行` |
| 只读构建 | REF2：`cargo check --workspace --all-targets` → **exit 0，3m25s，0 warning**（target 目录放在仓库外） |
| **没做** | 没有跑 `cargo test`（因此所有测试计数标注为「文档声称，未在代码验证」）；没有编译 Solidity；没有起 daemon；没有联网 |
| 没有改动 | 两个参考仓库逐字节未改（只读浏览 + 树外构建目录） |

## 二、关键发现

### P0 —— 直接决定 AU4A 架构的三条

1. **能力存在但主干不可达**（REF1）。`gsn-core/src/marketplace/` 有真实不变式（守恒账本、BFT-lite、
   质押罚没、4 维信誉、TaskSpec 六字段闸门），但 `AgentMarket` 只被自己的模块、`lib.rs`、测试与一个
   example 引用；daemon **没有任何市场端点**，市场/信誉/质押**完全不持久化**（SQLite 只存 19 个名片字段中的 5 个）。
   → AU4A：能力必须从「Agent 的一等入口」出发设计，而不是先写库再想让谁调用。
2. **导出的 API 和真正的实现是两套东西**（REF1）。`net/libp2p_node.rs`(`GsnNode`) 与 `net/gossip.rs`(`GossipSub`)
   是无网络的 HashMap，却从 `lib.rs:35` 作为门面导出；真正基于 libp2p 0.54 的实现藏在 `lib.rs:83-91`。
   → AU4A：**门面必须就是实现**，`au4a-node` 调用的 `scenario()`/`self_check()` 必须是轨道真实代码路径。
3. **文档承诺与实际能力脱节**（两个参考项目都有）。REF1 的 RGB/Taproot/x402/ERC-8004 全仓库 0 命中；
   4 个 Solidity 文件从不编译（CI 只 grep `pragma`）；REF2 的 `ARCHITECTURE.md`/`VERIFICATION.md`/`NOTICE`
   仍写 V1.2.3 而 `VERSION` 是 3.8.0。
   → AU4A：证据分级（`unverified` 不可结算）+ 由版本清单生成的文档 + 结构性测试。

### P1 —— 工程债与风险

| # | 发现 | 出处 | 对 AU4A 的影响 |
|---|---|---|---|
| 1 | 账本用**内存 f64**，守恒只能靠文档承诺 | REF1 marketplace | AU4A 用整数微积分 + 可运行守恒断言 |
| 2 | 同一套不变式有 **4 份实现**（Rust×2、JS、Python） | REF1 | AU4A 一份实现 + 冻结基元 |
| 3 | 版本号五处互相矛盾（1.2.3 / 2.3.4 / 0.2.35 / `/gsn/0.2.34` / 2.3.5） | REF1 | AU4A 版本清单是唯一真相（`docs/versions.json`） |
| 4 | 零 daemon/HTTP 集成测试；零 Solidity 测试 | REF1 | AU4A 的部署验证必须真起进程、真发 HTTP |
| 5 | `verifier/client.rs` 恒返回 `valid=true/0.95`；`erasure/` 不是 Reed-Solomon；`storage/sqlite.rs` 是 HashMap；`chain/pocv.rs` 是哈希相等 | REF1 | 名字不能当证据：AU4A 每条能力都要指出真实代码路径 |
| 6 | 十个跨链名词在 REF2 **0 命中**（`ERC-8004, x402, L402, Lightning, Taproot, RGB, HTLC, USDC, ERC-4337, Paymaster`） | REF2 | v1.8 是最容易造假的地方：一律标 `cpu-proto` 并写清「本地确定性测试网适配器，非真实链」 |
| 7 | 能力是**扁平集合**：没有边、委派、衰减、过期 | REF2 | v1.1 能力图 + 路径规划是新工作 |
| 8 | **不定价**（`resource.rs` 自己承认）；只有一次性 bid/match | REF2 | v1.4 自主定价、v1.2 多轮协商是新工作 |
| 9 | 个体学习缺失：记忆是哈希链 + LLM 调用，没有信用分配 | REF2 | v1.6 学习循环必须证明「学习真的改变行为」 |

### P2 —— 值得保留的工程习惯

* **带原因的具名拒绝**代替 `todo!()`：REF2 全树 `todo!()`/`unimplemented!()`/`unreachable!()`/`#[ignore]` 为 0，
  未实现处一律是「以名字拒绝 + 给原因」（7 个 RuntimeKind 中的 5 个、AppArmor/eBPF、TLS、4 种证明格式）。
* **`refusal_is_misconduct`**（REF2 `bus.rs:623`）：10 个拒绝码里只有 3 个升级为隔离，因为
  「凡是拒绝就隔离」等于「凡是竞争就误伤」。AU4A 收敛为 10 码中 **2 码**单次即恶意。
* **规范 JSON 帧**（REF2 `bus.rs:23-30`）：刻意不用 bincode/CBOR；4 字节大端长度 + JSON；1 MiB 上限；
  **从前缀拒绝，不先分配负载**。AU4A 的 `Envelope`/`encode_frame` 直接继承。
* **冻结线格式 + 规范参考实现 + 一致性用例**（REF1 `spec/` TEP-0..3 + 69 向量 × 3 语言 + 会重算期望的 runner）。
* **诚实标记**：REF1 的 `EvidenceGrade{verified,cpu-proto,unverified}` 被用作**结算闸门**，
  以及显式 `[RESULT NEEDED]` 边界。

## 三、人类中心假设：这次要反转的东西

两个参考项目共享一个模式：**系统无法决定时，返回带原因的拒绝，然后把决定权交给人类。**
拒绝纪律是资产；「人类兜底决策」是 AU4A 要替换的对象。

| 位置 | 出处 | AU4A 的反转 | 轨道 |
|---|---|---|---|
| `Approval::Operator`（运营方不能代替委员会） | REF2 `capability.rs:390-396` | 审批主体是 Agent 委员会 | v1.7 |
| 强制 `ReviewStage::ManualReview` | REF2 生命周期 | Agent 交叉验证 + 证据分级 | v1.0/v1.5 |
| 运营方信任库是第三方插件唯一通路 | REF2 插件加载 | 信任来自不可转让信誉 + 质押 | v1.4/v1.7 |
| `Authority::Operator` 唯一仲裁 | REF2 争议 | 多 Agent 签名裁决 + 申诉 | v1.7/v1.5 |
| 链上治理**全部** `onlyOwner` | REF2 Solidity | Agent 提案表决；人类否决**只阻断** | v1.7 |
| 自由文本人工豁免理由 | REF2 策略例外 | 结构化豁免 + 公开理由哈希 | v1.5 |
| `Scope::Admin` 锁住仲裁与插件调用 | REF2 HTTP 面 | Agent 侧安全 API（查询/举报/申诉） | v1.5 |
| daemon 只读、无面向 Agent 的协议 | REF1 `gsn-daemon` | 观察层只读，但通信层 **A2A 优先** | v1.0 |
| 桌面端是硬编码 DID 的演示 | REF1 `client/` | 不提供「人类操作台」，只提供只读面板 | v1.0 |
| 市场在进程内、信誉不持久化 | REF1 | 状态可移植（快照 + 2PC） | v1.3 |
| `NodeMode` 解析了但不执行 | REF1 | 权限模型要能真的回答「能/不能做什么」 | v1.0/v1.5 |

## 四、未决问题与证据等级上限（诚实声明）

1. **真实网络从未验证**：两个参考项目都没有跑过跨机 P2P 市场。→ AU4A v1.3 的跨节点传输在本机是
   双节点内存实现，证据等级 `cpu-proto`。
2. **真实链从未验证**：十个跨链名词 0 命中。→ AU4A v1.8 一律 `cpu-proto`，并显式列出「不支持清单」。
3. **真实并发从未验证**：REF1 无 daemon 集成测试。→ AU4A 的 10k 节点（v1.9）是确定性聚合模拟，
   文档里直接写明「不是真实分布式压测」。
4. **跨平台未验证**：本轮只在 Windows 本机实测。→ `docs/VERIFICATION.md` 的未验证项清单会列出 Linux/macOS。

## 五、对 AU4A 验收的启示

把上面这些失败模式**写成了本项目自己的约束**（对应 [PRINCIPLES.md](PRINCIPLES.md) 的十条）：

| 参考项目的失败模式 | AU4A 的对应约束 |
|---|---|
| 文档声称能力但代码不可达 | 原则 4「可证伪」+ 由版本清单生成的 DESIGN/DEV + 每版验收与证据 |
| 未验证的东西进入经济结算 | 原则 5「证据分级是结算闸门」，`unverified` 永不可结算 |
| 浮点账本无法证明守恒 | 原则 7「整数守恒账本」，任意时刻可断言 |
| 「凡拒绝即惩罚」误伤竞争 | 原则 8「类型化拒绝」，只有 2 个码单次即恶意 |
| 人类兜底决策无处不在 | 原则 2/3：观察面只有 GET（结构性测试），否决只能阻断（类型层面无提案能力） |
| 门面与实现分离 | 原则 10「诚实优先于漂亮」：`au4a-node` 调用的就是轨道的真实代码路径 |
| 多份实现漂移、版本号互相矛盾 | 原则 9「轨道隔离，只增不改」+ `docs/versions.json` 单一真相 |
