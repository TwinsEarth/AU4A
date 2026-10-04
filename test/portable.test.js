// v1.3.0 — 可移植状态测试
import { test } from 'node:test';
import assert from 'node:assert/strict';
import au from '@twinsearth/agent-universe';
import { NodeState, Migrator, takeSnapshot, verifySnapshot } from '../src/state/portable.js';

const { Keypair } = au;

test('快照 → 验签 → 2PC 迁移：文件/内存/上下文完整恢复', () => {
  const kp = Keypair.generate();
  const src = new NodeState({
    files: { 'work.txt': 'hello' },
    memory: { step: 3 },
    context: { goal: 'translate' },
  });
  const dst = new NodeState({ files: { 'old.txt': 'x' } });

  const mig = new Migrator({ source: src, target: dst, identity: { keypair: kp } });
  const snap = mig.prepare();
  assert.ok(snap.hash.length === 64);
  assert.equal(verifySnapshot(snap).ok, true);
  mig.commit();
  mig.confirm();

  assert.equal(dst.files.get('work.txt'), 'hello', '文件恢复');
  assert.equal(dst.memory.get('step'), 3, '内存恢复');
  assert.equal(dst.context.goal, 'translate', '上下文恢复');
  assert.equal(dst.files.has('old.txt'), false, '迁移以快照为准：目标被完整替换（从中断点继续）');
});

test('篡改快照 → 校验失败', () => {
  const kp = Keypair.generate();
  const src = new NodeState({ memory: { x: 1 } });
  const snap = takeSnapshot({ keypair: kp }, 'n', src.toJSON());
  snap.data.memory.x = 999;
  assert.equal(verifySnapshot(snap).ok, false, '数据被篡改必须检出');
});

test('apply 失败 → rollback → 目标保持原状（无部分状态）', () => {
  const kp = Keypair.generate();
  const src = new NodeState({ files: { a: '1' } });
  const dst = new NodeState({ files: { keep: 'k' } });
  const mig = new Migrator({ source: src, target: dst, identity: { keypair: kp } });
  mig.prepare();

  // 破坏目标 apply：抛错模拟恢复失败
  dst.apply = () => { throw new Error('disk full'); };
  assert.throws(() => mig.commit(), /disk full/);
  assert.equal(dst.files.get('keep'), 'k', '回滚后目标保持原状');
  assert.equal(mig._committed, false);
});

test('未 prepare 直接 commit 报错；未 commit 直接 confirm 报错', () => {
  const kp = Keypair.generate();
  const src = new NodeState({ memory: { a: 1 } });
  const dst = new NodeState();
  const mig = new Migrator({ source: src, target: dst, identity: { keypair: kp } });
  assert.throws(() => mig.commit(), /未 prepare/);
  assert.throws(() => mig.confirm(), /未 commit/);
});
