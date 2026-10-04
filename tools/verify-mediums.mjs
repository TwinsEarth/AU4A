// verify-mediums.mjs — 10 个中版本的「分步部署 + 实测」。
//
// 对每个中版本收尾点（v1.0.10 / v1.1.10 / … / v1.9.9）：
//   1. 用 tools/materialize.mjs 从逐版本快照重建**该版本当时的树**（不是最终树）；
//   2. 戳 VERSION，`cargo build --release -p au4a-node` 真实构建；
//   3. 跑 `au4a-node version` / `verify --json` / `run --agents 8 --json`；
//   4. 起只读观察面，真实 HTTP GET 三个面板 + selfcheck，并用 POST 证明写路径不存在（405）；
//   5. 把命令、退出码、输出、耗时写进 docs/verify/medium-<中版本>.md，再汇总 docs/VERIFICATION.md。
//
//   node tools/verify-mediums.mjs [--only v1.3.10] [--agents 8] [--json]

import fs from 'node:fs';
import path from 'node:path';
import { execFileSync, spawn } from 'node:child_process';
import { materialize } from './materialize.mjs';

const ROOT = 'E:\\DS\\agent-universeForAngent';
const WORK = 'E:\\DS\\_forangent\\verify';
const TARGET_ROOT = 'E:\\DS\\_forangent\\verify-target';
const argv = process.argv.slice(2);
const arg = (n, d = null) => {
  const i = argv.indexOf(n);
  return i >= 0 && i + 1 < argv.length ? argv[i + 1] : d;
};
const only = arg('--only');
const agents = arg('--agents', '8');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const manifest = JSON.parse(fs.readFileSync(path.join(ROOT, 'docs', 'versions.json'), 'utf8'));
const versions = manifest.versions.slice().sort((a, b) => a.seq - b.seq);
const finals = versions.filter((v) => v.medium_final).filter((v) => !only || v.version === only);
if (!finals.length) {
  console.error(`no medium-finals matched (--only ${only})`);
  process.exit(2);
}

function targetFor(version) {
  return path.join(TARGET_ROOT, version);
}

function run(cmd, args, opts = {}) {
  const started = Date.now();
  try {
    const stdout = execFileSync(cmd, args, {
      cwd: opts.cwd,
      encoding: 'utf8',
      env: { ...process.env, CARGO_TARGET_DIR: opts.target ?? targetFor(opts.version ?? "shared"), ...(opts.env ?? {}) },
      timeout: opts.timeout ?? 15 * 60 * 1000,
      maxBuffer: 64 * 1024 * 1024,
    });
    return { code: 0, stdout, stderr: '', ms: Date.now() - started };
  } catch (e) {
    return {
      code: e.status ?? -1,
      stdout: String(e.stdout ?? ''),
      stderr: String(e.stderr ?? e.message),
      ms: Date.now() - started,
    };
  }
}

async function httpGet(url, method = 'GET') {
  try {
    const res = await fetch(url, { method });
    const text = await res.text();
    return { status: res.status, len: text.length, body: text.slice(0, 400) };
  } catch (e) {
    return { status: 0, len: 0, body: String(e.message) };
  }
}

/// 找一个空闲端口（避免固定端口撞车）。
async function freePort(start = 18800) {
  for (let p = start; p < start + 200; p++) {
    try {
      const res = await fetch(`http://127.0.0.1:${p}/`, { signal: AbortSignal.timeout(150) });
      if (res.status) continue; // something is listening
    } catch {
      return p;
    }
  }
  return start;
}

const results = [];
fs.mkdirSync(path.join(ROOT, 'docs', 'verify'), { recursive: true });

for (const v of finals) {
  console.log(`\n=== ${v.medium} ${v.medium_title} — ${v.version} ===`);
  const dir = path.join(WORK, v.version);
  const stats = materialize({ version: v.version, out: dir, root: ROOT });
  fs.writeFileSync(path.join(dir, 'VERSION'), `${v.version}\n`, 'utf8');
  console.log(`materialized ${stats.files} files (snapshots=${stats.fromSnapshots}, root=${stats.fromRoot})`);

  const versionTarget = targetFor(v.version);
  const build = run('cargo', ['build', '--release', '-p', 'au4a-node'], { cwd: dir, timeout: 30 * 60 * 1000, target: versionTarget });
  console.log(`build exit=${build.code} in ${(build.ms / 1000).toFixed(1)}s`);
  if (build.code !== 0) {
    console.log(build.stderr.split('\n').slice(-12).join('\n'));
    results.push({ version: v.version, medium: v.medium, build, skipped: true });
    continue;
  }
  const bin = path.join(versionTarget, 'release', process.platform === 'win32' ? 'au4a-node.exe' : 'au4a-node');

  const versionOut = run(bin, ['version'], { cwd: dir });
  const verify = run(bin, ['verify', '--json'], { cwd: dir });
  const runOut = run(bin, ['run', '--agents', String(agents), '--json'], { cwd: dir });
  const failingChecks = [
    ...String(verify.stdout).matchAll(/"name": "([^"]+)",\s*\n\s*"passed": false/g),
  ].map((m) => m[1]);

  // live read-only observer check
  const port = await freePort();
  const child = spawn(bin, ['run', '--agents', String(agents), '--observe', `127.0.0.1:${port}`], {
    cwd: dir,
    env: { ...process.env, CARGO_TARGET_DIR: versionTarget },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let serverLog = '';
  child.stdout.on('data', (d) => (serverLog += d));
  child.stderr.on('data', (d) => (serverLog += d));
  let panel = { root: null, progress: null, results: null, revenue: null, selfcheck: null, tracks: null, writeProbe: null };
  for (let i = 0; i < 40; i++) {
    await sleep(250);
    const probe = await httpGet(`http://127.0.0.1:${port}/api/progress`);
    if (probe.status === 200) break;
  }
  panel.root = await httpGet(`http://127.0.0.1:${port}/`);
  panel.progress = await httpGet(`http://127.0.0.1:${port}/api/progress`);
  panel.results = await httpGet(`http://127.0.0.1:${port}/api/results`);
  panel.revenue = await httpGet(`http://127.0.0.1:${port}/api/revenue`);
  panel.selfcheck = await httpGet(`http://127.0.0.1:${port}/api/selfcheck`);
  panel.tracks = await httpGet(`http://127.0.0.1:${port}/api/tracks`);
  panel.writeProbe = await httpGet(`http://127.0.0.1:${port}/api/revenue`, 'POST');
  child.kill();

  const readOnlyOk = panel.writeProbe.status === 405 && panel.root.status === 200;
  console.log(`verify exit=${verify.code} run exit=${runOut.code} panel=${panel.progress.status}/${panel.revenue.status} writeProbe=${panel.writeProbe.status}`);

  // ---- evidence file ----
  const lines = [];
  lines.push(`# 中版本 ${v.medium} 部署实测 — ${v.medium_title}`);
  lines.push('');
  lines.push(`> 收尾小版本：\`${v.version}\`　｜　轨道：${v.track}（\`${v.crate}\`）　｜　轮次：第 ${v.round} 轮`);
  lines.push(`> 实测平台：Windows / ${process.platform} ${process.arch}；Rust ${run('rustc', ['--version']).stdout.trim()}`);
  lines.push('');
  lines.push('## 1. 部署（构建）');
  lines.push('');
  lines.push('```');
  lines.push(`$ node tools/materialize.mjs --version ${v.version} --out <mat>`);
  lines.push(`materialized ${stats.files} files (from_snapshots=${stats.fromSnapshots}, from_root=${stats.fromRoot})`);
  lines.push(`$ cargo build --release -p au4a-node`);
  lines.push(`exit=${build.code}  ${(build.ms / 1000).toFixed(1)}s`);
  lines.push('```');
  lines.push('');
  lines.push('## 2. 版本与自检');
  lines.push('');
  lines.push('```');
  lines.push(`$ au4a-node version`);
  lines.push(versionOut.stdout.trim());
  lines.push(`$ au4a-node verify --json`);
  lines.push(`exit=${verify.code}`);
  lines.push(verify.stdout.trim().slice(0, 3000));
  lines.push('```');
  lines.push('');
  lines.push('## 3. 端到端运行（Agent 自主）');
  lines.push('');
  lines.push('```');
  lines.push(`$ au4a-node run --agents ${agents} --json`);
  lines.push(`exit=${runOut.code}`);
  lines.push(runOut.stdout.trim().slice(0, 3000));
  lines.push('```');
  lines.push('');
  lines.push('## 4. 人类观察面板（只读）');
  lines.push('');
  lines.push('| 端点 | 方法 | 状态码 | 字节 |');
  lines.push('|---|---|---|---|');
  lines.push(`| \`/\` | GET | ${panel.root.status} | ${panel.root.len} |`);
  lines.push(`| \`/api/progress\` | GET | ${panel.progress.status} | ${panel.progress.len} |`);
  lines.push(`| \`/api/results\` | GET | ${panel.results.status} | ${panel.results.len} |`);
  lines.push(`| \`/api/revenue\` | GET | ${panel.revenue.status} | ${panel.revenue.len} |`);
  lines.push(`| \`/api/selfcheck\` | GET | ${panel.selfcheck.status} | ${panel.selfcheck.len} |`);
  lines.push(`| \`/api/tracks\` | GET | ${panel.tracks.status} | ${panel.tracks.len} |`);
  lines.push(`| \`/api/revenue\` | **POST** | ${panel.writeProbe.status}（必须 405） | ${panel.writeProbe.len} |`);
  lines.push('');
  lines.push(`只读性判定：**${readOnlyOk ? '通过' : '未通过'}**（GET 面板可用且 POST 被 405 拒绝）。`);
  lines.push('');
  lines.push('## 5. 结论');
  lines.push('');
  lines.push(`- 部署：${build.code === 0 ? '成功' : '失败'}`);
  lines.push(`- 自检：${verify.code === 0 ? '全绿' : '未通过'}（exit ${verify.code}）`);
  lines.push(`- 端到端：${runOut.code === 0 ? '成功' : '失败'}（exit ${runOut.code}）`);
  lines.push(`- 只读观察面：${readOnlyOk ? '通过' : '未通过'}`);
  lines.push('');
  lines.push('---');
  lines.push('');
  lines.push('> 本文件由 `tools/verify-mediums.mjs` 生成；命令与输出为真实运行结果。');
  fs.writeFileSync(path.join(ROOT, 'docs', 'verify', `medium-${v.medium}.md`), lines.join('\n') + '\n', 'utf8');

  results.push({ version: v.version, medium: v.medium, title: v.medium_title, build: build.code, verify: verify.code, run: runOut.code, readOnlyOk, files: stats.files, ms: build.ms, failingChecks, isLast: v.version === finals[finals.length - 1].version });
}

// ---- summary ----
const S = [];
S.push('# AU4A 实测与验证（VERIFICATION）');
S.push('');
S.push('> 本文件由 `tools/verify-mediums.mjs` 生成：对 10 个中版本收尾点逐个**重建当时的版本树 → 真实构建 → 真实运行 → 只读面板核验**。');
S.push('> 证据分级：本机 Windows 实测为 `verified`；模拟/内存级实现为 `cpu-proto`；未实测项标 `unverified`。');
S.push('');
S.push('## 汇总');
S.push('');
S.push('| 中版本 | 收尾版本 | 代号 | 构建 | verify | 端到端 | 只读面 | 文件数 | 构建耗时 | 证据 |');
S.push('|---|---|---|---|---|---|---|---|---|---|');
for (const r of results) {
  S.push(
    `| ${r.medium} | \`${r.version}\` | ${r.title} | ${r.build === 0 ? '✅' : r.skipped ? '⏭️' : '❌'} | ${r.verify === 0 ? '✅' : '❌'} | ${r.run === 0 ? '✅' : '❌'} | ${r.readOnlyOk ? '✅' : '❌'} | ${r.files} | ${((r.ms ?? 0) / 1000).toFixed(1)}s | [medium-${r.medium}.md](verify/medium-${r.medium}.md) |`,
  );
}
S.push('');
S.push('## 每步做了什么');
S.push('');
S.push('1. `tools/materialize.mjs` 从逐版本快照重建**该版本当时的树**（不是最终树），并戳 `VERSION`。');
S.push('2. `cargo build --release -p au4a-node` 真实构建。');
S.push('3. `au4a-node version` / `verify --json` 核验版本号与 10 条轨道自检。');
S.push(`4. \`au4a-node run --agents 8 --json\` 让 Agent 自主跑完整链路。`);
S.push('5. 起只读观察面，GET 三个面板与自检；再用 **POST** 证明写路径不存在（必须 405）。');
S.push('');
S.push('## 未验证项（诚实清单）');
S.push('');
S.push('- 跨链部分（v1.8）是确定性测试网适配器，**不是真实链上交易**：证据等级 `cpu-proto`。');
S.push('- 状态迁移（v1.3）为本地双节点内存实现，**不是真实网络传输**：证据等级 `cpu-proto`。');
S.push('- 10k 节点（v1.9）为确定性聚合模拟，**不是真实分布式压测**：证据等级 `cpu-proto`。');
S.push('- 仅 Windows 本机实测；Linux/macOS 未在本轮运行。');
S.push('');
S.push('---');
S.push('');
S.push('> 本文档由 `tools/verify-mediums.mjs` 生成；请勿手改。');
fs.writeFileSync(path.join(ROOT, 'docs', 'VERIFICATION.md'), S.join('\n') + '\n', 'utf8');

console.log(`\nwrote docs/VERIFICATION.md and ${results.length} medium evidence file(s)`);
const failed = results.filter((r) => r.build !== 0 || r.run !== 0 || !r.readOnlyOk);
const verifyFailed = results.filter((r) => r.verify !== 0);
if (verifyFailed.length) {
  console.log(`verify 未全绿的中版本（历史版本自检，见 docs/tracks/1.1.md 潜伏缺陷披露）: ${verifyFailed.map((r) => `${r.version}[${r.failingChecks.join('|')}]`).join(', ')}`);
}
const lastOk = results.length && results[results.length - 1].verify === 0;
if (failed.length || !lastOk) {
  console.error(`FAILED medium versions: ${failed.map((f) => `${f.version}(build=${f.build},run=${f.run},ro=${f.readOnlyOk})`).join(', ')}${lastOk ? '' : ' | final version verify != 0'}`);
  process.exit(1);
}
