# deploy/verify.ps1 — 一键部署验证：构建 → 自检 → 端到端 → 只读面板探针。
#
# 任何一步失败即以非 0 退出；所有输出同时打印并写入 _forangent/logs/deploy-verify.log。
param(
    [int]$Agents = 8,
    [int]$Port = 8787,
    [string]$Configuration = 'release'
)

# 注意（Windows PowerShell 5.1）：原生程序写到 stderr 的内容在 $ErrorActionPreference='Stop' 下
# 会被当成终止性错误——而 cargo 正常编译的进度输出恰好走 stderr，脚本就会在第一步误报失败。
# 因此这里用 'Continue'，改为对每个原生命令显式检查 $LASTEXITCODE（下面每步都有）。
$ErrorActionPreference = 'Continue'
$repo = Split-Path -Parent $PSScriptRoot
$logDir = Join-Path (Split-Path -Parent $repo) '_forangent\logs'
New-Item -ItemType Directory -Force -Path $logDir | Out-Null
$log = Join-Path $logDir 'deploy-verify.log'
function Say($msg) { $msg | Tee-Object -FilePath $log -Append }

Set-Content -Path $log -Value "== au4a deploy verify $(Get-Date -Format o) ==" -Encoding utf8
Push-Location $repo
try {
    Say "[1/4] cargo build --$Configuration -p au4a-node"
    cargo build "--$Configuration" -p au4a-node 2>&1 | Tee-Object -FilePath $log -Append | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "build failed ($LASTEXITCODE)" }

    $exe = Join-Path $repo "target\$Configuration\au4a-node.exe"
    if (-not (Test-Path $exe)) { $exe = Join-Path $repo "target\$Configuration\au4a-node" }
    if (-not (Test-Path $exe)) { throw "binary not found" }

    Say "[2/4] au4a-node verify"
    $verify = & $exe verify --json
    $verify | Tee-Object -FilePath $log -Append | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "verify failed: not all track self-checks passed" }

    Say "[3/4] au4a-node run --agents $Agents"
    $run = & $exe run --agents $Agents --json
    $run | Tee-Object -FilePath $log -Append | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "run failed" }
    $summary = $run | ConvertFrom-Json
    Say ("      agents={0} tracks_ok={1}/{2} ledger_total={3}" -f $summary.agents, $summary.tracks_ok, $summary.tracks_total, $summary.ledger_total)

    Say "[4/4] read-only observer probe on 127.0.0.1:$Port"
    $proc = Start-Process -FilePath $exe -ArgumentList @('run', '--agents', "$Agents", '--observe', "127.0.0.1:$Port") -PassThru -WindowStyle Hidden
    try {
        $base = "http://127.0.0.1:$Port"
        $ready = $false
        for ($i = 0; $i -lt 40; $i++) {
            Start-Sleep -Milliseconds 250
            try { $null = Invoke-WebRequest "$base/api/progress" -TimeoutSec 2 -UseBasicParsing; $ready = $true; break } catch { }
        }
        if (-not $ready) { throw "observer did not become ready" }

        foreach ($route in @('/', '/api/progress', '/api/results', '/api/revenue', '/api/selfcheck', '/api/tracks')) {
            $r = Invoke-WebRequest "$base$route" -TimeoutSec 5 -UseBasicParsing
            Say ("      GET {0,-18} -> {1} ({2} bytes)" -f $route, $r.StatusCode, $r.RawContentLength)
            if ($r.StatusCode -ne 200) { throw "GET $route expected 200, got $($r.StatusCode)" }
        }

        $writeStatus = 0
        try {
            $null = Invoke-WebRequest "$base/api/revenue" -Method POST -TimeoutSec 5 -UseBasicParsing
            $writeStatus = 200
        } catch {
            $writeStatus = [int]$_.Exception.Response.StatusCode
        }
        Say ("      POST /api/revenue  -> {0} (expect 405)" -f $writeStatus)
        if ($writeStatus -ne 405) { throw "observer accepted a write: expected 405, got $writeStatus" }

        Say "RESULT: PASS — 构建 / 自检 / 端到端 / 只读面板四项全部通过"
    }
    finally {
        if ($proc -and -not $proc.HasExited) { Stop-Process -Id $proc.Id -Force }
    }
}
catch {
    Say "RESULT: FAIL — $_"
    exit 1
}
finally {
    Pop-Location
}
exit 0
