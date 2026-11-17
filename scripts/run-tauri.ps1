# 原样转发CLI参数（包括 --），避免PowerShell高级参数绑定截获选项。
[string[]]$TauriArgs = $args
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'env.ps1')
$cli=Join-Path (Split-Path $PSScriptRoot -Parent) 'node_modules\@tauri-apps\cli\tauri.js'
& node $cli @TauriArgs
exit $LASTEXITCODE
