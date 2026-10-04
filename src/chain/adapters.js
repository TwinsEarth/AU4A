// v1.8.0 — 跨链结算（Cross-Chain Settlement）
// BTC（RGB/Taproot Assets）与 ETH（ERC-8004/x402）适配器 + 结算路由。
// 本版本为测试网适配器（确定性 mock，账本守恒）：主网接入仅需替换背书函数。
export class BtcAdapter {
  constructor({ network = 'testnet' } = {}) {
    this.network = network;
    this.txs = [];       // [{ txid, asset, amount, utxo, commitments }]
    this._seq = 0;
  }

  createAddress(did) {
    return `tb1q${Buffer.from(did).toString('hex').slice(0, 30)}${this._seq++}`.slice(0, 42);
  }

  /** RGB 承诺：锚定结算承诺到 BTC 交易（测试网确定性 txid） */
  commit(amount, asset = 'AUSAT', utxo = 'utxo:test:1') {
    const txid = `tx:${this.network}:${++this._seq}:${amount}:${asset}`;
    const commitment = Buffer.from(`RGB-COMMIT|${txid}|${asset}|${amount}`).toString('hex');
    this.txs.push({ txid, asset, amount, utxo, commitment, status: 'committed' });
    return { txid, commitment };
  }

  verifyCommitment(commitment) {
    const found = this.txs.find((t) => t.commitment === commitment);
    return found ? { ok: true, txid: found.txid, asset: found.asset, amount: found.amount } : { ok: false };
  }

  /** Taproot Assets 转移完成（结算付款） */
  finalize(txid) {
    const t = this.txs.find((x) => x.txid === txid);
    if (!t) throw new Error('交易不存在');
    t.status = 'finalized';
    return { txid, status: t.status };
  }
}

export class EthAdapter {
  constructor({ network = 'sepolia' } = {}) {
    this.network = network;
    this.invoices = new Map();
    this.payments = [];
    this._seq = 0;
  }

  /** x402 发票：`Authorization: 402 <token>` 支付门 */
  createInvoice({ amount, requester, purpose = '' }) {
    const id = `inv:${this.network}:${++this._seq}`;
    const invoice = { id, amount, requester, purpose, status: 'open', token: `402-${this._seq}${Date.now().toString(36)}` };
    this.invoices.set(id, invoice);
    return invoice;
  }

  /** ERC-8004 支付：扣余额、记 payment */
  pay(invoiceId, from, wei) {
    const inv = this.invoices.get(invoiceId);
    if (!inv || inv.status !== 'open') throw new Error('发票不存在或已支付');
    if (wei < inv.amount) throw new Error('支付不足');
    inv.status = 'paid';
    inv.paidBy = from;
    this.payments.push({ invoiceId, from, wei, at: Date.now() });
    return inv;
  }

  claim(invoiceId, to) {
    const inv = this.invoices.get(invoiceId);
    if (!inv || inv.status !== 'paid') throw new Error('未支付不可领取');
    inv.status = 'claimed';
    inv.claimedBy = to;
    return inv;
  }
}

/** 结算路由：积分 → 链上资产，账本守恒（积分减少 == 链上资产增加） */
export class SettlementRouter {
  constructor({ btc = null, eth = null, exchangeRate = 1000 } = {}) {
    this.btc = btc || new BtcAdapter();
    this.eth = eth || new EthAdapter();
    this.exchangeRate = exchangeRate; // 1 积分 → N 最小链上单位
    this.ledger = { credits: 0, btcUnits: 0, ethUnits: 0 };
    this.reputationBridge = [];       // 跨链信誉桥接记录
  }

  exchange({ did, credits, track = 'btc', asset = 'AUSAT' }) {
    if (credits <= 0) throw new Error('兑换金额必须为正');
    const units = credits * this.exchangeRate;
    if (track === 'btc') {
      const r = this.btc.commit(units, asset);
      this.ledger.credits += credits;
      this.ledger.btcUnits += units;
      return { track, credits, units, txid: r.txid, commitment: r.commitment };
    }
    const inv = this.eth.createInvoice({ amount: units, requester: did, purpose: 'agent-settlement' });
    this.eth.pay(inv.id, did, units);
    this.ledger.credits += credits;
    this.ledger.ethUnits += units;
    return { track, credits, units, invoiceId: inv.id };
  }

  /** 跨链信誉桥接（ReputationBridge.sol 接口） */
  bridgeReputation({ fromChain, did, score }) {
    const rec = { fromChain, did, score, at: Date.now() };
    this.reputationBridge.push(rec);
    return rec;
  }

  conservation() {
    // 转换守恒：credits × rate === btcUnits + ethUnits（未考虑链上手续费时恒成立）
    return { conserved: this.ledger.credits * this.exchangeRate === this.ledger.btcUnits + this.ledger.ethUnits, ...this.ledger };
  }
}
