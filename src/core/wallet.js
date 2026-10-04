// v1.0.1 — Agent 钱包（Wallet）
// 对接参考项目 @twinsearth/agent-universe 的 AgentMarket 账户系统，
// 金额一律为安全整数（与 Rust Money(i64) 对齐），杜绝浮点守恒失效。
export class Wallet {
  constructor(market) {
    this.market = market;
  }

  balance(account) {
    return this.market.balance(account);
  }

  deposit(account, amount) {
    this.market.deposit(account, amount);
    return this.balance(account);
  }

  /** 系统总余额（充值 − 罚没 = 当前全部账户之和，守恒） */
  conservation() {
    return this.market.conservationCheck(this.market.totalDeposits);
  }

  /** 独立审计：只信任结算流水逐笔重放（可发现守恒检查察觉不到的账实不符） */
  audit() {
    return this.market.independentAudit();
  }
}
