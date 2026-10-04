// v1.5.4/v1.5.5 — 安全事件通知与总线扩展
// SafetyEvents：按案件订阅事件（REPORTED/APPEALED/ARBITRATED/…）；
// SafetyAPI 集成后：任何状态变更即通知订阅者，并可发布到外部事件总线（PMB 旁路）。
import { SafetyStatus } from './api.js';

export class SafetyEvents {
  constructor(bus = null) {
    this.bus = bus;           // 可选 PMB：{ publish(topic, payload) }
    this._subs = new Map();   // topic → Set<fn>
  }

  _emit(topic, payload) {
    if (this.bus && typeof this.bus.publish === 'function') this.bus.publish(topic, payload);
    const subs = this._subs.get(topic);
    if (subs) for (const fn of subs) fn(payload);
  }

  /** 订阅某案件事件：on(caseId, 'REPORTED', fn)；不关心具体案件可用 '*' */
  on(caseId, event, fn) {
    const topic = `${caseId}:${event}`;
    if (!this._subs.has(topic)) this._subs.set(topic, new Set());
    this._subs.get(topic).add(fn);
    return () => this._subs.get(topic).delete(fn);
  }

  publish(caseId, event, payload) {
    const body = { caseId, event, ...payload };
    // 总线走规范主题 /safety/1.0.0（人类观察层/外部订阅者旁路）
    if (this.bus && typeof this.bus.publish === 'function') this.bus.publish('/safety/1.0.0', body);
    // 本地订阅走案件级主题
    const topic = `${caseId}:${event}`;
    const subs = this._subs.get(topic);
    if (subs) for (const fn of subs) fn(body);
  }
}

/** 注入 SafetyAPI：状态变更 → 通知订阅者 + 总线广播 /safety/1.0.0 */
export function attachEvents(api, events) {
  const emit = (c, event) => events.publish(c.id, event, {
    reporter: c.reporter, target: c.target, reason: c.reason, decision: c.decision, at: c.at,
  });
  const origReport = api.report.bind(api);
  const origAppeal = api.appeal.bind(api);
  const origArbitrate = api.arbitrate.bind(api);
  api.report = (...args) => { const c = origReport(...args); emit(c, SafetyStatus.REPORTED); return c; };
  api.appeal = (...args) => { const c = origAppeal(...args); emit(c, SafetyStatus.APPEALED); return c; };
  api.arbitrate = (...args) => { const c = origArbitrate(...args); emit(c, SafetyStatus.ARBITRATED); return c; };
  return api;
}
