// v1.3.5/v1.3.6 — 分布式存储与增量快照测试
import { test } from 'node:test';
import assert from 'node:assert/strict';
import au from '@twinsearth/agent-universe';
import { NodeState, takeSnapshot } from '../src/state/portable.js';
import { diffState, applyDiff } from '../src/state/portable.js';
import { StateStore } from '../src/state/store.js';

const { Keypair } = au;

test('v1.3.5 分布式存储：put/get/list/remove', () => {
  const store = new StateStore();
  const kp = Keypair.generate();
  const snap = takeSnapshot({ keypair: kp }, 'node-a', { files: { a: '1' }, memory: {}, context: {} });
  store.put('node-a', snap);
  assert.equal(store.get('node-a').hash, snap.hash);
  assert.deepEqual(store.list(), ['node-a']);
  store.remove('node-a');
  assert.equal(store.get('node-a'), undefined);
});

test('v1.3.6 增量快照：diff 只含变更块，applyDiff 往返一致', () => {
  const prev = { files: { a: '1', b: '2' }, memory: { x: 1 }, context: { goal: 'g' } };
  const next = { files: { a: '1', c: '3' }, memory: { x: 2 }, context: { goal: 'g' } };
  const diff = diffState(prev, next);
  assert.deepEqual(diff.files.set, { c: '3' }, '新增 c');
  assert.deepEqual(diff.files.del, ['b'], '删除 b');
  assert.deepEqual(diff.memory.set, { x: 2 });
  assert.deepEqual(diff.context.set, {}, 'context 未变');
  const merged = applyDiff(prev, diff);
  assert.deepEqual(merged, next, 'applyDiff 必须精确还原 next');
  assert.equal(prev.files.b, '2', '基线不得被修改');
});

test('v1.3.6 增量快照跨迁移：store 存 diff 块后可恢复完整状态', () => {
  const store = new StateStore();
  const prev = new NodeState({ files: { a: '1' }, memory: { step: 1 }, context: { goal: 'x' } }).toJSON();
  const next = new NodeState({ files: { a: '1', b: '2' }, memory: { step: 2 }, context: { goal: 'x' } }).toJSON();
  const diff = diffState(prev, next);
  store.put('delta:node-a', { diff, baseHash: 'prev' });
  const restored = applyDiff(prev, store.get('delta:node-a').diff);
  assert.deepEqual(restored, next);
});
