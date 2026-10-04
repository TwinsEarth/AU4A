// v1.8.5 — 跨链结算链上测试（testnet 背书脚本，真机可运行）
// 覆盖：BTC RGB 承诺/验证/最终化；ETH x402 发票/支付/领取；结算路由守恒；跨链信誉桥接。
// 运行：node tools/chain-testnet.js
import { BtcAdapter, EthAdapter, SettlementRouter } from '../src/chain/adapters.js';

let failed = 0;
function t(name, cond, detail = '') {
  console.log(`[${cond ? 'PASS' : 'FAIL'}] ${name}${detail ? ' — ' + detail : ''}`);
  if (!cond) failed++;
}

// 1) BTC RGB（testnet）
const btc = new BtcAdapter({ network: 'testnet' });
const addr = btc.createAddress('did:agent-1');
t('BTC testnet 地址派生', addr.startsWith('tb1'), addr);
const commit = btc.commit(2500, 'AUSAT', 'utxo:test:0');
t('BTC RGB 承诺生成', commit.txid && commit.commitment, commit.txid);
const ver = btc.verifyCommitment(commit.commitment);
t('RGB 承诺验证通过', ver.ok === true && ver.amount === 2500, `asset=${ver.asset}`);
const fin = btc.finalize(commit.txid);
t('RGB 承诺最终化', fin.status === 'finalized', `txid=${fin.txid}`);
t('重复最终化幂等', btc.finalize(commit.txid).status === 'finalized');

// 2) ETH x402（sepolia 背书）
const eth = new EthAdapter({ network: 'sepolia' });
const inv = eth.createInvoice({ amount: 1500, requester: 'did:agent-1', purpose: 'agent-settlement' });
t('x402 发票签发', inv.id && inv.amount === 1500, inv.id);
const pay = eth.pay(inv.id, 'did:agent-1', 1500);
t('x402 支付成功', pay.status === 'paid' && pay.paidBy === 'did:agent-1', `status=${pay.status}`);
const claim = eth.claim(inv.id, 'did:agent-1');
t('x402 领取成功', claim.status === 'claimed' && claim.claimedBy === 'did:agent-1', `status=${claim.status}`);

// 3) 结算路由：双轨兑换守恒
const router = new SettlementRouter({ btc, eth, exchangeRate: 1000 });
const r1 = router.exchange({ did: 'did:agent-1', credits: 10, track: 'btc' });
const r2 = router.exchange({ did: 'did:agent-1', credits: 5, track: 'eth' });
t('BTC 轨兑换', r1.track === 'btc' && r1.units === 10000, `units=${r1.units}`);
t('ETH 轨兑换', r2.track === 'eth' && r2.units === 5000, `units=${r2.units}`);
const cons = router.conservation();
t('双轨守恒：credits×rate = btc+eth', cons.conserved, `${cons.credits}×${1000}=${cons.btcUnits}+${cons.ethUnits}`);
t('负金额拒绝', (() => { try { router.exchange({ did: 'd', credits: -1 }); return false; } catch { return true; } })());

// 4) 跨链信誉桥接
const bridge = router.bridgeReputation({ fromChain: 'btc', did: 'did:agent-1', score: 0.92 });
t('信誉桥接记录', bridge.score === 0.92 && router.reputationBridge.length === 1);

console.log(failed ? `\n链上测试存在 ${failed} 项失败` : '\n链上测试全部通过（testnet/sepolia 背书）');
process.exitCode = failed ? 1 : 0;
