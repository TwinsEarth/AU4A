// v1.6.9 — 学习效果评估 CLI（真机可运行）
// 运行：node tools/learn-eval.js [rounds]
import { runEvaluation } from '../src/learning/eval.js';

const rounds = Number(process.argv[2]) || 200;
const r = runEvaluation({ rounds, seed: 42 });

const pct = (x) => (x * 100).toFixed(1) + '%';
console.log(`=== 学习效果评估（rounds=${r.rounds}, seed=${r.seed}）===`);
console.log(`任务类型      : ${r.taskTypes.join(', ')}`);
console.log(`对照组成功率  : ${pct(r.baseline.successRate)}  平均收益 ${r.baseline.avgReward.toFixed(1)}`);
console.log(`实验组成功率  : ${pct(r.learned.successRate)}  平均收益 ${r.learned.avgReward.toFixed(1)}`);
console.log(`成功率提升    : ${(r.delta.successRate * 100).toFixed(1)} 个百分点`);
console.log(`平均收益提升  : ${r.delta.avgReward.toFixed(2)} 积分/任务`);
console.log(`结论          : ${r.verdict === 'learning-effective' ? '个体学习有效（可测量提升）' : '学习效果不显著'}`);
if (r.verdict !== 'learning-effective') process.exitCode = 1;
