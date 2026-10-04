// v1.3.5 — 分布式存储接口（StateStore）
// UDOS 分布式文件系统传输接口的内存实现：put/get/list，按 nodeId 索引快照。
// 主网实现仅需替换背书（put→网络写入、get→按 nodeId 拉取 + 验签）。
export class StateStore {
  constructor() {
    this.bucket = new Map(); // nodeId → Snapshot
  }

  put(nodeId, snapshot) {
    this.bucket.set(nodeId, snapshot);
    return { nodeId, storedAt: Date.now() };
  }

  get(nodeId) {
    return this.bucket.get(nodeId);
  }

  list() {
    return [...this.bucket.keys()];
  }

  remove(nodeId) {
    return this.bucket.delete(nodeId);
  }
}
