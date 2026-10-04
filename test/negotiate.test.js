// v1.2.0 — 协商协议测试
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Negotiation, NegotiationState } from '../src/negotiate/protocol.js';

test('多轮协商 → 合约 → 执行 → 结算（完整链路）', () => {
  const n = new Negotiation({ initiator: 'did:a', responder: 'did:b', goal: 'translate doc' });
  n.propose({ price: 10, terms: { deadline: 3600 } });
  n.counter({ price: 8, terms: { deadline: 3600 } });
  n.propose({ price: 9, terms: { deadline: 3600 } });
  const acc = n.accept('did:b');
  assert.equal(acc.price, 9, '应以最终出价成交');
  const contract = n.sign('did:b');
  assert.ok(contract.hash.startsWith('0') || /^[0-9a-f]{64}$/.test(contract.hash), '合约哈希应为 sha256 hex');
  n.execute();
  const settled = n.settle('did:a');
  assert.equal(settled.state, NegotiationState.SETTLED);
});

test('拒绝回到 IDLE；超轮数报错', () => {
  const n = new Negotiation({ initiator: 'did:a', responder: 'did:b', goal: 'g', maxRounds: 2 });
  n.propose({ price: 5 });
  assert.equal(n.reject('did:b', 'too expensive'), false);
  assert.equal(n.state, NegotiationState.IDLE);
  n.propose({ price: 5 });
  n.counter({ price: 4 });
  n.propose({ price: 4 });
  // maxRounds=2 → 每方至多 2 次报价（总 4 轮），第 5 轮触发限额
  assert.throws(() => n.counter({ price: 3 }), /轮数/);
  assert.throws(() => n.propose({ price: 3 }), /轮数/);
});

test('违约进入仲裁', () => {
  const n = new Negotiation({ initiator: 'did:a', responder: 'did:b', goal: 'g' });
  n.propose({ price: 5 });
  n.accept('did:b');
  n.sign('did:b');
  n.execute();
  const br = n.breach('did:a', '未能交付');
  assert.equal(br.arbitrationRequired, true);
  assert.equal(n.state, NegotiationState.BREACH);
  assert.throws(() => n.settle('did:b'), /非法状态转换/, '违约后不得结算');
});

test('持久化：toJSON/fromJSON 还原状态', () => {
  const n = new Negotiation({ initiator: 'did:a', responder: 'did:b', goal: 'g' });
  n.propose({ price: 7 });
  n.accept('did:b');
  n.sign('did:b');
  const restored = Negotiation.fromJSON(n.toJSON());
  assert.equal(restored.state, n.state);
  assert.equal(restored.contract.hash, n.contract.hash);
  assert.equal(restored.goal, 'g');
});
