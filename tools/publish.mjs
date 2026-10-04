// publish.mjs — 把 99 个版本与 10 个中版本 Release 发布到 GitHub。
//
// 这是**唯一**会写 GitHub 的脚本，只有 Lead 运行。它刻意做成可断点续跑：
// 每个版本的 commit/tree/tag 与文件哈希写进 `_forangent/publish-state.json`，
// 中途失败后重跑会跳过已完成版本，不会产生半个版本。
//
//   node tools/publish.mjs plan                     # 只读：打印计划与远端现状
//   node tools/publish.mjs bootstrap --yes          # 归档旧 master、清掉旧 release 与旧 tag
//   node tools/publish.mjs versions [--limit N]     # 逐版本提交 + 打 tag（可续跑）
//   node tools/publish.mjs releases [--yes]         # 10 个中版本 Release
//   node tools/publish.mjs status                   # 核验：99 tag / 10 release / master head
//
// 设计约束：
// * 每个版本的树 = materialize.mjs 重建的该版本树 + VERSION 文件戳（确定性变换）。
// * blob 只在内容变化时上传（对上一版本做哈希 diff），因此 99 个版本的 API 调用量可控。
// * commit 按 docs/versions.json 的 seq 线性排列，轮次交错体现「10 条轨道并行、轨道内串行」。

import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { materialize, walk } from './materialize.mjs';

const ROOT = 'E:\\DS\\AU4A';
const WORK = 'E:\\DS\\_forangent';
const MAT = path.join(WORK, 'mat');
const STATE_FILE = path.join(WORK, 'publish-state.json');
const TOKEN_FILE = 'E:\\DS\\_work\\token.json';
const REPO = 'TwinsEarth/AU4A';
const BRANCH = 'master';
const ARCHIVE_BRANCH = 'archive/js-line-2026-10-04';

const argv = process.argv.slice(2);
const has = (n) => argv.includes(n);
const arg = (n, d = null) => {
  const i = argv.indexOf(n);
  return i >= 0 && i + 1 < argv.length ? argv[i + 1] : d;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

if (has('--token-file')) {
  // override allowed for testing
}
const TOKEN = JSON.parse(fs.readFileSync(arg('--token-file', TOKEN_FILE), 'utf8')).access_token;
const H = { Authorization: `token ${TOKEN}`, Accept: 'application/vnd.github+json', 'User-Agent': 'au4a-publish' };

async function api(method, p, body, { allow404 = false } = {}) {
  for (let attempt = 0; attempt < 5; attempt++) {
    try {
      const res = await fetch(`https://api.github.com${p}`, {
        method,
        headers: body ? { ...H, 'Content-Type': 'application/json' } : H,
        body: body ? JSON.stringify(body) : undefined,
      });
      const text = await res.text();
      let json = null;
      try { json = text ? JSON.parse(text) : null; } catch { /* keep raw */ }
      if (allow404 && res.status === 404) return null;
      if (res.status === 403 || res.status === 429 || res.status >= 500) {
        const wait = 3000 * (attempt + 1);
        console.log(`  … ${method} ${p} -> ${res.status}, retry in ${wait}ms`);
        await sleep(wait);
        continue;
      }
      if (!res.ok) throw new Error(`${method} ${p} -> ${res.status} ${text.slice(0, 300)}`);
      return json;
    } catch (e) {
      if (attempt === 4) throw e;
      await sleep(1500 * (attempt + 1));
    }
  }
  throw new Error(`unreachable: ${method} ${p}`);
}

const manifest = JSON.parse(fs.readFileSync(path.join(ROOT, 'docs', 'versions.json'), 'utf8'));
const versions = manifest.versions.slice().sort((a, b) => a.seq - b.seq);
const byVersion = new Map(versions.map((v) => [v.version, v]));
const mediumFinals = versions.filter((v) => v.medium_final);

function loadState() {
  if (fs.existsSync(STATE_FILE)) return JSON.parse(fs.readFileSync(STATE_FILE, 'utf8'));
  return { repo: REPO, versions: {}, lastCommit: null, lastTree: null, lastFiles: null };
}
function saveState(s) {
  fs.mkdirSync(WORK, { recursive: true });
  fs.writeFileSync(STATE_FILE, JSON.stringify(s, null, 2) + '\n', 'utf8');
}

function sha256File(p) {
  return crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
}
function hashDir(dir) {
  const out = {};
  for (const rel of walk(dir)) out[rel] = sha256File(path.join(dir, rel));
  return out;
}

/// 读取该版本的轨道元数据（标题/目标/证据），用于 commit message 与 release notes。
function trackMeta(track) {
  const f = path.join(ROOT, 'docs', 'tracks', `${track}.json`);
  if (!fs.existsSync(f)) return null;
  try { return JSON.parse(fs.readFileSync(f, 'utf8')); } catch { return null; }
}

function versionInfo(v) {
  const meta = trackMeta(v.track);
  const entry = meta?.versions?.find((x) => x.version === v.version);
  return {
    title: entry?.title ?? v.title,
    goal: entry?.goal ?? '',
    acceptance: entry?.acceptance ?? [],
    evidence: entry?.evidence ?? null,
    status: entry?.status ?? 'unknown',
  };
}

function commitMessage(v) {
  const info = versionInfo(v);
  const head = `${v.version} ${info.title}（轨道 ${v.track} · ${v.medium_title}）`;
  const body = [info.goal, info.evidence ? `测试：${info.evidence.test_command} → ${info.evidence.tests_passed}/${info.evidence.tests_total}（${info.evidence.grade}）` : '']
    .filter(Boolean)
    .join('\n');
  return body ? `${head}\n\n${body}` : head;
}

// ---------------- plan ----------------
async function cmdPlan() {
  const repo = await api('GET', `/repos/${REPO}`);
  const refs = await api('GET', `/repos/${REPO}/git/refs/tags?per_page=100`);
  const releases = await api('GET', `/repos/${REPO}/releases?per_page=100`);
  const state = loadState();
  const wanted = new Set(versions.map((v) => v.version));
  const existing = refs.map((r) => r.ref.replace('refs/tags/', ''));
  const stale = existing.filter((t) => /^v1\./.test(t) && !wanted.has(t));
  const overlap = existing.filter((t) => wanted.has(t));
  const missing = versions.map((v) => v.version).filter((t) => !existing.includes(t));
  console.log(`repo            ${repo.full_name} (default ${repo.default_branch}, pushed ${repo.pushed_at})`);
  console.log(`manifest        10 medium versions, 99 small versions, ${manifest.rounds} rounds`);
  console.log(`release points  ${mediumFinals.map((v) => v.version).join(', ')}`);
  console.log(`tags wanted     99  existing-v1  ${existing.filter((t) => /^v1\./.test(t)).length}  overlap ${overlap.length}  missing ${missing.length}`);
  console.log(`tags stale      ${stale.length ? stale.join(', ') : 'none'}`);
  console.log(`releases        ${releases.length ? releases.map((r) => r.tag_name).join(', ') : 'none'}`);
  console.log(`state           ${Object.keys(state.versions).length}/99 published, lastCommit=${state.lastCommit ?? 'none'}`);
  console.log(`bootstrap plan  archive ${BRANCH} -> ${ARCHIVE_BRANCH}; delete ${stale.length} stale tag(s); delete ${releases.length} stale release(s); then publish v1.0.1 as a fresh root commit`);
}

// ---------------- bootstrap ----------------
async function cmdBootstrap() {
  if (!has('--yes')) {
    console.error('bootstrap rewrites refs; re-run with --yes');
    process.exit(2);
  }
  const ref = await api('GET', `/repos/${REPO}/git/ref/heads/${BRANCH}`);
  const head = ref.object.sha;
  console.log(`current ${BRANCH} = ${head.slice(0, 10)}`);

  const arch = await api('GET', `/repos/${REPO}/git/ref/heads/${ARCHIVE_BRANCH}`, null, { allow404: true });
  if (!arch) {
    await api('POST', `/repos/${REPO}/git/refs`, { ref: `refs/heads/${ARCHIVE_BRANCH}`, sha: head });
    console.log(`archived to ${ARCHIVE_BRANCH} @ ${head.slice(0, 10)}`);
  } else {
    console.log(`archive branch already exists @ ${arch.object.sha.slice(0, 10)}`);
  }

  const releases = await api('GET', `/repos/${REPO}/releases?per_page=100`);
  for (const r of releases) {
    await api('DELETE', `/repos/${REPO}/releases/${r.id}`);
    console.log(`deleted stale release ${r.tag_name}`);
  }

  const wanted = new Set(versions.map((v) => v.version));
  const refs = await api('GET', `/repos/${REPO}/git/refs/tags?per_page=100`);
  for (const r of refs) {
    const name = r.ref.replace('refs/tags/', '');
    if (/^v1\./.test(name) && !wanted.has(name)) {
      await api('DELETE', `/repos/${REPO}/git/refs/tags/${name}`);
      console.log(`deleted stale tag ${name}`);
    }
  }
  console.log('bootstrap done (tags overlapping the 99-version set will be force-updated during `versions`)');
}

// ---------------- versions ----------------
async function cmdVersions() {
  const limit = Number(arg('--limit', '0')) || Infinity;
  const state = loadState();
  let prevCommit = state.lastCommit;
  let prevTree = state.lastTree;
  let prevFiles = state.lastFiles;
  let published = 0;

  for (const v of versions) {
    if (state.versions[v.version]) {
      const s = state.versions[v.version];
      prevCommit = s.commit; prevTree = s.tree; prevFiles = s.files;
      continue;
    }
    if (published >= limit) break;

    const dir = path.join(MAT, v.version);
    const stats = materialize({ version: v.version, out: dir, root: ROOT });
    fs.writeFileSync(path.join(dir, 'VERSION'), `${v.version}\n`, 'utf8');
    const files = hashDir(dir);

    const entries = [];
    let uploaded = 0;
    for (const [rel, hash] of Object.entries(files)) {
      if (prevFiles && prevFiles[rel] === hash) continue;
      const content = fs.readFileSync(path.join(dir, rel)).toString('base64');
      const blob = await api('POST', `/repos/${REPO}/git/blobs`, { content, encoding: 'base64' });
      entries.push({ path: rel, mode: '100644', type: 'blob', sha: blob.sha });
      uploaded++;
    }
    const removed = prevFiles ? Object.keys(prevFiles).filter((rel) => !(rel in files)) : [];
    for (const rel of removed) entries.push({ path: rel, mode: '100644', type: 'blob', sha: null });

    const treeBody = { tree: entries };
    if (prevTree) treeBody.base_tree = prevTree;
    const tree = await api('POST', `/repos/${REPO}/git/trees`, treeBody);

    const message = commitMessage(v);
    const commitBody = { message, tree: tree.sha };
    if (prevCommit) commitBody.parents = [prevCommit];
    const commit = await api('POST', `/repos/${REPO}/git/commits`, commitBody);

    await api('PATCH', `/repos/${REPO}/git/refs/heads/${BRANCH}`, { sha: commit.sha, force: !prevCommit });

    const tagObj = await api('POST', `/repos/${REPO}/git/tags`, { tag: v.version, message, object: commit.sha, type: 'commit' });
    const existingRef = await api('GET', `/repos/${REPO}/git/ref/tags/${v.version}`, null, { allow404: true });
    if (existingRef) await api('PATCH', `/repos/${REPO}/git/refs/tags/${v.version}`, { sha: tagObj.sha, force: true });
    else await api('POST', `/repos/${REPO}/git/refs`, { ref: `refs/tags/${v.version}`, sha: tagObj.sha });

    state.versions[v.version] = { commit: commit.sha, tree: tree.sha, tag: tagObj.sha, files, materialized: stats.files };
    state.lastCommit = commit.sha;
    state.lastTree = tree.sha;
    state.lastFiles = files;
    saveState(state);

    prevCommit = commit.sha; prevTree = tree.sha; prevFiles = files;
    published++;
    console.log(
      `${String(v.seq).padStart(3)}/99 ${v.version.padEnd(8)} round ${String(v.round).padStart(2)} ${v.track}  ` +
        `files=${stats.files} up=${uploaded} del=${removed.length} commit=${commit.sha.slice(0, 10)}`,
    );
  }
  console.log(`published ${published} version(s) this run; total ${Object.keys(state.versions).length}/99`);
}

// ---------------- releases ----------------
function releaseNotes(v) {
  const meta = trackMeta(v.track);
  const trackVersions = versions.filter((x) => x.track === v.track);
  const lines = [];
  lines.push(`## ${v.medium_title}（中版本 ${v.medium}）`);
  lines.push('');
  lines.push(`本 Release 对应中版本 \`${v.medium}\` 的收尾小版本 **${v.version}**（该轨道共 ${trackVersions.length} 个小版本，串行交付）。`);
  lines.push('');
  lines.push('| 小版本 | 标题 | 目标 | 测试 | 证据 |');
  lines.push('|---|---|---|---|---|');
  for (const tv of trackVersions) {
    const e = meta?.versions?.find((x) => x.version === tv.version);
    const tests = e?.evidence ? `${e.evidence.tests_passed}/${e.evidence.tests_total}` : '—';
    lines.push(`| \`${tv.version}\` | ${e?.title ?? tv.title} | ${(e?.goal ?? '—').replace(/\|/g, '/')} | ${tests} | ${e?.evidence?.grade ?? '—'} |`);
  }
  lines.push('');
  const verifyFile = path.join(ROOT, 'docs', 'verify', `medium-${v.medium}.md`);
  if (fs.existsSync(verifyFile)) {
    lines.push('## 分步部署实测');
    lines.push('');
    lines.push(fs.readFileSync(verifyFile, 'utf8').trim());
  } else {
    lines.push('## 分步部署实测');
    lines.push('');
    lines.push('本中版本的部署实测记录见 `docs/VERIFICATION.md`。');
  }
  lines.push('');
  lines.push(`安装：\`cargo build --release -p au4a-node\`；观察：\`au4a-node run --agents 8 --observe 127.0.0.1:8787\`（只读面板）。`);
  return lines.join('\n');
}

async function cmdReleases() {
  if (!has('--yes')) {
    console.error('creating releases writes to GitHub; re-run with --yes');
    process.exit(2);
  }
  for (const v of mediumFinals) {
    const existing = await api('GET', `/repos/${REPO}/releases/tags/${v.version}`, null, { allow404: true });
    if (existing) {
      console.log(`release ${v.version} already exists: ${existing.html_url}`);
      continue;
    }
    const rel = await api('POST', `/repos/${REPO}/releases`, {
      tag_name: v.version,
      name: `${v.version} — ${v.medium_title}`,
      body: releaseNotes(v),
      draft: false,
      prerelease: false,
    });
    console.log(`release ${v.version} -> ${rel.html_url}`);
  }
}

// ---------------- status ----------------
async function cmdStatus() {
  const state = loadState();
  const tags = await api('GET', `/repos/${REPO}/git/refs/tags?per_page=100`);
  const tagMap = new Map(tags.map((r) => [r.ref.replace('refs/tags/', ''), r.object.sha]));
  let ok = 0;
  const problems = [];
  for (const v of versions) {
    const s = state.versions[v.version];
    if (!s) { problems.push(`${v.version}: not published`); continue; }
    const tagSha = tagMap.get(v.version);
    if (!tagSha) { problems.push(`${v.version}: tag missing`); continue; }
    const tagObj = await api('GET', `/repos/${REPO}/git/tags/${tagSha}`, null, { allow404: true });
    if (!tagObj) { problems.push(`${v.version}: tag object unreadable`); continue; }
    if (tagObj.object.sha !== s.commit) problems.push(`${v.version}: tag -> ${tagObj.object.sha.slice(0, 8)} != commit ${s.commit.slice(0, 8)}`);
    else ok++;
  }
  const ref = await api('GET', `/repos/${REPO}/git/ref/heads/${BRANCH}`);
  const releases = await api('GET', `/repos/${REPO}/releases?per_page=100`);
  console.log(`tags verified   ${ok}/99`);
  console.log(`master head     ${ref.object.sha.slice(0, 10)} (expected ${String(state.lastCommit).slice(0, 10)})`);
  console.log(`releases        ${releases.length}/10 -> ${releases.map((r) => r.tag_name).join(', ')}`);
  if (problems.length) {
    console.log(`problems (${problems.length}):`);
    for (const p of problems.slice(0, 20)) console.log(`  - ${p}`);
    process.exit(1);
  }
}

const cmd = argv[0];
if (cmd === 'plan') await cmdPlan();
else if (cmd === 'bootstrap') await cmdBootstrap();
else if (cmd === 'versions') await cmdVersions();
else if (cmd === 'releases') await cmdReleases();
else if (cmd === 'status') await cmdStatus();
else {
  console.error('usage: node tools/publish.mjs <plan|bootstrap|versions|releases|status> [--yes] [--limit N]');
  process.exit(2);
}
