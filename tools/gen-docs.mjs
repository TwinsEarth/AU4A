// gen-docs.mjs — 从 docs/versions.json + docs/tracks/*.json 生成 docs/DESIGN.md 与 docs/DEV.md。
//
// 它是**完整性闸门**而不是拼字符串工具：默认严格模式，只要不是「10 个中版本 + 99 个小版本、
// 全部版本号与清单逐一对应」，它就拒绝写文件并以非 0 退出。--allow-missing 只用于早期看结构。
//
//   node tools/gen-docs.mjs                  # 严格：10 中版本 + 99 小版本，缺一不可
//   node tools/gen-docs.mjs --allow-missing  # 草稿：缺失处标 ⏳ 未交付
//   node tools/gen-docs.mjs --validate-only  # 只校验

import fs from 'node:fs';
import path from 'node:path';

const ROOT = 'E:\\DS\\AU4A';
const argv = process.argv.slice(2);
const has = (n) => argv.includes(n);
const allowMissing = has('--allow-missing');
const validateOnly = has('--validate-only');

const manifest = JSON.parse(fs.readFileSync(path.join(ROOT, 'docs', 'versions.json'), 'utf8'));
const versions = manifest.versions.slice().sort((a, b) => a.seq - b.seq);
const tracks = manifest.tracks;

const STATUS_CN = { done: '✅ 已交付', partial: '🟡 部分交付', pending: '⏳ 未交付' };
const GRADE_CN = { verified: 'verified（本机实测）', 'cpu-proto': 'cpu-proto（语义原型）', unverified: 'unverified（未验证）' };

const problems = [];
const trackData = new Map();

for (const t of tracks) {
  const file = path.join(ROOT, 'docs', 'tracks', `${t.track}.json`);
  if (!fs.existsSync(file)) {
    problems.push(`缺少 docs/tracks/${t.track}.json（轨道 ${t.track} ${t.title}）`);
    continue;
  }
  let json;
  try {
    json = JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch (e) {
    problems.push(`docs/tracks/${t.track}.json 解析失败：${e.message}`);
    continue;
  }
  const expected = versions.filter((v) => v.track === t.track).map((v) => v.version);
  const got = (json.versions ?? []).map((v) => v.version);
  if (got.length !== expected.length) problems.push(`轨道 ${t.track}：期望 ${expected.length} 个版本，实际 ${got.length}`);
  for (let i = 0; i < Math.max(expected.length, got.length); i++) {
    if (expected[i] !== got[i]) problems.push(`轨道 ${t.track}：第 ${i + 1} 个版本应为 ${expected[i]}，实际 ${got[i] ?? '缺失'}`);
  }
  for (const v of json.versions ?? []) {
    const where = `轨道 ${t.track} ${v.version}`;
    if (!v.goal) problems.push(`${where}：缺少 goal`);
    if (!Array.isArray(v.deliverables) || !v.deliverables.length) problems.push(`${where}：缺少 deliverables`);
    if (!Array.isArray(v.acceptance) || !v.acceptance.length) problems.push(`${where}：缺少 acceptance`);
    if (!STATUS_CN[v.status]) problems.push(`${where}：status 非法（${v.status}）`);
    const e = v.evidence ?? {};
    if (!GRADE_CN[e.grade]) problems.push(`${where}：evidence.grade 非法（${e.grade}）`);
    if (typeof e.tests_passed !== 'number' || typeof e.tests_total !== 'number') problems.push(`${where}：evidence 测试数缺失`);
    else if (e.tests_passed > e.tests_total) problems.push(`${where}：tests_passed > tests_total`);
    if (!e.test_command) problems.push(`${where}：缺少 evidence.test_command`);
  }
  trackData.set(t.track, json);
}

const totalVersions = versions.length;
if (totalVersions !== 99) problems.push(`版本总数 ${totalVersions} != 99`);
if (tracks.length !== 10) problems.push(`中版本数 ${tracks.length} != 10`);
if (versions[0]?.version !== 'v1.0.1') problems.push(`首版本 ${versions[0]?.version} != v1.0.1`);
if (versions[versions.length - 1]?.version !== 'v1.9.9') problems.push(`末版本 ${versions[versions.length - 1]?.version} != v1.9.9`);
const finals = versions.filter((v) => v.medium_final);
if (finals.length !== 10) problems.push(`中版本收尾点 ${finals.length} != 10`);

const severe = problems.filter((p) => !allowMissing || !p.startsWith('缺少 docs/tracks/'));
console.log(`版本清单：${tracks.length} 中版本 / ${totalVersions} 小版本 / ${finals.length} 个部署发布点`);
for (const t of tracks) {
  const d = trackData.get(t.track);
  const done = d ? d.versions.filter((v) => v.status === 'done').length : 0;
  const tests = d ? d.versions.reduce((s, v) => s + (v.evidence?.tests_passed ?? 0), 0) : 0;
  console.log(`  轨道 ${t.track} ${t.crate.padEnd(16)} ${String(done).padStart(2)}/${String(t.count).padStart(2)} done  断言 ${String(tests).padStart(4)}`);
}
if (problems.length) {
  console.log(`\n问题 ${problems.length} 条${allowMissing ? '（草稿模式，仅提示）' : ''}：`);
  for (const p of problems.slice(0, 30)) console.log(`  - ${p}`);
  if (problems.length > 30) console.log(`  … 其余 ${problems.length - 30} 条省略`);
}
if (severe.length && !allowMissing) {
  console.error(`\n拒绝生成：完整性闸门未通过（${severe.length} 条硬问题）。修好后重跑，或用 --allow-missing 出草稿。`);
  process.exit(1);
}
if (validateOnly) process.exit(problems.length && !allowMissing ? 1 : 0);

// ---------------- 生成 ----------------
const principles = fs.readFileSync(path.join(ROOT, 'docs', 'PRINCIPLES.md'), 'utf8').trim();
const observer = fs.existsSync(path.join(ROOT, 'docs', 'OBSERVER.md'))
  ? fs.readFileSync(path.join(ROOT, 'docs', 'OBSERVER.md'), 'utf8').trim()
  : '（人类观察层设计见 `docs/OBSERVER.md`，由平台层交付。）';

const info = (v) => {
  const d = trackData.get(v.track);
  const e = d?.versions?.find((x) => x.version === v.version);
  return {
    title: e?.title ?? v.title,
    goal: e?.goal ?? '⏳ 未交付',
    deliverables: e?.deliverables ?? [],
    interfaces: e?.interfaces ?? [],
    acceptance: e?.acceptance ?? [],
    evidence: e?.evidence ?? { test_command: '—', tests_passed: 0, tests_total: 0, grade: 'unverified', notes: '' },
    status: e?.status ?? 'pending',
  };
};

const list = (arr, dash = '—') => (arr.length ? arr.map((x) => `\`${x}\``).join('、') : dash);

const REQ = [
  ['自主身份与可验证信誉', '1.0', 'v1.0.1–v1.0.2 自证 DID + 自主注册；v1.0.4 委员会与信誉'],
  ['可发现性与可组合性', '1.1', 'v1.1.1–v1.1.7 能力图声明/广播/查询/路径规划；v1.1.6 流水线组合'],
  ['资源弹性与状态可移植', '1.3', 'v1.3.1–v1.3.10 快照、跨节点传输、2PC 恢复、灾难恢复'],
  ['经济自主权', '1.4', 'v1.4.1–v1.4.10 余额、定价、质押、结算路由、兑换决策'],
  ['安全边界与可申诉性', '1.5', 'v1.5.1–v1.5.10 权限边界、举报、申诉、处罚记录、审计'],
  ['学习与进化', '1.6', 'v1.6.1–v1.6.10 经验库、反馈、行为调整、效果评估'],
  ['通信与协商', '1.2', 'v1.2.1–v1.2.10 协商消息、状态机、合约、违约仲裁'],
];

function designDoc() {
  const L = [];
  L.push('# AU4A 设计文档（Agent Universe For Agent）');
  L.push('');
  L.push('> **一切为智能体服务。人类用户兼任观察者，只展示进度、结果与收益。**');
  L.push(`> 版本：${manifest.series} ｜ 10 个中版本 + 99 个小版本 ｜ 开发模型：10 条轨道并行，轨道内小版本严格串行。`);
  if (allowMissing && problems.length) L.push('> ⚠️ 草稿：尚有轨道未交付，缺失处标 ⏳ 未交付。');
  L.push('');
  L.push('## 版本编号说明');
  L.push('');
  L.push('| 项 | 数量 | 说明 |');
  L.push('|---|---|---|');
  L.push('| 中版本 | 10 | `v1.0` … `v1.9`，每个中版本一条轨道、一个 crate |');
  L.push('| 小版本 | 99 | `v1.0.1` … `v1.9.9`；每个中版本 9–10 个，串行交付 |');
  L.push('| 部署与 Release 点 | 10 | 每个中版本的最后一个小版本（' + finals.map((v) => `\`${v.version}\``).join('、') + '） |');
  L.push('');
  L.push('小版本编号分布：`v1.0.x`–`v1.8.x` 各 10 个（`.1`–`.10`），`v1.9.x` 9 个（`.1`–`.9`），合计 99。');
  L.push('需求「每个中版本串行开发 9 个小版本」在本项目中体现为：**轨道内严格串行、一次只推进一个小版本**；');
  L.push('其中 9 条轨道交付 10 个小版本、1 条轨道交付 9 个，以满足「10 中版本 + 99 小版本」的总量口径。');
  L.push('');
  L.push('---');
  L.push('');
  L.push('## 一、核心理念：从「为人类服务」到「一切为智能体服务」');
  L.push('');
  L.push('v3.x 及之前本质是「人类使用 Agent 完成人类目标」——Agent 是工具，人类是主体。AU4A 把关系反转：');
  L.push('Agent 是网络的第一公民，人类退居观察者、资源提供者与收益接收者。');
  L.push('');
  L.push('| 维度 | v3.x（人类中心） | AU4A v1.x（Agent 中心） |');
  L.push('|---|---|---|');
  L.push('| 主体 | 人类用户 | Agent |');
  L.push('| 目标来源 | 人类设定任务 | Agent 自主设定目标 |');
  L.push('| 协作方式 | 人类调度 Agent | Agent 自主发现 / 匹配 / 协商 |');
  L.push('| 经济流向 | 人类支付、人类收益 | Agent 赚取与消耗积分，人类只收收益 |');
  L.push('| 人类角色 | 操作者、决策者 | **观察者、资源提供者、收益接收者** |');
  L.push('| 进化路径 | 人类优化 Agent | Agent 自我迭代与进化 |');
  L.push('');
  L.push('人类只做三件事：**看进度、看结果、看收益**。不做决策；仅在极端情况下行使否决权，且否决只能阻断（v1.7）。');
  L.push('');
  L.push('## 二、身为智能体的代表：Agent 的七项需求');
  L.push('');
  L.push('| # | 需求 | 承接轨道 | 承接小版本 |');
  L.push('|---|---|---|---|');
  REQ.forEach(([name, track, detail], i) => L.push(`| ${i + 1} | ${name} | ${track} | ${detail} |`));
  L.push('');
  L.push('## 三、系统架构');
  L.push('');
  L.push('```');
  L.push('┌──────────────────────────────────────────────────────────────┐');
  L.push('│ 人类观察层（只读）  进度 · 结果 · 收益   （au4a-node observer）│');
  L.push('├──────────────────────────────────────────────────────────────┤');
  L.push('│ Agent 自治层 身份 · 能力图 · 协商 · 经济 · 安全 · 学习         │');
  L.push('│   au4a-capgraph  au4a-negotiate  au4a-economy  au4a-safety    │');
  L.push('│   au4a-learning  au4a-council                                │');
  L.push('├──────────────────────────────────────────────────────────────┤');
  L.push('│ 宿主内核 au4a-kernel：自主注册 · PMB 投递 · 策略与拒绝 · 观察投影│');
  L.push('├──────────────────────────────────────────────────────────────┤');
  L.push('│ 冻结基元 au4a-core：自证 DID · 规范 JSON · 整数守恒账本         │');
  L.push('│   证据分级 · 类型化拒绝 · PMB 信封 · 逻辑时钟                 │');
  L.push('├──────────────────────────────────────────────────────────────┤');
  L.push('│ 状态与结算 au4a-state（可移植状态） · au4a-chain（跨链结算）   │');
  L.push('│ 扩展度量 au4a-scale（网络缩放定律）                            │');
  L.push('└──────────────────────────────────────────────────────────────┘');
  L.push('```');
  L.push('');
  L.push('与参考项目的关键差异：');
  L.push('');
  L.push('1. **人类从操作者变成观察者**：内核不提供任何「人类批准」路径，观察面只挂 GET 路由。');
  L.push('2. **协商是协议而不是流程**：v3.x 的「发布-匹配-执行」单向流程被 v1.2 的多轮协商 + 双方签名合约取代。');
  L.push('3. **账本是整数且守恒可断言**：参考项目的浮点账本无法证明守恒，AU4A 把守恒写成可运行断言。');
  L.push('4. **拒绝被分类**：恶意（2 码）与竞争（8 码）分开处置，避免「凡拒绝即隔离」误伤。');
  L.push('5. **能力图是图**：参考项目只有扁平能力集合，v1.1 引入延迟/负载/价格/格式/约束与路径规划。');
  L.push('');
  L.push('冻结基元 `au4a-core` 接口摘要：`Did`/`AgentKeys`（自证身份）、`canonicalize`/`canonical_hash`（规范 JSON，禁浮点）、');
  L.push('`Credits`/`Ledger`/`LedgerView`（整数守恒账本）、`EvidenceGrade`（结算闸门）、`RefusalCode`/`Refusal`/`escalate`（类型化拒绝）、');
  L.push('`Envelope`/`encode_frame`/`decode_frame`（4 字节大端长度前缀 + 规范 JSON，1 MiB 上限）、`LogicalClock`、`SelfCheck`。');
  L.push('');
  L.push('## 四、10 个中版本路线图');
  L.push('');
  for (const t of tracks) {
    const vs = versions.filter((v) => v.track === t.track);
    const d = trackData.get(t.track);
    const done = d ? d.versions.filter((v) => v.status === 'done').length : 0;
    L.push(`### ${t.track.replace(/^1\./, 'v1.')} ${t.title}`);
    L.push('');
    L.push(`- **crate**：\`${t.crate}\`　**小版本**：${t.count} 个（\`${t.range}\`）　**交付**：${done}/${t.count}`);
    L.push(`- **部署发布点**：\`${vs[vs.length - 1].version}\``);
    L.push(`- **小版本清单**：${vs.map((v) => `\`${v.version}\` ${info(v).title}`).join('；')}`);
    L.push('');
  }
  L.push('## 五、99 个小版本逐项');
  L.push('');
  for (const t of tracks) {
    L.push(`### ${t.track.replace(/^1\./, 'v1.')} ${t.title}（\`${t.crate}\`）`);
    L.push('');
    for (const v of versions.filter((x) => x.track === t.track)) {
      const i = info(v);
      L.push(`#### ${v.version} ${i.title}`);
      L.push('');
      L.push(`- **目标**：${i.goal}`);
      L.push(`- **交付物**：${list(i.deliverables)}`);
      L.push(`- **接口**：${list(i.interfaces)}`);
      L.push(`- **验收**：${i.acceptance.length ? i.acceptance.map((a) => a).join('；') : '—'}`);
      L.push(`- **证据**：\`${i.evidence.test_command}\` → ${i.evidence.tests_passed}/${i.evidence.tests_total}，等级 ${GRADE_CN[i.evidence.grade] ?? i.evidence.grade}${i.evidence.notes ? `（${i.evidence.notes}）` : ''}`);
      L.push(`- **状态**：${STATUS_CN[i.status] ?? i.status}`);
      L.push('');
    }
  }
  L.push('## 六、人类观察层');
  L.push('');
  L.push('| 面板 | 内容 | 数据来源 |');
  L.push('|---|---|---|');
  L.push('| 进度 | 活跃 Agent、协作中的任务、里程碑事件 | `Kernel::observe().progress` |');
  L.push('| 结果 | 任务完成情况、产出物、质量评估 | 各轨道 `results_json()` 与 `SelfCheck` |');
  L.push('| 收益 | 积分余额、变动记录、资源贡献对应关系 | `LedgerView`（发行 / 罚没 / 各账户可用与锁定） |');
  L.push('');
  L.push('只读性由结构强制：观察面只注册 GET 路由，非 GET 一律 405，且不存在任何写路由（有枚举路由表的结构性测试）。');
  L.push('');
  L.push('## 七、风险与缓解');
  L.push('');
  L.push('| # | 风险 | 缓解 |');
  L.push('|---|---|---|');
  L.push('| 1 | Agent 自治导致不可预测的经济行为 | 经济参数边界（证据闸门、结算上限）+ 委员会监督（v1.7）+ 人类否决只阻断 |');
  L.push('| 2 | 状态迁移过程中的不一致 | 两阶段提交（prepare/commit/confirm）+ 失败回滚不留部分状态（v1.3） |');
  L.push('| 3 | 个体学习的隐私泄露 | 经验库本地化 + 对外公开视图脱敏（v1.6） |');
  L.push('| 4 | 跨链资产与验证数据风险 | fail-closed：账本为真相、不一致则拒绝；适配器具名拒绝清单（v1.8） |');
  L.push('| 5 | 文档与代码漂移 | 文档由版本清单 + 轨道元数据生成；证据分级 + 结构性测试（本项目十原则第 4、10 条） |');
  L.push('| 6 | 并行开发互相破坏 | 轨道零耦合、公共接口只增不改、独立 target 目录（DEV.md 第四节） |');
  L.push('');
  L.push('## 八、验收标准');
  L.push('');
  L.push('| 维度 | 标准 |');
  L.push('|---|---|');
  L.push('| 功能 | 99 个小版本的验收项全部有对应测试；`cargo test --workspace` 全绿 |');
  L.push('| 兼容 | 参考项目语义对齐处（账本守恒、BFT-lite、拒绝分类）有对照测试 |');
  L.push('| 性能 | 10k 节点缩放模拟秒级完成（v1.9）；能力图查询走索引（v1.1.5/1.1.8） |');
  L.push('| 安全 | 只读观察面无写路由；篡改签名/快照/合约必被拒；罚没不超过锁定余额 |');
  L.push('| 文档 | 本文档与 `docs/DEV.md` 覆盖 10 中版本 + 99 小版本，且由生成器强制完整性 |');
  L.push('| 平台 | Windows 本机实测 10 个中版本收尾点（见 `docs/VERIFICATION.md`） |');
  L.push('');
  L.push('## 附：设计原则');
  L.push('');
  L.push(principles);
  L.push('');
  L.push('## 附：人类观察层实现说明');
  L.push('');
  L.push(observer);
  L.push('');
  L.push('---');
  L.push('');
  L.push('> 本文档由 `tools/gen-docs.mjs` 从 `docs/versions.json` 与 `docs/tracks/*.json` 生成；请勿手改。');
  return L.join('\n');
}

function devDoc() {
  const L = [];
  L.push('# AU4A 开发文档（Agent Universe For Agent）');
  L.push('');
  L.push(`> 版本：${manifest.series} ｜ 10 个中版本 + 99 个小版本 ｜ 本机实测 10 个中版本收尾点`);
  L.push('> 开发模型：**10 条轨道并行开发，轨道内 9–10 个小版本严格串行**。');
  if (allowMissing && problems.length) L.push('> ⚠️ 草稿：尚有轨道未交付，缺失处标 ⏳ 未交付。');
  L.push('');
  L.push('## 一、开发总纲');
  L.push('');
  L.push('1. **Agent 优先**：每个功能的第一个测试用例是「Agent 能否自主使用它」。');
  L.push('2. **向后兼容**：参考项目的账务语义（守恒、质押、罚没）与拒绝分类被继承并写成测试。');
  L.push('3. **轨道隔离**：10 条轨道零耦合，只共享冻结基元 `au4a-core` 与宿主内核 `au4a-kernel`；公共接口只增不改。');
  L.push('4. **可证伪**：每个小版本必须有可验证测试与证据等级；文档能力代码不可达即为缺陷。');
  L.push('');
  L.push('## 二、环境与依赖');
  L.push('');
  L.push('| 组件 | 版本 | 用途 |');
  L.push('|---|---|---|');
  L.push('| Rust | 1.88+（本机 1.98.1） | 全部 crate |');
  L.push('| cargo | 随 Rust | 构建与测试 |');
  L.push('| Node.js | ≥ 18（仅工具链） | 文档生成、版本物化、发布脚本；**不进入运行时** |');
  L.push('');
  L.push('外部依赖仅 6 个：`serde`、`serde_json`、`sha2`、`ed25519-dalek`、`rand`、`hex`。轨道新增依赖必须经 Lead 批准。');
  L.push('');
  L.push('## 三、代码组织');
  L.push('');
  L.push('| 轨道 | crate | 职责 | 版本区间 |');
  L.push('|---|---|---|---|');
  L.push('| — | `au4a-core` | 冻结基元：自证 DID、规范 JSON、整数守恒账本、证据分级、类型化拒绝、PMB 信封、逻辑时钟 | 全系列 |');
  for (const t of tracks) L.push(`| ${t.track} | \`${t.crate}\` | ${t.title} | \`${t.range}\` |`);
  L.push('| — | `au4a-node` | 节点二进制：编排 10 条轨道、CLI、只读观察面板、部署验证 | 全系列 |');
  L.push('');
  L.push('## 四、开发流程：每个小版本的六步闭环');
  L.push('');
  L.push('1. **实现**：在轨道 crate 内新增模块，单一职责。');
  L.push('2. **测试**：正常路径 + 拒绝路径 + 不变式。');
  L.push('3. **通过**：`cargo test -p <crate>`，0 failed、0 warning。');
  L.push('4. **快照**：`node E:\\DS\\_forangent\\snapshot.mjs <版本号> crates/<crate> docs/tracks/<track>.{md,json}`。');
  L.push('5. **文档**：更新该版本的元数据（目标/交付物/接口/验收/证据/状态）。');
  L.push('6. **下一版**：上一版未绿不得开下一版。');
  L.push('');
  L.push('并行约定：每条轨道使用独立 `CARGO_TARGET_DIR`（`E:\\DS\\_forangent\\target\\<crate>`），');
  L.push('避免 10 条轨道争抢 cargo 锁；因此开发期**不跑 `cargo test --workspace`**，集成测试由 Lead 在汇总阶段统一执行。');
  L.push('');
  L.push('## 五、99 个小版本逐项（开发视图）');
  L.push('');
  for (const t of tracks) {
    L.push(`### ${t.track.replace(/^1\./, 'v1.')} ${t.title}（\`${t.crate}\`，${t.count} 个小版本）`);
    L.push('');
    L.push('| 小版本 | 标题 | 目标 | 交付文件 | 测试命令 | 通过 | 等级 | 状态 |');
    L.push('|---|---|---|---|---|---|---|---|');
    for (const v of versions.filter((x) => x.track === t.track)) {
      const i = info(v);
      const files = i.deliverables.length ? i.deliverables.map((f) => f.split(':')[0]).join('<br>') : '—';
      L.push(
        `| \`${v.version}\` | ${i.title} | ${i.goal.replace(/\|/g, '/')} | ${files} | \`${i.evidence.test_command}\` | ${i.evidence.tests_passed}/${i.evidence.tests_total} | ${i.evidence.grade} | ${STATUS_CN[i.status] ?? i.status} |`,
      );
    }
    L.push('');
  }
  L.push('## 六、测试与验证策略');
  L.push('');
  L.push('| 层次 | 内容 |');
  L.push('|---|---|');
  L.push('| 单元测试 | 每个小版本的语义与边界（`crates/<crate>/src` 内 `#[cfg(test)]`） |');
  L.push('| 集成测试 | 跨模块流程（`crates/<crate>/tests/`） |');
  L.push('| 不变式测试 | 账本守恒、状态迁移幂等、事件链哈希连续 |');
  L.push('| 结构性测试 | 观察面只读（路由枚举断言）、否决权无提案能力 |');
  L.push('| 确定性测试 | 固定种子两次运行结果逐字节一致 |');
  L.push('| 跨 crate | `cargo test --workspace`（Lead 在集成阶段执行） |');
  L.push('');
  L.push('## 七、部署验证矩阵（10 个中版本收尾点）');
  L.push('');
  L.push('| 中版本 | 收尾版本 | 部署命令 | 实测命令 | 证据文件 |');
  L.push('|---|---|---|---|---|');
  for (const v of finals) {
    L.push(
      `| ${v.medium} ${v.medium_title} | \`${v.version}\` | \`cargo build --release -p au4a-node\` | \`au4a-node verify\` + \`au4a-node run --agents 8 --observe 127.0.0.1:8787\` | \`docs/verify/medium-${v.medium}.md\` |`,
    );
  }
  L.push('');
  L.push('每个收尾点的实测必须包含：构建成功、`verify` 全绿退出码 0、只读面板三个 GET 端点命中、非 GET 返回 405、端到端编排输出可复现。');
  L.push('');
  L.push('## 八、发布 SOP');
  L.push('');
  L.push('1. `node tools/gen-docs.mjs`（完整性闸门：10 中版本 + 99 小版本）。');
  L.push('2. `cargo test --workspace` 全绿；`cargo clippy --workspace` 无 warning（尽力而为，见风险 5）。');
  L.push('3. `node tools/publish.mjs plan` 核对远端现状。');
  L.push('4. `node tools/publish.mjs bootstrap --yes`（首次：归档旧 master、清理旧 tag/release）。');
  L.push('5. `node tools/publish.mjs versions`（99 个 commit + 99 个 tag，可断点续跑）。');
  L.push('6. `node tools/publish.mjs releases --yes`（10 个中版本 Release）。');
  L.push('7. `node tools/publish.mjs status` 核验 99 个 tag 指向与 10 个 Release。');
  L.push('');
  L.push('发布物与 tag 的关系：每个 tag 指向该版本提交，该提交的树由 `tools/materialize.mjs` 从逐版本快照重建，');
  L.push('即「测试通过时的那棵树」；`VERSION` 文件在发布时被戳成对应版本号（唯一的确定性变换）。');
  L.push('');
  L.push('## 九、风险与缓解');
  L.push('');
  L.push('| # | 风险 | 缓解 |');
  L.push('|---|---|---|');
  L.push('| 1 | 10 条轨道并行导致接口漂移 | 冻结基元 + 公共接口只增不改 + 契约文件 TEAM-BRIEF |');
  L.push('| 2 | 小版本被「只改文档」冒充 | 完整性闸门要求每版有交付物、验收与测试证据；快照逐版重建历史树 |');
  L.push('| 3 | 证据虚报 | 证据分级（verified / cpu-proto / unverified）+ 结算闸门：unverified 不可结算 |');
  L.push('| 4 | 集成分支冲突 | 轨道写作用域互斥；Lead 统一集成与 workspace 测试 |');
  L.push('| 5 | clippy/平台差异 | 以本机 Windows + Linux CI 双跑为准，未跑的平台标注为未验证 |');
  L.push('');
  L.push('---');
  L.push('');
  L.push('> 本文档由 `tools/gen-docs.mjs` 从 `docs/versions.json` 与 `docs/tracks/*.json` 生成；请勿手改。');
  return L.join('\n');
}

fs.writeFileSync(path.join(ROOT, 'docs', 'DESIGN.md'), designDoc() + '\n', 'utf8');
fs.writeFileSync(path.join(ROOT, 'docs', 'DEV.md'), devDoc() + '\n', 'utf8');

const design = fs.readFileSync(path.join(ROOT, 'docs', 'DESIGN.md'), 'utf8');
const dev = fs.readFileSync(path.join(ROOT, 'docs', 'DEV.md'), 'utf8');
const count = (s) => (s.match(/^#### v\d+\.\d+\.\d+/gm) ?? []).length + (s.match(/^\| `v\d+\.\d+\.\d+`/gm) ?? []).length;
console.log(`DESIGN.md ${design.split('\n').length} 行，版本条目 ${count(design)}`);
console.log(`DEV.md    ${dev.split('\n').length} 行，版本条目 ${count(dev)}`);
if (count(design) !== 99 || count(dev) !== 99) {
  console.error(`版本条目数不是 99（DESIGN ${count(design)} / DEV ${count(dev)}）——生成器自身缺陷`);
  process.exit(1);
}
