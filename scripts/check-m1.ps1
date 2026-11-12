# M1 串行回归入口；只使用已有工具和离屏测试，不接管桌面。
$ErrorActionPreference = 'Stop'
$regressionRoot = Split-Path $PSScriptRoot -Parent
$env:RAYON_NUM_THREADS = '2'
Push-Location $regressionRoot
try {
    $regressionLogs = Join-Path $regressionRoot ('.tools/t9-regression-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
    New-Item -ItemType Directory -Path $regressionLogs -Force | Out-Null
    $results = [System.Collections.Generic.List[object]]::new()
    function Invoke-RegressionStep([string]$Name, [string]$Program, [string[]]$Arguments) {
        if ($Program -eq 'cargo') { . "$PSScriptRoot/env.ps1" }
        $null = Get-Command $Program -ErrorAction Stop
        $log = Join-Path $regressionLogs "$Name.log"
        Write-Host "检查 $Name（日志：$log）"
        $started = Get-Date
        # Windows PowerShell 会将原生 stderr 包装为 ErrorRecord；保留日志并按退出码判断。
        $ErrorActionPreference = 'Continue'
        & $Program @Arguments 2>&1 | Out-File -LiteralPath $log -Encoding utf8
        $code = $LASTEXITCODE
        $ErrorActionPreference = 'Stop'
        $results.Add([pscustomobject]@{
            name = $Name; exit_code = $code; seconds = ((Get-Date) - $started).TotalSeconds; log = $log
        })
        $results | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $regressionLogs 'result.json') -Encoding utf8
        if ($code -ne 0) { throw "$Name 失败（退出码 $code），请检查 $log" }
    }
    Invoke-RegressionStep 'fmt' 'cargo' @('fmt', '--manifest-path', 'src-tauri/Cargo.toml', '--check')
    Invoke-RegressionStep 'clippy' 'cargo' @('clippy', '--manifest-path', 'src-tauri/Cargo.toml', '--all-targets', '--', '-D', 'warnings')
    Invoke-RegressionStep 'rust' 'cargo' @('test', '--manifest-path', 'src-tauri/Cargo.toml', '--', '--test-threads=2')
    Invoke-RegressionStep 'build' 'npm.cmd' @('run', 'build')
    Invoke-RegressionStep 'ui-t1' 'node' @('scripts/check-m1-ui.mjs')
    foreach ($number in 2..8) {
        Invoke-RegressionStep "ui-t$number" 'node' @("scripts/check-m1-t$number-ui.mjs")
    }
    Write-Host "M1 常规自动回归通过。结果：$regressionLogs/result.json"
    Write-Host '默认 ignored 的大图/性能测试及原生人工验收需另行执行。'
} finally {
    Pop-Location
}
