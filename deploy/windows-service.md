# AU4A — Windows 10 服务化部署（v2.0.2）

> AU4A 运行期**不写盘**（零文件 I/O 是设计约束），因此在 Windows 上不需要数据目录、
> 不需要注册表配置，只需要「一个二进制 + 一个服务」。下面给两种方案，任选其一。

## 0. 前置

```powershell
# 1) 安装 Rust（若尚未安装）
winget install --id Rustlang.Rustup -e
# 2) 取代码并构建（以发布版本为准）
git clone https://github.com/TwinsEarth/AU4A.git; cd AU4A
git checkout v2.0.2
cargo build --release --locked -p au4a-node
# 3) 部署前自检：必须 exit 0（PowerShell 里 $LASTEXITCODE 应为 0）
.\target\release\au4a-node.exe verify
```

把 `target\release\au4a-node.exe` 复制到固定位置：

```powershell
New-Item -ItemType Directory -Force C:\AU4A | Out-Null
Copy-Item .\target\release\au4a-node.exe C:\AU4A\au4a-node.exe -Force
```

防火墙：只读面板默认只绑 `127.0.0.1`。若要对内网开放，显式放行并**只允许 GET 语义**由面板自身保证（非 GET 一律 405）：

```powershell
New-NetFirewallRule -DisplayName "AU4A observer (8787)" -Direction Inbound `
  -Protocol TCP -LocalPort 8787 -Action Allow -Profile Private
```

## 方案 A：内置 `sc.exe`（无需第三方）

```powershell
# 创建服务（注意：sc 要求等号后有空格，路径带空格时必须整段加引号）
sc.exe create AU4A binPath= "C:\AU4A\au4a-node.exe run --agents 8 --observe 127.0.0.1:8787" start= auto
sc.exe description AU4A "AU4A (Agents-UniverseForAgent) — agent-autonomous runtime with read-only observer panel"
sc.exe failure AU4A reset= 86400 actions= restart/3000/restart/3000/restart/3000

# 先手动验证一次（前台跑，确认面板可用）
C:\AU4A\au4a-node.exe run --agents 8 --observe 127.0.0.1:8787
# 另开一个 PowerShell：
curl.exe -s -o NUL -w "%{http_code}`n" http://127.0.0.1:8787/api/progress     # 期望 200
curl.exe -s -o NUL -w "%{http_code}`n" -X POST http://127.0.0.1:8787/api/revenue # 期望 405

# 启动服务
Start-Service AU4A
Get-Service AU4A
```

**限制**：`sc.exe` 把服务当作普通进程，**不会在启动前跑 `verify`**，也不做崩溃自愈之外的资源限制。
若需要「自检不通过就不上线」，用方案 B 或在包装脚本里先跑 `verify`。

## 方案 B：NSSM（推荐，支持启动前自检与 stdout 重定向）

```powershell
winget install --id NSSM.NSSM -e            # 或从 nssm.cc 下载
nssm install AU4A "C:\AU4A\au4a-node.exe" "run --agents 8 --observe 127.0.0.1:8787"
nssm set AU4A AppDirectory "C:\AU4A"
nssm set AU4A Start SERVICE_AUTO_START
# 崩溃/异常退出自动重启（延迟 3 秒），并把日志写到文件
nssm set AU4A AppExit Default Restart
nssm set AU4A AppRestartDelay 3000
nssm set AU4A AppStdout "C:\AU4A\au4a-node.out.log"
nssm set AU4A AppStderr "C:\AU4A\au4a-node.err.log"
nssm set AU4A AppRotateFiles 1
nssm set AU4A AppRotateBytes 10485760
Start-Service AU4A
```

**启动前自检**（把 `verify` 作为上线闸门）：

```powershell
# 用一个包装脚本做门禁，再交给 NSSM 托管
@'
$ErrorActionPreference = "Continue"
& "C:\AU4A\au4a-node.exe" verify
if ($LASTEXITCODE -ne 0) { Write-EventLog -LogName Application -Source AU4A -EventId 1 -EntryType Error -Message "verify failed; refusing to start"; exit 1 }
& "C:\AU4A\au4a-node.exe" run --agents 8 --observe 127.0.0.1:8787
'@ | Set-Content C:\AU4A\start-au4a.ps1 -Encoding UTF8
nssm set AU4A Application "C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
nssm set AU4A AppParameters "-NoProfile -ExecutionPolicy Bypass -File C:\AU4A\start-au4a.ps1"
```

## 运维

```powershell
Restart-Service AU4A          # 重启（无状态，重启即回到初始 Agent 网络）
Stop-Service AU4A             # 停止
Get-Content C:\AU4A\au4a-node.err.log -Tail 50      # 方案 B 的日志
sc.exe delete AU4A            # 卸载（方案 A）
nssm remove AU4A confirm      # 卸载（方案 B）
```

## 回滚（Windows）

```powershell
Stop-Service AU4A
Copy-Item C:\AU4A\au4a-node.exe C:\AU4A\au4a-node.exe.bak -Force
# 取上一个版本重新构建，或从 GitHub Release 下载对应版本的二进制
git checkout v1.9.9; cargo build --release --locked -p au4a-node
Copy-Item .\target\release\au4a-node.exe C:\AU4A\au4a-node.exe -Force
& C:\AU4A\au4a-node.exe verify      # 必须 exit 0
Start-Service AU4A
```

## 验收（与本仓库 `docs/ACCEPTANCE.md` 的 B 组一致）

```powershell
C:\AU4A\au4a-node.exe version                                   # au4a-node v2.0.2
C:\AU4A\au4a-node.exe verify; $LASTEXITCODE                      # 0
foreach ($r in @('/','/api/progress','/api/results','/api/revenue','/api/selfcheck','/api/tracks')) {
  curl.exe -s -o NUL -w "$r %{http_code}`n" "http://127.0.0.1:8787$r" }   # 6 行全 200
curl.exe -s -o NUL -w "%{http_code}`n" -X POST http://127.0.0.1:8787/api/revenue  # 405
```
