param([Parameter(ValueFromRemainingArguments=$true)][string[]]$TauriArgs)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'env.ps1')
$cli=Join-Path (Split-Path $PSScriptRoot -Parent) 'node_modules\@tauri-apps\cli\tauri.js'
& node $cli @TauriArgs
exit $LASTEXITCODE
