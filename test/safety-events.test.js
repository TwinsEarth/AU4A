// v1.5.4/v1.5.5 — 安全事件通知测试
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { PermissionPolicy, SafetyAPI, SafetyStatus } from '../src/safety/api.js';
import { SafetyEvents, attachEvents } from '../src/safety/events.js';

const policy = () => new PermissionPolicy([
  { action: 'sandbox.exec', allow: ['did:trusted', '*'], deny: ['did:evil'] },
]);

test('v1.5.4 通知：REPORTED/APPEALED/ARBITRATED 事件按案件送达', () => {
  const api = new SafetyAPI(policy());
  const events = new SafetyEvents();
  attachEvents(api, events);
  const got = [];
  const off = events.on('safety-1', 'REPORTED', (p) => got.push(['r', p.event, p.target]));
  events.on('safety-1', 'ARBITRATED', (p) => got.push(['a', p.decision]));
  const c = api.report('did:a', 'did:b', '违规调用');
  assert.equal(c.id, 'safety-1');
  assert.deepEqual(got, [['r', 'REPORTED', 'did:b']]);
  api.arbitrate('safety-1', 'guilty', { slashed: 50 });
  assert.deepEqual(got.at(-1), ['a', 'guilty']);
  off(); // 退订后不再收到
  const c2 = api.report('did:a', 'did:c', 'x');
  assert.equal(got.length, 2, '退订后同案件不再通知');
  assert.ok(c2.id === 'safety-2');
});

test('v1.5.5 总线扩展：事件发布到外部总线 /safety/1.0.0', () => {
  const topics = [];
  const bus = { publish: (t, p) => topics.push([t, p.event, p.target]) };
  const api = new SafetyAPI(policy());
  const events = new SafetyEvents(bus);
  attachEvents(api, events);
  api.report('did:x', 'did:y', '越权');
  api.appeal('did:y', 'safety-1', '证据');
  assert.deepEqual(topics, [
    ['/safety/1.0.0', 'REPORTED', 'did:y'],
    ['/safety/1.0.0', 'APPEALED', 'did:y'],
  ]);
});
