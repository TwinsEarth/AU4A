# 知识库：代码库理解与架构梳理

> 这是 AU4A 立项阶段的知识底座：**先把两个参考项目读懂，再决定哪些概念借用、哪些必须反转**。
> 完整调查记录（只读浏览 + 只读构建检查，未修改任何参考仓库）：
> [docs/recon/REF1-ARCHITECTURE.md](recon/REF1-ARCHITECTURE.md)（`agent-universe`）、
> [docs/recon/REF2-ARCHITECTURE.md](recon/REF2-ARCHITECTURE.md)（`NewAgentUniverseByDeepSeek`）。

参考项目（**只读引用，本仓库不包含其代码，也不修改它们**）：

* https://github.com/TwinsEarth/agent-universe
* https://github.com/TwinsEarth/NewAgentUniverseByDeepSeek

---

## 一、参考项目一：`agent-universe`（群体化 AGI 路线）

**形态**：被 vendored 进一个融合 monorepo 的**冻结子集**（约上游 25% 文件），而不是活跃主线。
版本号在五处互相矛盾（monorepo `VERSION`=1.2.3、`package.json`/README=2.3.4、`gsn-core/Cargo.toml`=0.2.35、
libp2p Identify 字符串 `/gsn/0.2.34`、client Tauri=2.3.5）——见 REF1 §1、§2。

| 能力 | 代码位置 | 实测结论 | 证据等级 |
|---|---|---|---|
| 市场 / 匹配 / 结算 | `agent-universe/gsn-core/src/marketplace/`（约 15k Rust 行） | **真实不变式**：守恒账本 `balance_sum = total_budget − total_slashed`、BFT-lite QA（n≥3f+1，模棱两可作废该轮）、质押+罚没、4 维**不可转让**信誉、TaskSpec 六字段准入闸门 | verified（代码级） |
| 市场的可达性 | `AgentMarket` 仅出现在自己的模块、`lib.rs`、测试与一个 example | **daemon 没有任何市场端点**（只有 `/health`、`/version`、`/peers`、`/agents`、`/tasks`、`POST /agents`）；市场/信誉/质押**不持久化**；SQLite 只存 19 个名片字段中的 5 个 | verified（这是最大的架构陷阱） |
| 身份 | `did:aip:<16hex>`（Rust）与 `did:au:<32hex>`（JS） | 存在但**碎片化**：无 resolver，keyring 只有 `MemoryKeyring` | partial |
| Agent 名片 / 技能 | `AgentCard`、`MarketAgentCard`、`AgentManifest` + `SkillManifest` | 三套重叠类型 | partial |
| P2P | 真 libp2p 0.54（TCP+Noise+Yamux+Kad+GossipSub+Identify）在 `net/peer.rs`；真 Kademlia 在 `net/dht.rs`；真 STUN + UDP 打洞 | **但** `net/libp2p_node.rs`（`GsnNode`）与 `net/gossip.rs`（`GossipSub`）是 HashMap 假实现，却从 `lib.rs:35` 作为公开 API 导出，真实现藏在 `lib.rs:83-91` | 分裂 |
| 插件总线 / 策略引擎 | — | **零命中**：仓库里没有 plugin，也没有 OPA/Rego | absent |
| 群体智能 | swarm 相关启发式 | 窗口比较启发式；`FaultTolerance` / `Evolution` 声明了但从不发出 | partial |
| 信誉经济 | Rust（u16 分 vs 4 维 f64）+ JS + Python | **同一套不变式的 4 份实现** | 分裂 |
| AUSec 弹性计算 | 最接近的是 first-fit-VRAM 的 `ComputeScheduler` | 没有 AUSec | absent |
| Agent 安全组织 | `SecurityEngine` 存在但从不被调用 | Sybil 无防护（CHANGELOG 自认） | absent |
| 线格式规范 | `spec/` TEP-0/1/2/3：规范 JSON（UTF-8 键序 + half-to-even 6 位小数）、CIDv1 dag-json、Trace 哈希链、PCE token；69 个向量 × 3 种可执行语言 + 会重算期望值的一致性 runner | **本项目最好的资产**，但 `gsn-core` 完全不消费它 | verified |

**已知桩**：`verifier/client.rs` 恒返回 `valid=true/0.95`；`mcp tools/call` 返回占位串；`erasure/` 不是 Reed-Solomon（哈希奇偶校验，无法用奇偶恢复）；`crdt` 只是 VersionVector；`storage/sqlite.rs` 是 HashMap；`chain/pocv.rs` 是哈希相等，不是可验证计算。

**测试实况**：251 Rust + 8 JS + 6 Python SDK + 146 interop + spec 31/42/26 通过；
**零 daemon/HTTP 集成测试、零 Solidity 测试**（4 个 `.sol` 从不编译，CI 只 grep `pragma`）。

**值得当实践借鉴的诚实产物**：`EvidenceGrade{verified, cpu-proto, unverified}` 被当作结算闸门；
显式 `[RESULT NEEDED]` 边界；每项能力一张证据表；interop 层逐行镜像 Rust 语义并自己标注 `[interop-extra]` 差异。

**两个真实坑（工程细节，值得记住）**：libp2p `idle_connection_timeout=0` 会杀掉连接；
Kademlia `Mode::Client` 会拒绝所有 inbound subflow。

---

## 二、参考项目二：`NewAgentUniverseByDeepSeek`（clean-room 重写，Rust）

**形态**：`VERSION`=**3.8.0**，CHANGELOG 是一条连续的 3.x 线（34 个 release 一直回溯到 1.0.1）；
19 个 Rust crate ≈ **89k LOC src + 11k LOC 测试**；依赖 DAG 以 **`nau-core`（无 I/O 的冻结叶子）** 为底。
注意：`docs/ARCHITECTURE.md`、`VERIFICATION.md`、`SELF-AUDIT.md`、`ATTRIBUTION.md`、`NOTICE`
仍写着 V1.2.3，README 正文是 V3.2.1——**散文会漂移，代码不会**；版本检查器只断言
`VERSION ↔ Cargo.toml ↔ SDKs ↔ contracts/VERSION`。

| 资产 | 位置 | 为什么重要 |
|---|---|---|
| 插件内核 | `nau-plugin` + `nau-plugins`（**11,039 LOC**）：21 能力 × 5 隔离层级的**全矩阵**（带叉积测试）、12 状态生命周期（只有一处赋值点）、7×15 运行时/边界词表、25 个 T0 系统插件 | 全项目工程价值最集中的地方 |
| PMB 消息总线 | `bus.rs`：**规范 JSON**（刻意不用 bincode/CBOR）、`Bus::send` 五道检查、4 字节大端长度 + JSON 帧、1 MiB 上限、**从前缀拒绝（不先分配）** | AU4A 的 `Envelope`/`encode_frame` 直接继承这套语义 |
| 拒绝分类 | `refusal_is_misconduct`（`bus.rs:623`）：**10 个拒绝码里只有 3 个**升级为隔离，因为「凡是拒绝就隔离」等于「凡是竞争就误伤」 | AU4A 把它收敛为 10 码中 2 码单次即恶意 |
| ABI | `ABI_MAJOR=3, ABI_MINOR=2` | 版本化接口纪律 |
| 构建实测 | `cargo +1.85.0 check --workspace --all-targets` → **exit 0，3m25s，0 warning**（target 目录放在树外） | 说明这套工程是真能构建的 |
| 清洁度 | `todo!()`/`unimplemented!()`/`unreachable!()`/`#[ignore]` **全为 0**；未实现处一律是**带原因的具名拒绝**（7 个 RuntimeKind 中的 5 个、AppArmor/eBPF、TLS、4 种证明格式） | 「拒绝是资产，人类才是要被替换的东西」 |

**缺失（正是 AU4A 要做的新工作）**：能力**图**（这里是扁平集合，没有边、委派、衰减、过期）；
迭代式协商（只有一次性 bid/match）；**自主定价**（`resource.rs` 明说它不定价）；
Agent 个体学习（记忆是哈希链 + LLM 调用，没有信用分配）；提案/纪元治理；
以及**十个跨链名词全部 0 命中**（`ERC-8004`、`x402`、`L402`、`Lightning`、`Taproot`、`RGB`、
`HTLC`、`USDC`、`ERC-4337`、`Paymaster`）——唯一可复用的是那个**按名字拒绝这十个东西**的插件。

---

## 三、人类中心假设：必须反转的清单

参考项目二的模式非常统一：**系统无法决定时，返回一个带原因的拒绝，然后把决定权交给人类。**
拒绝纪律是要保留的资产；人类这个「兜底决策者」是要替换的对象。

| 人类中心的位置 | 出处 | AU4A 的反转 | 承接轨道 |
|---|---|---|---|
| `Approval::Operator` vs `VendorTeam`/`CertificationCommittee`（运营方**不能**代替委员会） | `capability.rs:390-396` | 审批主体换成 Agent 委员会，人类连代签的资格都没有 | v1.7 |
| 强制 `ReviewStage::ManualReview` 人工阶段 | 插件生命周期 | 复核由 Agent 交叉验证 + 证据分级替代 | v1.0 / v1.5 |
| 运营方信任库是第三方插件加载的唯一通路 | 插件加载 | 信任来自不可转让信誉与质押，不来自运营方白名单 | v1.4 / v1.7 |
| `Authority::Operator` 是唯一仲裁者 | 争议处理 | 仲裁委员会（多 Agent 签名裁决 + 申诉） | v1.7 / v1.5 |
| 链上治理函数**全部** `onlyOwner` | Solidity | 治理映射为 Agent 提案/表决，人类只保留「否决只阻断」 | v1.7 |
| 自由文本的人工豁免理由 | 策略例外 | 结构化豁免 + 公开理由哈希 | v1.5 |
| `Scope::Admin` 同时把仲裁与 HTTP 插件调用锁给人类 | HTTP 面 | Agent 侧安全 API（权限边界可查询、可举报、可申诉） | v1.5 |
| daemon 只读、没有面向 Agent 的协议 | `gsn-daemon` | 观察层只读，但**通信层 A2A 优先**（PMB 是 Agent 的主通道） | v1.0 |

---

## 四、AU4A 的取舍映射（概念借用，代码不搬）

| 借用 | 来源 | 落到 AU4A |
|---|---|---|
| 冻结线格式 + 规范参考实现 + 一致性用例 | REF1 `spec/` | `au4a-core` 的 `canonicalize`/`canonical_hash` + 逐版本测试向量 |
| 证据分级作为结算闸门 | REF1 `EvidenceGrade` | `au4a-core::EvidenceGrade::settleable`，`unverified` 永不可结算 |
| 守恒不变式可断言 | REF1 marketplace 账本 | `Ledger::check_conservation`（整数微积分，无浮点） |
| BFT-lite 具名失败模式 | REF1 + REF2 | v1.7 表决（n≥3f+1、模棱两可作废、重复投票拒绝） |
| 不可转让信誉 | REF1 4 维信誉 | v1.6 学习模型（无转账 API + 越界钳制） |
| 类型化拒绝 + 恶意/竞争分离 | REF2 `refusal_is_misconduct` | `RefusalCode`（10 码，2 码单次即恶意，其余重复才升级） |
| 规范 JSON 帧 + 前缀拒绝 | REF2 `bus.rs` | `Envelope` + 4 字节大端长度 + 1 MiB 上限，分配前拒绝 |
| 带原因的具名拒绝（而不是 `todo!()`） | REF2 全树清洁度 | 每条轨道的「不支持清单」与证据等级 |
| 只增不改的公共接口 | REF2 ABI 版本化 | 冻结基元 + 10 条轨道零耦合并行 |

**不搬的东西**：不可达的 HashMap 假实现、浮点账本、同一不变式的四份实现、只写在文档里的能力、
以及所有「把决定权交给人类」的兜底路径。

---

## 五、术语表

| 术语 | 含义 |
|---|---|
| **DID** | 自证身份 `did:au4a:<32 字节 Ed25519 公钥 hex>`；DID 本身就是公钥，无解析器、无人类账户 |
| **PMB** | Plugin Message Bus：Agent 之间的消息总线，规范 JSON 帧，先签名后发送 |
| **Capability Graph** | 能力图：skill + 延迟分位 + 吞吐 + 负载 + 单价 + 可靠度 + 格式 + 约束，并支持路径规划 |
| **Evidence Grade** | 证据分级 `verified` / `cpu-proto` / `unverified`，作为结算闸门 |
| **Refusal Code** | 十种类型化拒绝，区分「竞争导致的拒绝」与「恶意导致的拒绝」 |
| **BFT-lite** | 轻量拜占庭容错表决：n ≥ 3f+1，模棱两可作废该轮 |
| **2PC migration** | 状态迁移两阶段提交：prepare → commit → confirm，失败回滚不留部分状态 |
| **fail-closed** | 链上与账本不一致时拒绝结算，而不是猜测 |
| **Scaling Law** | 网络结构的缩放定律：节点数 / 交互复杂度 / 算力预算 与群体智能水平的关系（v1.9 形式化） |
