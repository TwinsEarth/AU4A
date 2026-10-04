// v1.1.0 — 能力图测试
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Capability, CapabilityGraph } from '../src/capgraph/graph.js';

test('声明与查询：按 skill/格式/延迟/负载/价格过滤', () => {
  const g = new CapabilityGraph();
  g.declare('did:a', [
    new Capability({ skill: 'translate', latencyP50: 200, pricePerUnit: 5, supportedFormats: ['text'] }),
    new Capability({ skill: 'summarize', latencyP50: 300, pricePerUnit: 8, supportedFormats: ['text'] }),
  ]);
  g.declare('did:b', [
    new Capability({ skill: 'translate', latencyP50: 500, pricePerUnit: 3, supportedFormats: ['text'] }),
  ]);

  const fast = g.query('translate', { maxLatency: 300 });
  assert.equal(fast.length, 1);
  assert.equal(fast[0].did, 'did:a');

  const cheap = g.query('translate');
  assert.equal(cheap[0].did, 'did:b', '按价格排序应优先便宜者');

  const none = g.query('translate', { maxLoad: 0.5 });
  assert.ok(Array.isArray(none));
});

test('路径规划：多能力流水线 + 格式兼容', () => {
  const g = new CapabilityGraph();
  g.declare('translator', [
    new Capability({ skill: 'translate', latencyP50: 100, pricePerUnit: 4, supportedFormats: ['text'], reliability: 0.99 }),
  ]);
  g.declare('summarizer', [
    new Capability({ skill: 'summarize', latencyP50: 150, pricePerUnit: 6, supportedFormats: ['text'] }),
  ]);
  g.declare('loader', [
    new Capability({ skill: 'fetch', latencyP50: 80, pricePerUnit: 2, supportedFormats: ['url'] }),
  ]);

  const ok = g.route(['translate', 'summarize']);
  assert.equal(ok.compatible, true);
  assert.equal(ok.path.length, 2);
  assert.equal(ok.totalPrice, 10);

  const bad = g.route(['fetch', 'summarize']);
  assert.equal(bad.compatible, false, 'url 输出不能作为 summarize 输入（格式不兼容）');
  assert.equal(bad.missingAt, 'summarize');
});

test('能力变更版本自增 + 总线广播', () => {
  const seen = [];
  const g = new CapabilityGraph();
  g.subscribe('/capgraph/1.0.0', (p) => seen.push(p));
  g.declare('did:x', [{ skill: 'code', latencyP50: 10, pricePerUnit: 1 }]);
  const c = g.query('code')[0];
  const v0 = c.version;
  g.setLoad('did:x', 'code', 0.5);
  assert.equal(g.query('code')[0].currentLoad, 0.5);
  assert.ok(g.query('code')[0].version > v0, '负载变更应提升能力版本');
  assert.ok(seen.some((p) => p.type === 'LOAD'), '总线应收到负载广播');
});
