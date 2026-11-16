# 用户手动运行的独立外测前验收入口；不会自动启动，也不访问 .test-data/manual。
param([string]$Session = 'pretest-acceptance')
$ErrorActionPreference = 'Stop'
if ($Session -notmatch '^pretest-acceptance[-a-zA-Z0-9]*$') { throw '会话名必须以 pretest-acceptance 开头，只包含字母、数字和连字符。' }
$acceptanceProject = Split-Path $PSScriptRoot -Parent
$acceptanceRoot = Join-Path $acceptanceProject ".test-data/$Session"
$acceptanceBinary = Join-Path $acceptanceProject '.tools/pretest-preview-secondary-bin/ClipRaft.exe'
if (-not (Test-Path -LiteralPath $acceptanceBinary -PathType Leaf)) { throw '验收程序尚未准备好，请先完成构建。' }
foreach ($name in @('source','profile','exports','restores','migrated')) {
    New-Item -ItemType Directory -Path (Join-Path $acceptanceRoot $name) -Force | Out-Null
}
$acceptanceOldData = $env:CLIPBOARD_M0_DATA_DIR
$acceptanceOldConfig = $env:CLIPRAFT_TEST_CONFIG_DIR
$acceptanceOldSmoke = $env:CLIPBOARD_M0_SMOKE
$acceptanceOldDiag = $env:CLIPBOARD_M0_DIAG
try {
    $env:CLIPBOARD_M0_DATA_DIR = Join-Path $acceptanceRoot 'source'
    $env:CLIPRAFT_TEST_CONFIG_DIR = Join-Path $acceptanceRoot 'profile'
    $env:CLIPBOARD_M0_SMOKE = '0'
    $env:CLIPBOARD_M0_DIAG = '1'
    Start-Process -FilePath $acceptanceBinary -WorkingDirectory $acceptanceRoot -WindowStyle Hidden | Out-Null
    Write-Host "外测前验收程序已启动。通过托盘打开主窗口。验收目录：$acceptanceRoot"
    Write-Host '本会话配置和数据独立；重复使用同一会话名会保留此前验收数据。请不要同时运行正式实例。'
} finally {
    $env:CLIPBOARD_M0_DATA_DIR = $acceptanceOldData
    $env:CLIPRAFT_TEST_CONFIG_DIR = $acceptanceOldConfig
    $env:CLIPBOARD_M0_SMOKE = $acceptanceOldSmoke
    $env:CLIPBOARD_M0_DIAG = $acceptanceOldDiag
}
