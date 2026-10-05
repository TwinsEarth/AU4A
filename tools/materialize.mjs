// materialize.mjs — 从逐版本快照重建任意版本的仓库树（可作模块导入，也可命令行调用）。
//
// 为什么需要它：10 条轨道在同一个工作树里并行开发，每条轨道在每个小版本测试通过后
// 把「自己的完整文件」快照到 E:\DS\_forangent\snapshots\<版本>\（保持仓库相对路径）。
// 发布 99 个版本时，需要的是**每个版本当时的确切树**，而不是最终树，所以这里按 seq 重放：
//
//   规则 1：目标版本 V 的树 = 对每个出现在「seq <= V.seq 的快照」里的文件路径 P，
//          取包含 P 的最新版本的那一份内容。
//   规则 2：从未被快照过的文件（genesis：根 Cargo.toml / au4a-core / docs / tools …）
//          从当前工作树复制。
//   规则 3：删除必须显式声明在 _forangent/deletions.json（[{"version":"vX.Y.Z","path":"..."}]），
//          因为「某文件不在快照里」无法区分「没改」和「删了」。
//
// 用法：
//   node tools/materialize.mjs --list
//   node tools/materialize.mjs --version v1.3.4 --out E:\DS\_forangent\mat\v1.3.4
//   node tools/materialize.mjs --version v1.9.9 --out DIR --from-root

import fs from 'node:fs';
import path from 'node:path';

const DEFAULT_ROOT = 'E:\\DS\\AU4A';
const DEFAULT_SNAP = 'E:\\DS\\_forangent\\snapshots';

const JUNK_DIRS = new Set(['target', '.git', '.snapshots', 'node_modules', '.pytest_cache']);
const JUNK_FILES = new Set(['.snapshot.json']);
const JUNK_EXT = ['.pdb', '.rs.bk'];

export function isJunk(rel) {
  const parts = rel.split('/');
  if (parts.some((p) => JUNK_DIRS.has(p))) return true;
  const base = parts[parts.length - 1];
  if (JUNK_FILES.has(base)) return true;
  return JUNK_EXT.some((e) => base.endsWith(e));
}

export function walk(dir, base = dir, out = []) {
  if (!fs.existsSync(dir)) return out;
  for (const name of fs.readdirSync(dir).sort()) {
    const abs = path.join(dir, name);
    const rel = path.relative(base, abs).replace(/\\/g, '/');
    if (isJunk(rel)) continue;
    const st = fs.statSync(abs);
    if (st.isDirectory()) walk(abs, base, out);
    else if (st.isFile()) out.push(rel);
  }
  return out;
}

export function loadManifest(root = DEFAULT_ROOT) {
  const manifestPath = path.join(root, 'docs', 'versions.json');
  const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
  const versions = manifest.versions.slice().sort((a, b) => a.seq - b.seq);
  return { manifest, versions, byVersion: new Map(versions.map((v) => [v.version, v])) };
}

export function listSnapshots({ snapshots = DEFAULT_SNAP, byVersion } = {}) {
  const map = byVersion ?? loadManifest().byVersion;
  if (!fs.existsSync(snapshots)) return [];
  return fs
    .readdirSync(snapshots)
    .filter((n) => /^v\d+\.\d+\.\d+$/.test(n) && fs.statSync(path.join(snapshots, n)).isDirectory())
    .filter((n) => map.has(n))
    .sort((a, b) => map.get(a).seq - map.get(b).seq);
}

/// 重建某个版本到 out 目录，返回统计。
export function materialize({ version, out, root = DEFAULT_ROOT, snapshots = DEFAULT_SNAP, fromRoot = false, deletionsFile = path.join(DEFAULT_SNAP, '..', 'deletions.json') }) {
  const { versions, byVersion } = loadManifest(root);
  const target = byVersion.get(version);
  if (!target) throw new Error(`unknown version: ${version}`);

  fs.rmSync(out, { recursive: true, force: true });
  fs.mkdirSync(out, { recursive: true });

  const chosen = new Map();
  let newestSnapshot = null;

  // ── 规则 0（v2.1.0 修复）：**按 crate 取单一来源**，且按「轮次」而非「版本序号」筛选 ──
  //
  // 为什么必须这样：第 11 轮里 10 条轨道同时发布各自的 `.10` 收尾版本。于是
  // `au4a-council`(轨道 1.7) 的同轮快照叫 `v1.7.10`、`au4a-chain`(1.8) 叫 `v1.8.10`、
  // `au4a-scale`(1.9) 叫 `v1.9.9` —— 它们的 **seq 都大于 `v1.6.10`**。
  // 旧规则 1 按文件 + `seq <= target.seq` 重放，会把这些同轮快照整体排除，
  // 使这些 crate 回落到工作树（最终版）内容：同一 crate 的 `src/` 与 `tests/` 来自不同轮次，
  // 于是 `cargo test` 报 `unresolved import au4a_council::audit` 这类"文件互相不认识"的错误。
  //
  // 现在的规则：对每个 crate，在「轮次 <= 目标轮次」的快照里取 **seq 最大的那一个**，
  // 用它的**整个 crate 目录**作为该 crate 的唯一来源 —— src 与 tests 必然同源。
  const crateSource = new Map(); // "crates/<name>/" -> 用作唯一来源的快照版本
  if (!fromRoot) {
    const eligible = listSnapshots({ snapshots, byVersion }).filter(
      (s) => byVersion.get(s).round <= target.round,
    );
    const filesOf = new Map(eligible.map((s) => [s, walk(path.join(snapshots, s))]));
    const crateNames = new Set();
    for (const files of filesOf.values()) {
      for (const rel of files) {
        const m = rel.match(/^crates\/([^/]+)\//);
        if (m) crateNames.add(m[1]);
      }
    }
    for (const name of [...crateNames].sort()) {
      const prefix = `crates/${name}/`;
      let best = null;
      for (const s of eligible) {
        // eligible 已按 seq 升序，后覆盖前 => 最终留下 seq 最大者
        if (filesOf.get(s).some((r) => r.startsWith(prefix))) best = s;
      }
      if (!best) continue;
      crateSource.set(prefix, best);
      const dir = path.join(snapshots, best);
      for (const rel of filesOf.get(best)) {
        if (rel.startsWith(prefix)) chosen.set(rel, path.join(dir, rel));
      }
    }
  }

  if (!fromRoot) {
    for (const s of listSnapshots({ snapshots, byVersion })) {
      if (byVersion.get(s).seq > target.seq) break;
      const snapDir = path.join(snapshots, s);
      if (!fs.existsSync(path.join(snapDir, '.snapshot.json'))) {
        throw new Error(`snapshot ${s} has no .snapshot.json — refusing to guess`);
      }
      for (const rel of walk(snapDir)) {
        if (rel.includes('..')) throw new Error(`snapshot ${s} contains an escaping path: ${rel}`);
        // 已由规则 0 确定唯一来源的 crate，不再按文件覆盖（否则又会混源）
        if ([...crateSource.keys()].some((p) => rel.startsWith(p))) continue;
        chosen.set(rel, path.join(snapDir, rel));
        newestSnapshot = s;
      }
    }
  }

  const deleted = new Set();
  if (fs.existsSync(deletionsFile)) {
    for (const d of JSON.parse(fs.readFileSync(deletionsFile, 'utf8'))) {
      const dv = byVersion.get(d.version);
      if (dv && dv.seq <= target.seq) deleted.add(String(d.path).replace(/\\/g, '/'));
    }
  }

  for (const rel of walk(root)) {
    if (deleted.has(rel)) continue;
    if (!chosen.has(rel)) chosen.set(rel, path.join(root, rel));
  }
  for (const rel of deleted) chosen.delete(rel);

  let bytes = 0;
  let fromSnapshots = 0;
  let fromRootCount = 0;
  for (const [rel, src] of [...chosen.entries()].sort()) {
    const dest = path.join(out, rel);
    fs.mkdirSync(path.dirname(dest), { recursive: true });
    fs.copyFileSync(src, dest);
    bytes += fs.statSync(dest).size;
    if (src.startsWith(snapshots)) fromSnapshots++;
    else fromRootCount++;
  }

  return {
    version,
    seq: target.seq,
    track: target.track,
    round: target.round,
    files: chosen.size,
    fromSnapshots,
    fromRoot: fromRootCount,
    bytes,
    newestSnapshot,
    deleted: deleted.size,
    out,
    totalVersions: versions.length,
  };
}

// ---------------- CLI ----------------
const isCli = process.argv[1] && path.resolve(process.argv[1]).endsWith('materialize.mjs');
if (isCli) {
  const argv = process.argv.slice(2);
  const arg = (name, dflt = null) => {
    const i = argv.indexOf(name);
    return i >= 0 && i + 1 < argv.length ? argv[i + 1] : dflt;
  };
  const has = (name) => argv.includes(name);
  const ROOT = arg('--root', DEFAULT_ROOT);
  const SNAP = arg('--snapshots', DEFAULT_SNAP);

  if (has('--list') || argv.length === 0) {
    const { versions, byVersion } = loadManifest(ROOT);
    const snaps = listSnapshots({ snapshots: SNAP, byVersion });
    if (!snaps.length) {
      console.log(`no snapshots yet in ${SNAP}`);
    } else {
      console.log('version'.padEnd(10), 'seq'.padStart(4), 'files'.padStart(6), '  union');
      const lastOf = new Map();
      for (const s of snaps) {
        const files = walk(path.join(SNAP, s));
        for (const f of files) lastOf.set(f, s);
        console.log(s.padEnd(10), String(byVersion.get(s).seq).padStart(4), String(files.length).padStart(6), `  ${lastOf.size}`);
      }
      const missing = versions.filter((v) => !snaps.includes(v.version)).map((v) => v.version);
      console.log(`\nsnapshots: ${snaps.length}/${versions.length}; missing: ${missing.length ? missing.join(', ') : 'none'}`);
    }
    process.exit(0);
  }

  const VERSION = arg('--version');
  const OUT = arg('--out');
  if (!VERSION || !OUT) {
    console.error('usage: node tools/materialize.mjs --version vX.Y.Z --out <dir> [--from-root] | --list');
    process.exit(2);
  }
  const stats = materialize({ version: VERSION, out: OUT, root: ROOT, snapshots: SNAP, fromRoot: has('--from-root') });
  console.log(
    `materialized ${stats.version} (seq ${stats.seq}, track ${stats.track}, round ${stats.round})\n` +
      `  files=${stats.files}  from_snapshots=${stats.fromSnapshots}  from_root=${stats.fromRoot}  bytes=${stats.bytes}\n` +
      `  newest_snapshot_used=${stats.newestSnapshot ?? 'none'}  deleted=${stats.deleted}\n` +
      `  out=${stats.out}`,
  );
}
