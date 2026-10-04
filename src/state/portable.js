// v1.3.0 — 可移植状态（Portable State）
// Agent 从节点 A 迁移到节点 B 不丢执行状态：快照 → 传输 → 验签 → 2PC 恢复。
// 两阶段提交：prepare(暂存) → commit(原子生效) → confirm(源节点确认)；
// 任何一步失败即 rollback，绝不产生部分状态。
import { createHash } from 'node:crypto';

function hashOf(data) {
  return createHash('sha256').update(JSON.stringify(data)).digest('hex');
}

export class Snapshot {
  constructor({ nodeId, data, hash, sig, at }) {
    this.nodeId = nodeId;
    this.data = data;      // { files: {}, memory: {}, context: {} }
    this.hash = hash;
    this.sig = sig;
    this.at = at;
  }
}

export function takeSnapshot(identity, nodeId, state) {
  const data = JSON.parse(JSON.stringify(state));
  const hash = hashOf(data);
  const sig = identity.keypair.sign(`AU4A-STATE\n${nodeId}\n${hash}`);
  return new Snapshot({ nodeId, data, hash, sig, at: Date.now() });
}

export function verifySnapshot(snap) {
  if (hashOf(snap.data) !== snap.hash) return { ok: false, reason: 'checksum_mismatch' };
  const verified = snap.publicKey
    ? snap.publicKey.verify(`AU4A-STATE\n${snap.nodeId}\n${snap.hash}`, snap.sig)
    : null;
  if (snap.publicKey && !verified) return { ok: false, reason: 'signature_invalid' };
  return { ok: true };
}

/** 单节点执行状态容器（files/memory/context） */
export class NodeState {
  constructor(initial = {}) {
    this.files = new Map(Object.entries(initial.files || {}));
    this.memory = new Map(Object.entries(initial.memory || {}));
    this.context = { ...(initial.context || {}) };
  }

  toJSON() {
    return {
      files: Object.fromEntries(this.files),
      memory: Object.fromEntries(this.memory),
      context: this.context,
    };
  }

  apply(snapshot) {
    const data = snapshot.data;
    // 构造全新状态，全部成功后才替换（原子生效；这里抛出即视为 apply 失败）
    const next = new NodeState({
      files: data.files || {},
      memory: data.memory || {},
      context: data.context || {},
    });
    this.files = next.files;
    this.memory = next.memory;
    this.context = next.context;
    return this;
  }
}

/** 迁移器：2PC */
export class Migrator {
  constructor({ source, target, identity, publicKey } = {}) {
    this.source = source;       // 源 NodeState
    this.target = target;       // 目标 NodeState
    this.identity = identity;   // 快照签名者
    this.publicKey = publicKey; // 验签公钥（默认 identity.keypair）
    this._staged = null;
    this._committed = false;
  }

  /** 阶段一：取快照并暂存到目标（不生效） */
  prepare() {
    const snap = takeSnapshot(this.identity, 'node-a', this.source.toJSON());
    snap.publicKey = this.publicKey || this.identity.keypair;
    const check = verifySnapshot(snap);
    if (!check.ok) throw new Error(`快照校验失败：${check.reason}`);
    this._staged = snap;
    return snap;
  }

  /** 阶段二：原子提交（apply 抛错 → rollback，目标保持原状态） */
  commit() {
    if (!this._staged) throw new Error('未 prepare');
    try {
      this.target.apply(this._staged);
      this._committed = true;
      return true;
    } catch (e) {
      this.rollback();
      throw e;
    }
  }

  /** 阶段三：源节点确认（迁移完成，可清理源） */
  confirm() {
    if (!this._committed) throw new Error('未 commit，不能确认');
    this._committed = false;
    this._staged = null;
    return { migrated: true, at: Date.now() };
  }

  /** 回滚：丢弃暂存，目标保持原状 */
  rollback() {
    this._staged = null;
    this._committed = false;
    return true;
  }
}

// —— v1.3.6 增量快照：块级 diff ——
// 只传输变更块（set/del），大状态迁移体积显著下降；可逐块校验后合并。

/** 计算两块状态的块级差异（files/memory/context 三区） */
export function diffState(prev, next) {
  const diff = {
    files: { set: {}, del: [] },
    memory: { set: {}, del: [] },
    context: { set: {}, del: [] },
  };
  for (const sec of ['files', 'memory', 'context']) {
    const p = prev[sec] || {};
    const n = next[sec] || {};
    const keys = new Set([...Object.keys(p), ...Object.keys(n)]);
    for (const k of keys) {
      if (!(k in n)) diff[sec].del.push(k);
      else if (JSON.stringify(p[k]) !== JSON.stringify(n[k])) diff[sec].set[k] = n[k];
    }
  }
  return diff;
}

/** 在基线上应用差异，返回新状态（不修改基线） */
export function applyDiff(base, diff) {
  const out = JSON.parse(JSON.stringify(base));
  for (const sec of ['files', 'memory', 'context']) {
    for (const k of diff[sec].del) delete out[sec][k];
    Object.assign(out[sec], diff[sec].set);
  }
  return out;
}
