# 中版本 v1.0 部署实测 — Autonomy 自治内核

> 收尾小版本：`v1.0.10`　｜　轨道：1.0（`au4a-kernel`）　｜　轮次：第 10 轮
> 实测平台：Windows / win32 x64；Rust rustc 1.98.1 (48a229cea 2026-09-01)

## 1. 部署（构建）

```
$ node tools/materialize.mjs --version v1.0.10 --out <mat>
materialized 267 files (from_snapshots=195, from_root=72)
$ cargo build --release -p au4a-node
exit=0  91.6s
```

## 2. 版本与自检

```
$ au4a-node version
au4a-node v1.0.10
$ au4a-node verify --json
exit=1
{
  "all_passed": false,
  "checks": [
    {
      "detail": "Autonomy 自治内核 v1.0.1 → v1.0.10 已接入 au4a-node",
      "name": "track.wired",
      "passed": true,
      "track": "1.0"
    },
    {
      "detail": "8 项宿主不变式全部成立（确定性种子引导）",
      "name": "host.audit",
      "passed": true,
      "track": "1.0"
    },
    {
      "detail": "两次独立引导得到同一注册表指纹（同种子 → 同结果）",
      "name": "registry.deterministic",
      "passed": true,
      "track": "1.0"
    },
    {
      "detail": "恰好 10 个小版本",
      "name": "manifest.version_count",
      "passed": true,
      "track": "1.0"
    },
    {
      "detail": "版本顺序与官方一致（v1.0.1 → v1.0.10）",
      "name": "manifest.order",
      "passed": true,
      "track": "1.0"
    },
    {
      "detail": "1.0 / Autonomy 自治内核 / v1.0.1 → v1.0.10 与 crate 常量一致",
      "name": "manifest.identity",
      "passed": true,
      "track": "1.0"
    },
    {
      "detail": "10 个版本的交付物/接口/验收/目标均非空且 status=done",
      "name": "manifest.fields",
      "passed": true,
      "track": "1.0"
    },
    {
      "detail": "证据等级合法、测试数自洽且命令指向本 crate；最后一版 139 个测试",
      "name": "manifest.evidence",
      "passed": true,
      "track": "1.0"
    },
    {
      "detail": "测试数单调不减（27 → 139）",
      "name": "manifest.test_growth",
      "passed": true,
      "track": "1.0"
    },
    {
      "detail": "清单 JSON 往返后完全相等",
      "name": "manifest.roundtrip",
      "passed": true,
      "track": "1.0"
    },
    {
      "detail": "Capability Graph 能力图 v1.1.1 → v1.1.10 已接入 au4a-node",
      "name": "track.wired",
      "passed": true,
      "track": "1.1"
    },
    {
      "detail": "规范 JSON 出现小数字符：{\"constraints\":{\"max_concurrency\":1,\"max_input_bytes\":1048576,\"min_deadline_ms\":1,\"regions\":[]},\"current_load_bp\":0,\"latency_p50_ms\":100,\"latency_p99_ms\":250,\"price_per_unit\":7,\"produced_formats\":[\"text/plain\"],\"reliability_bp\":9000,\"skill\":\"translate.en-zh\",\"supported_formats\":[\"text/plain\"],\"throughput_per_min\":60}",
      "name": "1.1.1.integer_metrics",
      "passed": false,
      "track": "1.1"
    },
    {
      "detail": "断言 p99<p50、load>10000bp、reliability>10000bp 三类不合法能力均返回 Err",
      "name": "1.1.1.validation_rejects",
      "passed": true,
      "track": "1.1"
    },
    {
      "detail": "断言 p50=100,p99=300,负载=5000bp 时加权延迟=200ms、加权可靠度=4500bp（整数运算）",
      "name": "1.1.1.load_weighted_math",
      "passed": true,
      "track": "1.1"
    },
    {
      "detail": "断言同能力同指纹、改价后指纹改变（SHA-256 内容寻址）",
      "name": "1.1.1.content_addressed",
      "passed": true,
      "track": "1.1"
    },
    {
      "detail": "断言产出∩接受={application/json} 可接续；产出∩接受=∅ 不可接续",
      "name": "1.1.1.format_handoff",
      "passed": true,
      "track": "1.1"
    },
    {
      "detail": "断言：替他人声明返回 InvalidSignature；自签声明验签通过",
      "name": "1.1.2.self_signed_only",
      "passed": true,
      "track": "1.1"
    },
    {
      "detail": "断言：改动签名后的声明被判 unauthorized，且邻居视图保持为空",
      "name": "1.1.2.tampering_is_unauthorized",
      "passed
```

## 3. 端到端运行（Agent 自主）

```
$ au4a-node run --agents 8 --json
exit=0
{
  "agents": 8,
  "deterministic_seeds": true,
  "ledger_minted": 44000,
  "ledger_slashed": 107,
  "ledger_total": 43893,
  "messages_delivered": 13,
  "observe": {
    "progress": {
      "agents": [
        {
          "available": 980,
          "did": "did:au4a:11dd95abef4bc373e38f2b7d5b2d1dd33e22c85454224c4ca4b452b63a343db0",
          "display": "agent-00",
          "evidence": "verified",
          "locked": 20,
          "skills": [
            "translate.en-zh"
          ],
          "stake": 20
        },
        {
          "available": 975,
          "did": "did:au4a:8377856a09f5f4a9924f949b18c30f16c31015901256bbfdfbf2138a1f48ff72",
          "display": "agent-01",
          "evidence": "verified",
          "locked": 25,
          "skills": [
            "sentiment.analyse"
          ],
          "stake": 25
        },
        {
          "available": 970,
          "did": "did:au4a:9eb756cedfd84beea3c05b81cf9a820fe8f95bb524d13768e10c96c5e5d81660",
          "display": "agent-02",
          "evidence": "verified",
          "locked": 30,
          "skills": [
            "summarise.zh"
          ],
          "stake": 30
        },
        {
          "available": 965,
          "did": "did:au4a:ddf0170e644b4604a5245252aa04bbb642fd44250dbe190560b235fdec176b8c",
          "display": "agent-03",
          "evidence": "verified",
          "locked": 35,
          "skills": [
            "code.review"
          ],
          "stake": 35
        },
        {
          "available": 960,
          "did": "did:au4a:950a37b564144561644854e1a1e8f1a56c29a999237e666409a9e0003a639088",
          "display": "agent-04",
          "evidence": "verified",
          "locked": 40,
          "skills": [
            "data.clean"
          ],
          "stake": 40
        },
        {
          "available": 955,
          "did": "did:au4a:d39371d1197e5945c442f6bd939f74787e6b9234f9a7fc55dd426497d7cd26ed",
          "display": "agent-05",
          "evidence": "verified",
          "locked": 45,
          "skills": [
            "image.tag"
          ],
          "stake": 45
        },
        {
          "available": 950,
          "did": "did:au4a:463fcb6eff5edb16a14526125f6e7d9909e0862e8095a85458842955a66043d5",
          "display": "agent-06",
          "evidence": "verified",
          "locked": 50,
          "skills": [
            "audio.transcribe"
          ],
          "stake": 50
        },
        {
          "available": 945,
          "did": "did:au4a:ff0adf0b446a75a221ee2627d63c9b6a6a77e96618213cd0adf120e34d2978d2",
          "display": "agent-07",
          "evidence": "verified",
          "locked": 55,
          "skills": [
            "plan.route"
          ],
          "stake": 55
        },
        {
          "available": 980,
          "did": "did:au4a:66be7e332c7a453332bd9d0a7f7db055f5c5ef1a06ada66d98b39fb6810c473a",
          "display": "alice",
          "evidence": "verified",
          "locked": 20,
          "skills": [],
     
```

## 4. 人类观察面板（只读）

| 端点 | 方法 | 状态码 | 字节 |
|---|---|---|---|
| `/` | GET | 200 | 1864 |
| `/api/progress` | GET | 200 | 164 |
| `/api/results` | GET | 200 | 100 |
| `/api/revenue` | GET | 200 | 135 |
| `/api/selfcheck` | GET | 200 | 24504 |
| `/api/tracks` | GET | 200 | 657 |
| `/api/revenue` | **POST** | 405（必须 405） | 172 |

只读性判定：**通过**（GET 面板可用且 POST 被 405 拒绝）。

## 5. 结论

- 部署：成功
- 自检：未通过（exit 1）
- 端到端：成功（exit 0）
- 只读观察面：通过

---

> 本文件由 `tools/verify-mediums.mjs` 生成；命令与输出为真实运行结果。
