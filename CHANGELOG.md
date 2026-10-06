# Changelog

## v2.6.0（2026-10-06）

**签名覆盖面收窄：只覆盖重点与高风险项。** 按产品决策，"报告来源证明"不再覆盖整份报告。

### 决策与口径

| 面板 | 是否在签名范围内 | 理由 |
|---|---|---|
| **结果**（`results`） | ✅ | 审计结论（守恒 / 准入 / 质押覆盖 / 注册表 / 队列 / 生命周期 / 时钟）——治理与信任的根 |
| **收益**（`yield`） | ✅ | 账本投影与每个 Agent 的可用/锁定余额——**钱** |
| **进度**（`progress`） | ❌ **刻意排除** | 事件流体量大、风险低；逐条事件签名会让成本随事件数增长，而"进度被篡改"不改变任何账或审计结论 |

签名载荷 = `network_id + now + [(route, fingerprint)] @ 结果/收益`。
新增 `ObserverReport::signed_routes()` 作为**口径单点**（面板与测试都从它枚举，避免两处漂移）。
`verify_provenance()` 的"逐投影重算指纹"也同步收窄到同一范围——校验范围与签名范围**必须一致**，
否则会出现"签名不覆盖、验签却拦下"的自相矛盾。

### 变更的影响面

- **进度面板的篡改不会导致验签失败**：这是显式取舍，不是漏洞。它仍然有自己的 `fingerprint`，
  需要比对的调用方可以自行比对；
- 进度面板载荷**仍随报告一起返回**（人类照常能看到），只是不在签名范围内。

### 测试（4 个，`observer_scope_v260.rs`）

- `signed_scope_covers_results_and_yield_only` —— 口径断言：`signed_routes() == ["results", "yield"]`；
- `tampering_a_signed_panel_is_detected` —— 改 `now`／改收益指纹／替换收益载荷／换签发者 **四类篡改全部被发现**；
- `progress_panel_is_intentionally_out_of_scope` —— 只改进度 → 验签**通过**（刻意）；再改收益 → **立刻失败**（证明范围不是"全都放行"）；
- `unsigned_report_is_not_sealed` —— 未签名/半签名/垃圾签名/非法 DID 四种错误分别断言。

**旧测试文件 `observer_provenance_v250.rs` 已删除**：它断言"改进度也要被发现"，与新口径矛盾；
新文件是它的**超集**（含全部四种未签名错误分支）。

### 证据

- `cargo test -p au4a-kernel` → exit 0（含新套件 4 passed）
- `cargo test --workspace --locked -j 2` → exit 0

### 仍未做（下一版）

**`au4a-node` 面板接入签名**——按你的决策需要：
密钥来源**三种全支持**（`--node-key <64hex>` / `--node-key-file <path>` / 环境变量 `AU4A_NODE_KEY`，
**默认用环境变量**，容器友好）；签名**两种暴露都支持**（默认嵌入各面板响应，另加独立路由 `/api/provenance`）。

---

## v2.5.0（2026-10-06）

**观察报告的来源证明（新增能力）。** 这是 P1-2 审查里唯一被单列一条的能力项：
修复前"人类看到的报告"**只自洽、无来源**。

### 问题

`ObserverReport::fingerprint()` 只是报告内容的哈希，它证明的是**"这份报告自洽"**，
不是**"这份报告来自某个真实的内核状态"**。任何人都能用 `serde_json::from_value` 造一份
字段完全自洽的报告（`writable: false`、任意 `payload`、自洽的 `fingerprint`），
指纹照样算得出来。而"人类只能看到真相"是本项目的立身之本——**真相必须可验证来源**。

### 新增

| 项 | 说明 |
|---|---|
| `ObserverReport::node_did: Option<String>` | 签发节点 DID（`None` = 未签名的本地渲染；旧格式反序列化默认 `None`，**向后兼容**） |
| `ObserverReport::sig: String` | 对签名载荷的 Ed25519 签名（hex）；空串 = 未签名 |
| `ObserverReport::signing_payload()` | 被签名载荷 = `network_id + now + 各投影的 (route, fingerprint)`。投影指纹已把载荷内容绑住，因此不必重复编码整份载荷，同时保证**可复算** |
| `ObserverReport::verify_provenance()` | 来源验证：未签名 → `NotSealed`（与"签名错"**明确区分**）；DID 非法 → `InvalidDid`；签名不符 → `InvalidSignature` |
| `Observer::report_signed(kernel, keys)` | 签发入口。渲染仍然只读（只取 `&Kernel`），密钥由调用方显式传入 —— **"观察层在类型上不存在写路径"这一性质不受影响** |

线上形状：新增两个字段且都带 `#[serde(default)]`，旧报告仍可解析；`Observer::report` 的行为
（`node_did = None`、`sig = ""`）与之前完全一致。

### 回归测试（2 个，独立集成测试文件）

- `signed_report_verifies_and_tampering_is_detected` —— 合法签名通过；**改时刻 / 改任一投影指纹 / 换签发者 / 改投影载荷**四种篡改全部被验签发现；
- `unsigned_report_is_not_sealed` —— 未签名 → `NotSealed`；只给 DID 不给签名 → 仍是 `NotSealed`；合法 DID + 垃圾签名 → `InvalidSignature`；非法 DID → `InvalidDid`。

### 证据

- `cargo test -p au4a-kernel` → 全绿（lib 套件 95 passed + 新集成套件 2 passed）
- `cargo test --workspace --locked -j 2` → **exit 0**

### 备注

- 本版是**新增能力**（不是修复），按此前约定单列；
- 仍未做：观察层 HTTP 面板（`au4a-node`）尚未调用 `report_signed`——面板要验证来源，
  需要节点持有密钥并在 `/api/*` 响应里带上 DID + 签名，这属于下一版（v2.6.0）的接口工作。

---

## v2.4.0（2026-10-06）

**内核审计（P1-2）落地批次：1 条 P0 + 15 条 P1 + 若干 P2。** 全部带回归测试，每批都过
`cargo test --workspace --locked`。

### 🔴 P0（远端可触发、单条消息、节点不自愈）

| 缺陷 | 修法 |
|---|---|
| **时钟毒化**：`pmb::admit` 只查"落后"不查"超前"，一条 `ts = u64::MAX` 的**合法**信封通过准入后，`Kernel::send` 会把本地逻辑时钟取 max 顶到天花板 → 此后所有正常信封都被判 `stale_epoch` → 节点通信永久瘫痪 | ① `admit` 增加**对称的超前界**；② `Kernel::send` 改为**有界推进**（`observe` 自带 +1，故上界取 `MAX_LAG - 1`），使"单条远端信封最多推进 MAX_LAG 步"成为**精确**不变式 |

### P1（15 条）

| 类别 | 内容 |
|---|---|
| **serde 绕过构造校验**（同族第 4、5 例） | `RegistrySnapshot` 反序列化校验一致性（DID 可解析、无重复、索引不指向局外人）；`Lifecycle` 反序列化**必须重放历史**并断言 `state == 折叠结果` |
| **策略阈值可注入** | `KernelConfig` 反序列化校验（结算上限有**硬上界**、准入下限/创世额度非负、能力数上限在 `[1, 4096]`、网络标识非空） |
| **证据可伪造** | `TrackManifest`/`VersionSpec`/`VersionEvidence` 校验（分级名必须是三个已知值、`tests_passed <= tests_total` 且总数非零、版本号形状、版本号唯一） |
| **投递账可伪造** | `RouteDecision` 校验自洽（`accepted` 与 `code` 互斥、通过必须有收件人、被拒必须有类型化码、`class` 必须等于 `classify_kind(kind)`） |
| **静默失败** | `autonomy` 意图执行失败不再被 `unwrap_or_default()` 吞掉（按既有分类记入本回合拒绝）；`lib.rs` 观察投影失败不再伪装成 `null`；重放指纹失败用显式哨兵 `<unavailable>` 而非空串（空串会让两次失败比较相等 = 假通过）；`migration` 新增三态 `intact_status` 区分"无法计算"与"内容不符" |
| **不变式未强制** | `permission` 在**构造期**强制"allowed ∪ denied == ALL"（此前只是文档承诺）；`Lifecycle` 指纹改为覆盖 `state + history`（此前"同历史不同状态"指纹相同） |
| **准入瑕疵** | `registry` **准入即拒绝**重复/空能力声明（此前只事后报告，一个 Agent 用重复字符串即可稀释能力图权重）；审计侧保留为**第二道闸门** |
| **可复算性** | `permission` 报告记录抽签时刻 `sorted_at` 并导出到 JSON（此前默认 `explain` 的结果第三方无法复算） |
| **内存型 DoS** | `pmb` 重放表按滞后视野**回收**（此前只增不减）；`audit` 乱序计数基线改 `last.max(event.at)`（此前一次倒退会被重复计数） |

### P2（本版）

- `canon` 补"像整数的浮点"拒绝测试（`1.0` / `1e10` / `-0.0`），防止有人"优化"成"小数部分为 0 就转整数"；
- `observer` 的「结果」面板**审计只算一次**（原来连续调三次，每次遍历全部 Agent/队列/拒绝）；
- `audit` 模块文档补齐第 8 条检查（生命周期台账）；
- `au4a-core` 的 `SERIES` 常量拆分：`V1_SERIES`（v1 事实）+ `V2_LINE`（当前线），不再用 `SERIES` 声称"当前系列"。

### 证据

- `cargo test -p au4a-core -p au4a-kernel` → 全绿（内核 lib 套件 **95 passed**；新增 4 个集成测试文件：`config_validation`(3) / `manifest_validation`(2) / `route_decision_validation`(2)）
- `cargo test --workspace --locked -j 2` → **exit 0**
- 每个修复都有回归测试，且断言"修复前会失败"的具体行为（例：P0 断言时钟 `2 -> 66` 而非 `67`；`Lifecycle` 断言 `Provisional` 经竞争性拒绝后仍是 `Provisional`）

### 未纳入本版

- **`observer` 报告来源签名**（节点 DID + 签名 + 验证路径）——属新增能力，按计划单列 **v2.5.0**；
- `CoreError → 短名` 的两份映射尚未合并（`kernel/errors.rs` 与 `safety/schema.rs`）。

### 运维注记

- 工作区构建**必须加 `-j 2`**：本机为每次闸门开独立 `CARGO_TARGET_DIR` 会累积数十 GB 构建缓存，
  并发编译会以 `rustc-LLVM ERROR: out of memory` / `STATUS_STACK_BUFFER_OVERRUN` 失败（本会话已发生两次，已清理 ~45 GB）。

---

## v2.3.0（2026-10-05）

**冻结基元四项 P1 修复 + 4 个回归测试。** 这是 P1-1（`au4a-core` 九个文件逐行审查）的落地批次。

### 修复

| # | 缺陷 | 文件 | 修法 |
|---|---|---|---|
| ① | **`MsgKind` 的 serde 绕过校验**：`#[serde(transparent)]` + `derive(Deserialize)` 让 `"UPPER!!"` 直接成为合法 kind。而 `kind` 是**路由键**（`classify_kind`、安全轨道 schema 映射），脏值会绕开路由假设 | `au4a-core/src/msg.rs` | `#[serde(try_from = "String", into = "String")]` + `TryFrom`（复用 `new`）→ 所有 serde 入口都过校验；线上形状仍是字符串。**这是 v2.2.0 修掉 `Did`/`Refusal` 后，在同一个文件里发现的第三例** |
| ② | **`slash()` 静默部分执行**：超出锁定余额时钳制并返回 `Ok(())`，调用方看不到"要求罚 999、实际只罚 20" | `au4a-core/src/ledger.rs` | 改为返回**实际销毁量** `CoreResult<Credits>`；钳制语义保留（刻意的），但真实数量不再被吞掉。10 个调用点（safety/economy/negotiate/council）源码兼容 |
| ③ | **`checked_sub` 可产出负值**，与类型注释「永远非负」矛盾 | `au4a-core/src/ledger.rs` | **不改旧语义**（全仓 30 处调用中有若干合法依赖负差值：规模轨道 `step_gain`、学习轨道 `revenue_lift`、度量 diff）；新增 `checked_sub_nonneg`（负结果返回 `NegativeAmount`）给金额路径，并把类型注释改成说真话的版本 |
| ④ | **守恒破坏被误报为 `Overflow`** | `au4a-core/src/error.rs`、`ledger.rs` | 新增 `CoreError::ConservationViolated`；`check_conservation` 改用它。**新增变体是接口变更**：4 处穷举 match 同步适配（core/error 的 Display、safety/schema 的拒绝码、kernel/errors 的 `code()`、kernel/autonomy 的 `classify_error`——后者归类为 `ResourceExhausted`，因为内部状态不一致不是对端行为，不能因此隔离对方） |

### 回归测试（4 个新增）

- `msg::tests::unvalidated_kind_is_refused_by_serde` —— 4 种非法 kind 经 serde 全被拒；合法 kind 往返且线上形状仍是字符串；**信封里的 kind 同样受校验**；
- `ledger::tests::slash_reports_the_actual_amount_destroyed` —— `slash(999)` 在锁定 20 时返回 `Credits(20)`；罚光后再罚返回 0；
- `ledger::tests::checked_sub_is_signed_for_deltas_but_nonneg_for_money` —— 同时钉住两种减法的语义；
- `ledger::tests::broken_conservation_reports_its_own_error_not_overflow` —— 制造真实守恒破坏，断言专属错误码且不再是 `Overflow`。

### 证据

- `cargo test -p au4a-core` → **45 passed / 0 failed**（v2.2.0 时 41，本版 +4）
- `cargo test --workspace --locked` → **exit 0**（`slash` 签名变更 + 枚举新增变体，零破坏）
- `cargo fmt --all --check` → exit 0

### 系统性结论（P1-1 九文件审查的产出）

**「`derive(Deserialize)` 绕过构造函数」是这一层的模式性缺陷**，已在三个类型上确认：
`Did`（v2.2.0 修）、`Refusal`（v2.2.0 修）、`MsgKind`（本版修）。
凡是 `new()` 里做校验的新类型，都必须走 `try_from` —— 否则类型成立、校验不成立。

---

## v2.2.0（2026-10-05）

**冻结基元的两处「serde 绕过构造校验」修复（P1×2）+ 4 个回归测试。**
线上 JSON 形状**完全不变**，因此这不是协议变更，而是把类型不变式重新钉回反序列化路径。

### 缺陷（同一类，出现在两个类型上）

| 类型 | 问题 | 后果 |
|---|---|---|
| `Did` | 只是 `derive(Deserialize)` 的新类型，反序列化**不经过 `parse()`** | 信封体、注册请求、状态导入这些**全部走 serde 的入口**都能造出 `Did("garbage")`：类型成立、校验不成立。它不能伪造签名（`public_key()` 会失败），但会让非法身份先被写进状态、更晚才炸，并污染任何以 `Did` 为键的去重/计费 |
| `Refusal` | 字段全 `pub` 且 derive `Deserialize`，`retryable` 是**自由字段**而非由 `code` 派生 | 可构造自相矛盾记录（`code: malformed` + `retryable: true`）。`retryable` 正是给 Agent 自主重试策略看的：一旦被写成 `true`，策略会对"不可重试的恶意帧"重试——恰好是这套设计要避免的竞争/恶意误伤 |

### 修复

- `Did`：`#[serde(try_from = "String", into = "String")]` + `impl TryFrom<String> for Did`（复用 `parse`）→ **所有 serde 入口都过校验**；
- `Refusal`：`#[serde(try_from = "RawRefusal", into = "RawRefusal")]`，反序列化时用 `Refusal::new` 重算 `retryable` 并与输入比对，**矛盾记录返回 `CoreError::Encoding`**；新增 `RawRefusal` 仅表示线上形态，字段一一对应，JSON 形状不变。

### 回归测试（4 个，正是点名的那四个）

- `did::tests::unvalidated_did_is_refused_by_serde` —— 4 种非法 DID 经 serde 全部被拒；合法 DID 往返正常；
- `did::tests::did_parse_and_serde_agree` —— `parse()` 与 serde 路径对同一字符串**同判**（防两条入口分叉）；
- `refusal::tests::contradictory_refusal_is_refused_by_serde` —— `malformed + retryable:true` 被拒且原因是 `CoreError::Encoding`；合法记录往返**逐字节相同**（证明线上形状没变）；
- `refusal::tests::retryable_is_derived_from_code` —— 对全部 10 个码断言 `retryable == code.retryable()`，且**伪造 retryable 一律被拒**。

### 证据

- `cargo test -p au4a-core` → **41 passed / 0 failed**（原 37 + 4）
- `cargo test --workspace --locked` → **exit 0**（`Did`/`Refusal` 被全仓使用，serde 改动不影响任何 crate）

### 未改变

- 协议与线上 JSON 形状不变；`Refusal` 字段仍为 `pub`（仓内直接字面量构造不受影响，本次修的是**网络输入**路径）。

---

## v2.1.0（2026-10-05）

**冻结基元的三个安全/正确性修复（P0×1 + P1×2）+ 回归测试。** 按项目自身规则，
改动冻结基元必须走新的中版本，因此这些修复不塞进 v2.0.x 补丁，而是升到 v2.1.0。

### 修复

| 级别 | 文件 | 问题 | 修法 |
|---|---|---|---|
| **P0** | `crates/au4a-core/src/evidence.rs` | `EvidenceGrade` 派生 `PartialOrd, Ord`，按**声明顺序**得到 `Verified < CpuProto < Unverified`，与"值越大越可信"完全相反——任何 `max()`/排序都会选到**最不可信**的那一级 | 删除派生，改为显式 `rank()`（Unverified=0 < CpuProto=1 < Verified=2）+ 手写 `Ord`/`PartialOrd` 以 `rank()` 为准，使 `Ord` 与结算闸门不再分叉 |
| **P1** | `crates/au4a-core/src/evidence.rs` | `settleable()` 只比大小不校验符号；`Credits(pub i64)` 的元组构造可绕过 `Credits::new` 的非负校验，负数在下溢包装后落进 `amount <= threshold` 的"小额"分支被放行 | 闸门首行拒绝负额：任何等级下 `amount < 0` 一律不可结算 |
| **P1** | `crates/au4a-core/src/prims.rs` | `LogicalClock::tick()` 用 `self.t += 1`（`u64::MAX` 时 debug panic / release 回绕到 0）；`observe(remote)` 的 `max(remote) + 1` 在 `remote == u64::MAX` 时溢出，而 `remote` 来自**对端信封的 ts**（外部可控） | 两处改 `saturating_add`：到顶停在 `u64::MAX`，单调不减性质（确定性重放的前提）得以保持 |

### 回归测试（共 3 个新用例）

- `evidence::tests::ord_direction_matches_trust_not_declaration_order` —— 断言 `Verified > CpuProto > Unverified`，并用 `Iterator::max()` 断言**选出的是最可信级别**（这正是修复前会错的场景）；
- `evidence::tests::negative_amount_is_never_settleable` —— `Credits(-1)`、`Credits(i64::MIN)` 在三个等级下全部拒绝；同时断言 0 与正额行为未变（防过度收紧）;
- `prims::tests::clock_saturates_at_u64_max_instead_of_wrapping` —— `observe(u64::MAX)` 饱和、`tick()` 到顶停住，并以 [0, 5, u64::MAX, 3, u64::MAX] 序列验证 `observe` 单调不减。

### 证据

- `cargo test -p au4a-core` → **37 passed / 0 failed**（原 34 + 新增 3）
- `cargo test --workspace --locked` → **exit 0**（96 个测试套件；`Ord` 派生移除未破坏任何 crate）

### 未改变（明确边界）

- `CoreError` 的语义重载（`InsufficientFunds` 复用为"证据闸门拒绝"）**未修**：它需要内核侧新增错误变体，属于接口变更，留给下一个中版本；
- 基元错误枚举被内核语义污染（`UnknownAgent`/`DuplicateAgent`/`InsufficientStake`）同样**未修**，理由同上。

---

## v2.0.2（2026-10-05）

**多平台交付级补齐 + 版本线缺陷修复。** 基线仍是 Rust 重写线（12 crate / 69,092 行），
本版不改内核语义，只把「可部署、可回滚、可验收」的缺口补齐，并修掉一个已发布的真实性缺陷。

### 新增
- `deploy/au4a-node.service` —— Linux systemd 单元。含 `ExecStartPre=au4a-node verify`（**自检不过不上线**）、
  `Restart=on-failure`、`NoNewPrivileges` / `ProtectSystem=strict` / `ReadOnlyPaths=/ /usr /etc` /
  `MemoryDenyWriteExecute` 等加固，与容器版的 `read_only + cap_drop ALL` 等价。
- `deploy/windows-service.md` —— Windows 10 服务化两种方案（内置 `sc.exe` 与 NSSM），含启动前自检包装脚本、
  日志轮转、防火墙、回滚与验收命令。
- `deploy/docker-compose.yml` + `.env.example` —— 单机 Docker 一键部署（只读根文件系统、丢全部 capability、
  健康检查直接用 `au4a-node verify`）。
- `docs/DEPLOY.md` —— 部署与回滚手册（含 7 条部署后验证、三层回滚、7 类常见报错→原因→修复）。
- `docs/ACCEPTANCE.md` —— 验收清单 A–F 六组，含**诚实清单**（明确列出未通过项）。

### 修复
- **已发布 tag 的真实性缺陷（P0）**：`v1.0.10` / `v1.1.10` / `v1.2.10` / `v1.3.10` / `v1.4.10` / `v1.5.10` / `v1.6.10`
  七棵版本树里混入了**当时尚不存在**的平台层 `crates/au4a-node` 与后期交付物（平台层到 v1.9.9 才逐版本快照，
  更早版本在重建时回落到工作树）。已**重新物化并强制移动**这 7 个 tag：移除当时不存在的 23 个路径，
  并同步从 workspace `members` 摘除 `crates/au4a-node`——**移除而非伪造**。主分支历史未改写。
- CI `version line` 随之恢复为对 **全部 10 个中版本收尾 tag**（v1.0.10 … v1.9.9）跑
  `cargo test --workspace --locked`，不再需要按 tag 收敛。
- 发布流水线两处假绿/环境缺陷：`|| echo` 吞掉真实发布失败、打包器写死 Windows 路径。
- 仓库同步脚本改为以远端实际 head 为父提交（此前用本地状态导致 `422 not a fast forward`）。

### 变更
- `crates/au4a-node` 全量实现（注册 / PMB / 场景编排 / 只读观察面板 / CLI）。
- `clippy` 作业在 CI 中显式标注为 **advisory**（照常真跑真报，暂不判红），清完即恢复硬门禁。

### 证据与验收
- `cargo test --workspace --locked` → **1238 passed / 0 failed**（96 个测试套件）
- `au4a-node verify` → **exit 0**（10 条轨道自检全 `[ok]`，213/213）
- 部署实测 10/10 中版本：构建成功、端到端成功、6 条只读路由全 200、写方法全 405
- 发布物：99 个 tag（v1.0.1 → v1.9.9）+ 10 个中版本 Release + npm `@twinsearth/au4a`（10 中版本 + 大版本线 `1.0.0`）+ GHCR 镜像

### 已知未完成（不粉饰）
- `clippy` 硬门禁未达成（advisory）；真实网络 / 真实链 / 真实并发未验证（相关能力证据等级 `cpu-proto`）。

---

## v1.9.9（2026-10-04）

10 个中版本 × 99 个小版本系列收尾：Autonomy / Capability Graph / Negotiation / Portable State /
Economic Autonomy / Safety API / Individual Learning / Committee Governance / Cross-Chain Settlement /
Network Scaling。`docs/DESIGN.md`（1197 行）与 `docs/DEV.md`（257 行）逐项覆盖 99 个小版本。
