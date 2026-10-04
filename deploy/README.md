# 部署与验证（deploy）

本目录是「人类能自己跑一遍」的最短路径。真正的逐中版本证据由 `tools/verify-mediums.mjs` 生成，
这里提供单机一键脚本。

## 一键

```powershell
pwsh -File deploy/verify.ps1            # 构建 + 自检 + 端到端 + 只读面板探针
pwsh -File deploy/verify.ps1 -Agents 16 -Port 8899
```

脚本做四件事，任何一步失败即以非 0 退出：

1. `cargo build --release -p au4a-node`
2. `au4a-node verify --json` —— 聚合 10 条轨道自检，全绿才继续
3. `au4a-node run --agents N --json` —— Agent 自主跑完整链路，输出 JSON 摘要
4. 起 `--observe` 面板，用 HTTP GET 命中三个面板与自检，再用 **POST** 验证写路径不存在（必须 405）

## 观察（人类唯一入口）

```powershell
cargo run --release -p au4a-node -- run --agents 8 --observe 127.0.0.1:8787
# 浏览器打开 http://127.0.0.1:8787/
```

| 端点 | 内容 |
|---|---|
| `GET /` | HTML 三面板（进度 / 结果 / 收益），每 2 秒自动刷新 |
| `GET /api/progress` | 活跃 Agent、已投递消息、拒绝记录、进度事件 |
| `GET /api/results` | 各轨道产物摘要 |
| `GET /api/revenue` | 账本总量、发行、罚没、各账户余额 |
| `GET /api/selfcheck` | 10 条轨道自检明细 |
| `GET /api/tracks` | 轨道清单（轨道号 / crate / 标题 / 版本区间） |

非 GET 一律 **405**；未知路径 **404**。写路由不存在，而不是「被禁用」。

## 环境要求与两个已修过的坑

1. **`verify.ps1` 必须保存为 UTF-8 with BOM**。Windows PowerShell 5.1（系统默认的 `powershell.exe`）
   在没有 BOM 时会按 ANSI 代码页读取文件，中文注释会被解码成乱码并导致**语法错误**
   （`Unexpected token ')' in expression or statement`）。用编辑器保存时请保留 BOM；
   若文件被工具重写导致 BOM 丢失，可这样补回：

   ```powershell
   $p = 'deploy\verify.ps1'; $b = [IO.File]::ReadAllBytes($p)
   $o = [Collections.Generic.List[byte]]::new(); $o.AddRange([byte[]](0xEF,0xBB,0xBF)); $o.AddRange($b)
   [IO.File]::WriteAllBytes($p, $o.ToArray())
   ```

2. **脚本内 `$ErrorActionPreference` 是 `'Continue'`，不是 `'Stop'`**。PS 5.1 会把原生程序写到 stderr 的
   内容当成终止性错误，而 cargo 正常编译的进度输出恰好走 stderr —— 用 `'Stop'` 会在第一步误报
   `build failed`。因此脚本改为对每个原生命令显式检查 `$LASTEXITCODE`。

3. 若系统禁止运行脚本，直接放行本次调用即可（不改全局策略）：

   ```powershell
   powershell.exe -NoProfile -ExecutionPolicy Bypass -File deploy\verify.ps1 -Agents 8 -Port 8787
   ```

实测（本机 Windows，`v1.9.9` 树）：

```text
[1/4] cargo build --release -p au4a-node
[2/4] au4a-node verify
[3/4] au4a-node run --agents 8
      agents=8 tracks_ok=10/10 ledger_total=47878
[4/4] read-only observer probe on 127.0.0.1:8787
      GET /                  -> 200 (2131 bytes)
      GET /api/progress      -> 200 (162 bytes)
      GET /api/results       -> 200 (98 bytes)
      GET /api/revenue       -> 200 (133 bytes)
      GET /api/selfcheck     -> 200 (35957 bytes)
      GET /api/tracks        -> 200 (753 bytes)
      POST /api/revenue  -> 405 (expect 405)
RESULT: PASS — 构建 / 自检 / 端到端 / 只读面板四项全部通过
```

## 10 个中版本的分步部署

```bash
node tools/materialize.mjs --list                       # 看逐版本快照覆盖情况
node tools/verify-mediums.mjs                           # 重建每个中版本收尾点的树 → 构建 → 实测
node tools/verify-mediums.mjs --only v1.3.10            # 只测一个
```

产物：`docs/verify/medium-vX.Y.md`（每个中版本一份，含命令、退出码、输出、面板状态码）与
`docs/VERIFICATION.md`（汇总 + 未验证项清单）。
