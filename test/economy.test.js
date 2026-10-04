// v1.4.0 — 经济自主测试
import { test } from 'node:test';
import assert from 'node:assert/strict';
import au from '@twinsearth/agent-universe';
import { PricingStrategy, AgentEconomy, ExchangeRouter } from '../src/economy/pricing.js';

test('自主定价：信誉↑/负载↑/稀缺↑ → 价格↑', () => {
  const s = new PricingStrategy({ basePrice: 10 });
  const low = s.price({ load: 0.1, providers: 5, reputation: 0.2 });
  const high = s.price({ load: 0.9, providers: 1, reputation: 0.95 });
  assert.ok(high > low, `高负载+稀缺+高信誉应显著更贵：${low} → ${high}`);
  assert.ok(low >= 1);
});

test('Agent 自治经济体：发布→投标→匹配→结算→守恒+独立审计', () => {
  const market = new au.AgentMarket();
  const econ = new AgentEconomy(market);
  const worker = 'did:au:worker';
  const requester = 'did:au:requester';

  econ.fund(worker, 200);
  econ.fund(requester, 150);
  econ.stakeRegister(au.AgentCard.new({ did: worker, name: 'Worker' }), 100);
  econ.publish('t1', 'translate doc', 50, requester);
  econ.bid('t1', worker, 40);
  const winner = market.matchTask('t1');
  assert.equal(winner.agentId, worker);
  market.completeTask('t1', true, 'Verified');
  const settle = econ.settle('t1');
  assert.equal(settle.reason, 'settled');
  assert.equal(settle.paid, 40);

  const integ = econ.integrity();
  assert.equal(integ.conservation.conserved, true, '守恒必须成立');
  assert.equal(integ.audit.passed, true, '独立审计必须通过');
  assert.equal(market.balance(worker), 200 - 100 + 40, 'Worker 余额 = 充值−质押+报酬');
});

test('兑换路由：小额聚合；高时效走 BTC；常规走 ETH', () => {
  const r = new ExchangeRouter();
  assert.equal(r.route({ amount: 50 }).track, 'hold');
  assert.equal(r.route({ amount: 500, urgency: 0.9 }).track, 'btc');
  assert.equal(r.route({ amount: 500, urgency: 0.2 }).track, 'eth');
  assert.equal(r.route({ amount: 500, urgency: 0.2, ethGasPremium: 2.0 }).track, 'btc');
});
