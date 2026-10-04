// v1.1.6/v1.1.7 — 能力图缓存与索引测试
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Capability, CapabilityGraph } from '../src/capgraph/graph.js';
import { GraphCache } from '../src/capgraph/cache.js';

test('v1.1.6 缓存：命中/未命中/重写刷新/容量驱逐', () => {
  const c = new GraphCache({ maxEntries: 2 });
  c.set('a', 'translate', [{ p: 1 }]);
  c.set('b', 'translate', [{ p: 2 }]);
  assert.equal(c.size(), 2);
  assert.ok(c.get('a', 'translate'), '命中');
  // 命中 a 后 a 变最新；再塞 c 驱逐最旧（b）
  c.set('c', 'code', [{ p: 3 }]);
  assert.equal(c.size(), 2);
  assert.ok(c.get('a', 'translate'), 'a 应仍在（被命中提升）');
  assert.equal(c.get('b', 'translate'), undefined, 'b 应被驱逐');
  assert.equal(c.get('missing', 'x'), undefined);
});

test('v1.1.6 缓存失效：按 Agent 全量失效', () => {
  const c = new GraphCache();
  c.set('a', 't1', [1]);
  c.set('a', 't2', [2]);
  c.set('b', 't1', [3]);
  assert.equal(c.invalidate('a'), 2);
  assert.equal(c.size(), 1);
  assert.equal(c.get('b', 't1')[0], 3);
});

test('v1.1.7 索引：查询只命中该 skill 的 Agent；声明覆盖后索引同步', () => {
  const g = new CapabilityGraph();
  g.declare('a', [new Capability({ skill: 'translate', pricePerUnit: 5 }), new Capability({ skill: 'code', pricePerUnit: 9 })]);
  g.declare('b', [new Capability({ skill: 'translate', pricePerUnit: 3 })]);
  assert.equal(g.query('translate').length, 2);
  assert.equal(g.query('code').length, 1);
  // a 重新声明时去掉 code → 索引应同步摘除
  g.declare('a', [new Capability({ skill: 'translate', pricePerUnit: 5 })]);
  assert.equal(g.query('code').length, 0, '索引必须与声明同步');
  assert.equal(g.query('translate').length, 2);
  assert.equal(g.query('none').length, 0);
});
