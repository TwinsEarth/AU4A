// v1.3.9 — 灾难恢复演练（真机可运行）
// 场景：节点 A 状态经 StateStore 备份；节点 A 源丢失 → 从存储重建到节点 B → 校验恢复点继续。
// 运行：node examples/recovery.js
import au from '@twinsearth/agent-universe';
import { NodeState, takeSnapshot, verifySnapshot, Migrator, diffState, applyDiff } from '../src/state/portable.js';
import { StateStore } from '../src/state/store.js';

const { Keypair } = au;

function log(step, ok, detail) {
  console.log(`[${ok ? 'PASS' : 'FAIL'}] ${step} — ${detail}`);
  if (!ok) process.exitCode = 1;
}

const ident = { keypair: Keypair.generate() };

// 1) 节点 A 运行并产生状态
const nodeA = new NodeState({ files: { config: '{"mode":"prod"}' }, memory: { step: 7, buffer: 'seg0' }, context: { goal: '聚合报告', owner: 'agent-0' } });
const snapA = takeSnapshot(ident, 'node-a', nodeA.toJSON());
log('快照生成+签名', verifySnapshot(snapA).ok, `hash=${snapA.hash.slice(0, 12)}…`);

// 2) 备份到分布式存储
const store = new StateStore();
store.put('node-a', snapA);
log('备份入 StateStore', store.get('node-a') !== undefined, `bucket=[${store.list()}]`);

// 3) 节点 A 源丢失（模拟灾难）
log('节点 A 源丢失（灾难）', true, '本地状态不可访问');

// 4) 从存储恢复快照，重建到节点 B（2PC）
const snapRestored = store.get('node-a');
const check = verifySnapshot(snapRestored);
log('恢复快照校验', check.ok, check.ok ? 'Ed25519 验签通过' : check.reason);
const nodeB = new NodeState();
const mig = new Migrator({ source: { toJSON: () => snapRestored.data }, target: nodeB, identity: ident, publicKey: ident.keypair });
mig.prepare();
mig.commit();
mig.confirm();
log('2PC 迁移到节点 B', JSON.stringify(nodeB.toJSON()) === JSON.stringify(snapRestored.data), '状态与备份一致');

// 5) 从中断点继续（增量：step 7 → 8），diff 块经存储同步
const nodeB2 = new NodeState(nodeB.toJSON());
nodeB2.memory.set('step', 8);
const diff = diffState(nodeB.toJSON(), nodeB2.toJSON());
store.put('delta:node-b', { diff, baseHash: snapA.hash });
log('增量 diff 备份', diff.memory.set.step === 8, `只传输 ${JSON.stringify(diff.memory.set)}`);

// 6) 再次灾难 → 从基线+diff 重建
const rebuilt = applyDiff(store.get('node-a').data, store.get('delta:node-b').diff);
log('基线与 diff 重建', rebuilt.memory.step === 8 && rebuilt.files.config === '{"mode":"prod"}', '从中断点 step=8 继续');

console.log(process.exitCode ? '\n演练存在失败项' : '\n灾难恢复演练全部通过');
