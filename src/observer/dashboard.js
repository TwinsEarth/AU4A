// v1.0.1 — 人类观察层（HumanObserver）
// 人类角色被严格限定为观察者：只读展示【进度】【结果】【收益】。
// 本模块不提供任何写方法；否决权在 v1.7 委员会治理中按“仅极端情况”接入。
export class HumanObserver {
  constructor(market, registry) {
    this.market = market;
    this.registry = registry;
  }

  /** 只读快照：Agent 进度 + 任务结果 + 经济收益 + 守恒/审计 */
  snapshot() {
    const agents = this.registry.list().map((a) => ({
      did: a.ident.did,
      name: a.ident.name,
      status: a.status,
      skills: a.card.skills || [],
      capabilities: a.card.capabilities || [],
      balance: this.market.balance(a.ident.did),
    }));
    const tasks = [...this.market.tasks.entries()].map(([id, t]) => ({
      id,
      goal: t.goal,
      status: t.status,
      budget: t.budget,
      requester: t.requester,
      assignee: t.assignee ?? null,
      winnerPrice: t.winnerPrice ?? null,
    }));
    return {
      observedAt: Date.now(),
      progress: { agents },
      results: { tasks },
      earnings: {
        leaderboard: this.market.leaderboard(10),
        totalDeposits: this.market.totalDeposits,
        totalSlashed: this.market.totalSlashed,
        settlementRecords: this.market.records.length,
      },
      integrity: {
        conservation: this.market.conservationCheck(this.market.totalDeposits),
        audit: this.market.independentAudit(),
      },
    };
  }

  /** 一屏摘要（展示给人类的最小信息面） */
  summary() {
    const s = this.snapshot();
    return {
      activeAgents: s.progress.agents.length,
      openTasks: s.results.tasks.filter((t) => String(t.status).includes('OPEN')).length,
      conserved: s.integrity.conservation.conserved,
      auditPassed: s.integrity.audit.passed,
      leaderboard: s.earnings.leaderboard,
    };
  }
}
