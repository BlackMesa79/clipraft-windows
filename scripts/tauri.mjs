import { spawn } from 'node:child_process';
import { join } from 'node:path';
// PowerShell 包装器只初始化当前子进程环境，并沿用项目本地依赖。
const child = spawn('powershell.exe', ['-NoProfile', '-NonInteractive', '-File', join(process.cwd(), 'scripts', 'run-tauri.ps1'), ...process.argv.slice(2)], { stdio: 'inherit', shell: false, windowsHide: true });
child.on('error', () => process.exit(1));
child.on('exit', code => process.exit(code ?? 1));
