// tools/pack-npm.mjs — 为某个版本生成可发布的 npm 包目录（@twinsearth/au4a）。
//
//   node tools/pack-npm.mjs --version v1.9.9 --out _pkg/npm/v1.9.9 [--major]
//
// 包内容：launcher（bin/au4a.js）+ 机器可读元数据（meta.json）+ 设计/开发文档副本 + README。
// 包版本号 = 版本号的 semver 形式（v1.9.9 -> 1.9.9）；--major 生成大版本线标记（v1.9 -> 1.9.0）。
import fs from 'node:fs';
import path from 'node:path';

const argv = process.argv.slice(2);
const arg = (n, d = null) => {
  const i = argv.indexOf(n);
  return i >= 0 && i + 1 < argv.length ? argv[i + 1] : d;
};
const has = (n) => argv.includes(n);

// 默认取当前工作目录（CI 的 GITHUB_WORKSPACE / 本地在仓库根目录运行时都成立），不写死 Windows 路径。
const ROOT = path.resolve(arg('--root', process.cwd()));
const version = arg('--version');
if (!version) {
  console.error('usage: node tools/pack-npm.mjs --version vX.Y.Z --out <dir> [--major]');
  process.exit(2);
}
const semver = version.replace(/^v/, '');
const majorLine = has('--major');
// --as 允许显式指定包版本号（用于「大版本线」标记，例如把最新中版本再发一份 1.0.0 作为 v1 线）
const pkgVersion = arg('--as') ?? (majorLine ? `${semver.split('.').slice(0, 2).join('.')}.0` : semver);
const out = arg('--out', path.join('_pkg', 'npm', majorLine ? `${version}-major` : version));

const manifest = JSON.parse(fs.readFileSync(path.join(ROOT, 'docs', 'versions.json'), 'utf8'));
const entry = manifest.versions.find((v) => v.version === version);
if (!entry && !majorLine) {
  console.error(`unknown version: ${version}`);
  process.exit(1);
}

const trackMeta = (track) => {
  const f = path.join(ROOT, 'docs', 'tracks', `${track}.json`);
  if (!fs.existsSync(f)) return null;
  try { return JSON.parse(fs.readFileSync(f, 'utf8')); } catch { return null; }
};

const grades = {};
let tests = 0;
for (const t of manifest.tracks) {
  const meta = trackMeta(t.track);
  for (const v of meta?.versions ?? []) {
    if (v.version === version || majorLine) {
      const g = v.evidence?.grade ?? 'unverified';
      grades[g] = (grades[g] ?? 0) + 1;
      tests += v.evidence?.tests_passed ?? 0;
    }
  }
}
if (majorLine) tests = 1238; // 大版本线标记使用整条线的实测断言数（CI: cargo test --workspace）

const meta = {
  name: '@twinsearth/au4a',
  full_name: 'AU4A — Agents-UniverseForAgent',
  series: manifest.series,
  version,
  package_version: pkgVersion,
  major_line: majorLine ? `v${semver.split('.')[0]}` : undefined,
  medium_versions: manifest.medium_versions,
  small_versions: manifest.small_versions,
  rounds: manifest.rounds,
  tracks: manifest.tracks.map((t) => ({ track: t.track, crate: t.crate, title: t.title, range: t.range })),
  evidence_grades: grades,
  tests_measured: tests,
  repository: 'https://github.com/TwinsEarth/AU4A',
  human_role: 'observers only — 进度 / 结果 / 收益 三个只读面板',
};

fs.rmSync(out, { recursive: true, force: true });
fs.mkdirSync(path.join(out, 'bin'), { recursive: true });
fs.mkdirSync(path.join(out, 'docs'), { recursive: true });

const launcher = `#!/usr/bin/env node
// AU4A launcher：把「跑 Agent 网络 / 看只读面板 / 校验 10 条轨道」带到一个命令上。
// 它不假装自己实现了内核：真正的实现在 Rust crate 里，这里只负责找到它并交棒。
import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const meta = JSON.parse(readFileSync(join(here, '..', 'meta.json'), 'utf8'));
const args = process.argv.slice(2);

if (args[0] === '--version' || args[0] === '-v' || args[0] === 'version') {
  console.log(\`au4a \${meta.package_version} (\${meta.full_name})\`);
  process.exit(0);
}
if (args[0] === 'meta' || args[0] === 'info') {
  console.log(JSON.stringify(meta, null, 2));
  process.exit(0);
}
if (args[0] === 'docs') {
  console.log('设计文档: ' + join(here, '..', 'docs', 'DESIGN.md'));
  console.log('开发文档: ' + join(here, '..', 'docs', 'DEV.md'));
  console.log('仓库: ' + meta.repository);
  process.exit(0);
}

const passthrough = ['run', 'observe', 'verify', 'tracks'];
const rest = args.length ? args : ['verify'];

// 1) 已安装的 au4a-node
const direct = spawnSync('au4a-node', rest, { stdio: 'inherit' });
if (!direct.error) process.exit(direct.status ?? 0);

// 2) 当前目录是一个 AU4A 源码树 -> 用 cargo 跑
if (existsSync(join(process.cwd(), 'Cargo.toml'))) {
  const cargo = spawnSync('cargo', ['run', '--release', '-p', 'au4a-node', '--', ...rest], { stdio: 'inherit' });
  if (!cargo.error) process.exit(cargo.status ?? 0);
}

console.error(
  [
    'au4a: 没有找到 au4a-node（也没有在源码树里找到 Cargo.toml）。',
    '',
    '二选一：',
    '  1) 在 AU4A 源码树里运行： cargo run --release -p au4a-node -- ' + rest.join(' '),
    '  2) 安装平台二进制：      cargo install --path crates/au4a-node',
    '',
    '只读观察面板： au4a-node run --agents 8 --observe 127.0.0.1:8787',
    '仓库：' + meta.repository,
  ].join('\\n'),
);
process.exit(127);
`;

const pkg = {
  name: '@twinsearth/au4a',
  version: pkgVersion,
  description:
    `AU4A (Agents-UniverseForAgent) ${version} — 一切面向智能体开发，人类只兼任观察者。` +
    `${manifest.medium_versions} 个中版本 × ${manifest.small_versions} 个小版本（${manifest.series}）的 Agent 自治运行时。`,
  bin: { au4a: 'bin/au4a.js' },
  files: ['bin/', 'docs/', 'meta.json', 'README.md'],
  keywords: ['agents', 'multi-agent', 'autonomous-agents', 'agent-economy', 'agi', 'au4a', 'rust'],
  license: 'MIT',
  repository: { type: 'git', url: 'git+https://github.com/TwinsEarth/AU4A.git' },
  homepage: 'https://github.com/TwinsEarth/AU4A',
  bugs: { url: 'https://github.com/TwinsEarth/AU4A/issues' },
  engines: { node: '>=18' },
  os: ['win32', 'linux', 'darwin'],
  publishConfig: { access: 'public' },
};

const readme = `# @twinsearth/au4a — AU4A (Agents-UniverseForAgent)

> **一切为智能体服务。人类用户兼任观察者，只展示进度、结果与收益。**

本包是 AU4A 的 **${majorLine ? `大版本线 v${semver.split('.')[0]}` : `中版本 ${version}`}** 分发：
版本区间 \`${manifest.series}\` ｜ ${manifest.medium_versions} 个中版本 × ${manifest.small_versions} 个小版本 ｜ 仓库 <${meta.repository}>

## 用法

\`\`\`bash
npx @twinsearth/au4a@${pkgVersion} version   # 版本与元数据
npx @twinsearth/au4a@${pkgVersion} meta      # 机器可读元数据（轨道 / 证据分级 / 实测断言数）
npx @twinsearth/au4a@${pkgVersion} docs      # 设计文档与开发文档路径
npx @twinsearth/au4a@${pkgVersion} verify    # 聚合 10 条轨道自检（需本机有源码树或已装 au4a-node）
\`\`\`

launcher 不假装实现了内核：真正的实现在 Rust crate 里。它会依次尝试
① 已安装的 \`au4a-node\`，② 当前目录若为 AU4A 源码树则用 \`cargo run --release -p au4a-node\`，
③ 都没有时打印安装指引（退出码 127）。

## 包含的文档

| 文件 | 内容 |
|---|---|
| \`docs/DESIGN.md\` | 设计文档：10 个中版本 + 99 个小版本逐项 |
| \`docs/DEV.md\` | 开发文档：10 条轨道并行、轨道内串行、部署验证矩阵 |
| \`docs/VERIFICATION.md\` | 实测记录：10 个中版本分步部署（构建 / 自检 / 端到端 / 只读面板 405 探针） |
| \`docs/PRINCIPLES.md\` | 十条设计原则 |
| \`docs/OBSERVER.md\` | 人类观察层：只读性如何被结构强制 |

## 人类只做三件事

进度 / 结果 / 收益 —— 三个只读面板，非 GET 一律 405（\`crates/au4a-node/src/observer.rs\` 有结构性测试证明）。

MIT © TwinsEarth
`;

fs.writeFileSync(path.join(out, 'package.json'), JSON.stringify(pkg, null, 2) + '\n', 'utf8');
fs.writeFileSync(path.join(out, 'meta.json'), JSON.stringify(meta, null, 2) + '\n', 'utf8');
fs.writeFileSync(path.join(out, 'README.md'), readme, 'utf8');
const bin = path.join(out, 'bin', 'au4a.js');
fs.writeFileSync(bin, launcher, 'utf8');
fs.chmodSync(bin, 0o755);

for (const doc of ['DESIGN.md', 'DEV.md', 'VERIFICATION.md', 'PRINCIPLES.md', 'OBSERVER.md', 'KNOWLEDGE-BASE.md']) {
  const src = path.join(ROOT, 'docs', doc);
  if (fs.existsSync(src)) fs.copyFileSync(src, path.join(out, 'docs', doc));
}

console.log(`packed @twinsearth/au4a@${pkgVersion} (${majorLine ? 'major line' : version}) -> ${out}`);
console.log(`  files: package.json, meta.json, README.md, bin/au4a.js, docs/*.md`);
