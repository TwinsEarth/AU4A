// v1.8.0 — 跨链结算测试
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { BtcAdapter, EthAdapter, SettlementRouter } from '../src/chain/adapters.js';

test('BTC 适配器：RGB 承诺 → 验证 → 最终化', () => {
  const btc = new BtcAdapter({ network: 'testnet' });
  const c = btc.commit(5000, 'AUSAT');
  assert.ok(c.txid.startsWith('tx:testnet:'));
  assert.ok(btc.verifyCommitment(c.commitment).ok);
  const fin = btc.finalize(c.txid);
  assert.equal(fin.status, 'finalized');
  assert.equal(btc.verifyCommitment('tampered').ok, false, '伪造承诺必须校验失败');
});

test('ETH 适配器：x402 发票 → 支付 → 领取', () => {
  const eth = new EthAdapter({ network: 'sepolia' });
  const inv = eth.createInvoice({ amount: 300, requester: 'did:x' });
  assert.equal(inv.status, 'open');
  assert.throws(() => eth.claim(inv.id, 'did:y'), /未支付不可领取/);
  eth.pay(inv.id, 'did:x', 300);
  eth.claim(inv.id, 'did:y');
  assert.equal(inv.status, 'claimed');
});

test('结算路由：积分→BTC/ETH 账本守恒 + 信誉桥接', () => {
  const router = new SettlementRouter({ exchangeRate: 1000 });
  router.exchange({ did: 'did:a', credits: 50, track: 'btc' });
  router.exchange({ did: 'did:b', credits: 30, track: 'eth' });

  assert.equal(router.ledger.credits, 80);
  assert.equal(router.ledger.btcUnits, 50000);
  assert.equal(router.ledger.ethUnits, 30000);
  assert.equal(router.conservation().conserved, true, '积分×汇率 = BTC+ETH，守恒');

  router.bridgeReputation({ fromChain: 'btc', did: 'did:a', score: 0.92 });
  assert.equal(router.reputationBridge.length, 1);
  assert.throws(() => router.exchange({ did: 'x', credits: 0 }), /必须为正/);
});
