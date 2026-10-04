# 多智能体网络的缩放定律度量框架（v1.9.8 论文稿）

> 项目：agent-universeForAngent · 文档代号：SCALING-PAPER
> 状态：已实现（度量模块 v1.9.0 + 集群模拟 v1.9.2 实测背书） · 版本：v1.9.8

## 摘要

Agent 网络的扩展能力不取决于"能并行放多少 Agent"，而取决于"编排、状态、验证与结算的开销如何随规模增长"。本文给出一个可复现的缩放度量框架：以**吞吐（throughput）**、**效率（efficiency）**、**成本效率（cost efficiency）**与**编排开销占比（overhead ratio）**四个可计算指标刻画网络行为，并以"完成率峰值 + 峰值后开销恶化"判定**饱和点（saturation）**。框架在 10/100/1k/10k 节点矩阵上实测，给出饱和判定的形式化定义与复现路径。

## 1. 问题

当 Agent 从"单个模型调用"变为"百万级协同网络"，瓶颈从 GPU 浮点运算转移到 CPU 编排与治理（任务分解、工具调用、权限检查、沙箱执行、RAG、记忆检索、验证与结算）。已有 Scaling Law 多聚焦模型参数与数据，缺少面向**多智能体协作网络**的度量框架：节点数、交互复杂度、算力预算如何决定群体智能水平，缺少可计算定义。

## 2. 核心概念

| 指标 | 定义 | 含义 |
|---|---|---|
| 吞吐 throughput | 单位时间完成的任务数 | 网络整体产出速率 |
| 效率 efficiency | completed / totalTasks | 任务完成比例 |
| 成本效率 costEfficiency | throughput / (完成单位任务耗时) | 每成本单元的产出 |
| 编排开销占比 overheadRatio | overheadMs / durationMs | 编排消耗相对预算的比例 |

编排开销模型：`overheadMs = nodes × overheadPerNodeMs × totalTasks`——编排成本随节点数与任务数线性累积，是"治理税"的物理化。

## 3. 饱和判定（Scaling Verdict）

```
sorted = samples 按 nodes 升序
peak = 效率最高样本
declining = 峰值后任一效率下降
plateauOverhead = 末样本效率==峰值 且 末样本开销占比>峰值开销占比
verdict = (末节点数 > 峰值节点数) && (declining || plateauOverhead) ? 'saturated' : 'scaling'
```

- **scaling**：加节点仍在提升效率或保持持平；
- **saturated**：加节点不再提升（或下降）完成率，且编排开销持续恶化——即"结构决定增长曲线形状"的物理表达。

## 4. 实验设计（可复现）

- 模型：`ExperimentRunner({ overheadPerNodeMs, capacityPerNode })`
- 参数：`overheadPerNodeMs=1, capacityPerNode=4, totalTasks=50000, durationMs=100`
- 矩阵：`nodes ∈ {10, 100, 1000, 10000}`
- 运行：`node tools/cluster-sim.js`（确定性，无随机源）

## 5. 实测结果（v1.9.2）

| 节点数 | 完成量 | 吞吐 | 效率 | 开销占比 |
|---|---|---|---|---|
| 10 | 40 | 400 | 0.1% | 500000% |
| 100 | 400 | 4000 | 0.8% | 5000000% |
| 1000 | 4000 | 40000 | 8.0% | 50000000% |
| 10000 | 40000 | 400000 | 80.0% | 500000000% |

判定：`SCALING`——完成量随节点数线性扩展，效率单调上升（容量未触顶）。

## 6. 讨论：与既有观察的对应

- **P0b 实验**（8 个智能体比 1 个还慢，0.695×）：CPU 密集任务在物理并行度附近见顶反降——本框架的 `saturated` 即该现象的参数化形式；
- **白皮书判断**（CPU 编排不足时 GPU 无法转化为端到端吞吐）：`overheadRatio` 随规模爆炸正是编排瓶颈的度量；
- 二者共同指向：**扩展瓶颈不是"能不能算出更多"，而是"能不能管好更多"。**

## 7. 局限与后续

- 当前为确定性模型，未接入真实分布式运行时（libp2p 集群）与真实延迟分布；
- 未建模通信拓扑（GossipSub fanout、DHT 路由跳数）对开销的影响；
- 后续 v1.9.x：接入真实集群 trace，用 `overheadPerNodeMs` 实测标定取代默认参数。

## 复现清单

```bash
npm install @twinsearth/agent-universe-for-angent
node tools/cluster-sim.js          # 缩放矩阵
node --test test/scale.test.js     # 度量单元测试
```
