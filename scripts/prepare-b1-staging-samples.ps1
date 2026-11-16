# 仅在独立验收目录生成合成暂存样本，不读取用户历史。
param([string]$Session = 'b1-acceptance')
$ErrorActionPreference = 'Stop'
if ($Session -notmatch '^b1-acceptance[-a-zA-Z0-9]*$') { throw '验收会话名无效。' }
$sampleProject = Split-Path $PSScriptRoot -Parent
$sampleParent = [IO.Path]::GetFullPath((Join-Path $sampleProject ".test-data/$Session/staging"))
New-Item -ItemType Directory -Path $sampleParent -Force | Out-Null
$samplePaths = [System.Collections.Generic.List[object]]::new()
foreach ($sampleKind in @('empty', 'partial', 'unknown')) {
    $sampleId = [Guid]::NewGuid().ToString('N')
    $sampleRoot = Join-Path $sampleParent ".clipraft-export-$sampleId"
    New-Item -ItemType Directory -Path $sampleRoot | Out-Null
    $sampleOwner = @{ version = 1; kind = 'export'; root = "\\?\$sampleRoot" } | ConvertTo-Json -Compress
    [IO.File]::WriteAllText((Join-Path $sampleRoot '.clipraft-owner.json'), $sampleOwner, [Text.UTF8Encoding]::new($false))
    if ($sampleKind -eq 'partial') { [IO.File]::WriteAllText((Join-Path $sampleRoot 'archive.part'), 'B1 synthetic interrupted archive; no real history.') }
    if ($sampleKind -eq 'unknown') { [IO.File]::WriteAllText((Join-Path $sampleRoot 'user-note.txt'), 'Keep this synthetic unknown user file.') }
    $samplePaths.Add([pscustomobject]@{ kind = $sampleKind; path = $sampleRoot })
}
$samplePaths | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $sampleParent 'sample-index.json') -Encoding utf8
Write-Output "合成暂存样本已生成：$sampleParent"
Write-Output 'empty 应可预检清空；partial 应显示1个文件并要求确认；unknown 必须拒绝预检并保留。'
