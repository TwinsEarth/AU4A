// v1.1.6 — 邻居能力图缓存（GraphCache）
// 降低查询延迟：did:skill → 能力缓存；LRU 容量上限；按 Agent 失效。
export class GraphCache {
  constructor({ maxEntries = 100 } = {}) {
    this.maxEntries = maxEntries;
    this.map = new Map();
  }

  static key(did, skill) {
    return `${did}:${skill}`;
  }

  set(did, skill, caps) {
    const k = GraphCache.key(did, skill);
    // 重写时先删旧键，保证 Map 顺序 = LRU 顺序
    this.map.delete(k);
    this.map.set(k, caps);
    while (this.map.size > this.maxEntries) {
      const oldest = this.map.keys().next().value;
      this.map.delete(oldest);
    }
    return caps;
  }

  get(did, skill) {
    const k = GraphCache.key(did, skill);
    const v = this.map.get(k);
    if (v !== undefined) {
      // 命中即提升为最新（LRU）
      this.map.delete(k);
      this.map.set(k, v);
    }
    return v;
  }

  /** Agent 能力变更时失效其全部缓存 */
  invalidate(did) {
    const prefix = `${did}:`;
    let removed = 0;
    for (const k of [...this.map.keys()]) {
      if (k.startsWith(prefix)) {
        this.map.delete(k);
        removed++;
      }
    }
    return removed;
  }

  size() {
    return this.map.size;
  }
}
