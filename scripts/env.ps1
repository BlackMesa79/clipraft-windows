$env:CARGO_HOME='D:\DevTools\cargo'
$env:RUSTUP_HOME='D:\DevTools\rustup'
$env:PATH='D:\DevTools\cargo\bin;'+$env:PATH
$projectRoot = Split-Path $PSScriptRoot -Parent
$env:TEMP=Join-Path $projectRoot '.tools\tmp'
$env:TMP=$env:TEMP
New-Item -ItemType Directory -Force -Path $env:TEMP | Out-Null
$env:CARGO_BUILD_JOBS='2'
$env:LC_ALL='C'
# 只加入 Strawberry Perl 的 perl\bin；不要加入 c\bin，避免 MinGW 工具链干扰 MSVC 构建。
$env:PATH='D:\DevTools\perl\perl\bin;'+$env:PATH
$env:OPENSSL_SRC_PERL='D:\DevTools\perl\perl\bin\perl.exe'
# 只初始化当前进程环境，不修改系统或用户环境变量。
$vcvars='D:\DevTools\BuildTools\VC\Auxiliary\Build\vcvars64.bat'
if ((Test-Path -LiteralPath $vcvars) -and ($env:VCINSTALLDIR -ne 'D:\DevTools\BuildTools\VC\' -or $env:VSCMD_ARG_TGT_ARCH -ne 'x64')) {
    $lines = & cmd.exe /d /s /c ('""{0}" >nul && set"' -f $vcvars)
    foreach ($line in $lines) {
        if ($line -match '^([^=]+)=(.*)$') { [Environment]::SetEnvironmentVariable($matches[1], $matches[2], 'Process') }
    }
}
