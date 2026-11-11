# 交接文档（Handoff）

> 更新时间：2026-11-11
> 用途：在新对话中继续开发时，先读这份文档，再读下面列出的资料。

## 一、项目一句话

Windows 剪贴板管理器，目标是集 Ditto（功能全）、CopyQ（分组和脚本）、Clippi（界面好看）的优点，并修掉它们的缺陷：快速粘贴面板（按住快捷键、方向键选择、松开即粘贴）、文件记录可以直接粘贴成文件、所有图片都有预览、不卡输入法、本地数据加密。

## 二、当前进度

- **M0 技术验证：已完成并通过用户人工验收。**最新提交为 `90d12ff`。
- **M1 进行中**，任务拆分见 `docs/M1开发计划.md`（T1～T9）。T1～T5 已完成并通过用户人工验收（T5 于 2026-11-11 验收），另完成了存储瘦身、超大图片、OLE 退出固化等修复。**下一步：T6 忽略程序与来源记录。**
- 之后的阶段：M1 完成后，统一打磨界面，并实现垂直 3D 滚轮快速面板（规格见设计文档第三节）。用户明确要求按这个顺序进行。

## 三、必读资料

| 文件 | 内容 |
|---|---|
| `docs/需求整理与选型评估.md` | 需求、功能优先级、技术根因、快速面板交互和滚轮视觉规格、技术选型、加密方案 |
| `docs/M1开发计划.md` | M1 任务拆分、依赖关系、M0 遗留问题、完成标准 |
| `README.md` | 构建运行方法、模块说明、已知限制、各轮验证记录、诊断日志说明 |
| `docs/M0人工验收清单.md` | 人工验收步骤（包括 M0 各轮修复的复测项） |

## 四、已确定的决策（不要再重新讨论）

- 技术栈：**Tauri 2 + Rust 核心 + Vue 3 + TypeScript + Vite**，第一阶段**只做 Windows**。
- 加密：SQLCipher 整库加密（`rusqlite` 的 `bundled-sqlcipher-vendored-openssl` 特性）；主密钥 32 字节，由 DPAPI 保护，存放在 `key.bin`；blob 使用 XChaCha20-Poly1305 加密；缩略图和预览只在内存中解密，通过 `thumb://` 自定义协议提供给前端。
- 快速面板最终形态：无背景的垂直 3D 滚轮，默认可见 5 条（以后可在设置中调整），**可以翻完整个历史**，到头后接回另一头。

## 五、环境（重要）

- Rust 工具链在 `D:\DevTools\cargo` 和 `D:\DevTools\rustup`，**没有加入全局环境变量**；MSVC 构建工具在 `D:\DevTools\BuildTools`；Strawberry Perl（编译 OpenSSL 用）在 `D:\DevTools\perl`。
- **每次执行 cargo 或 npm tauri 命令前，必须先在 PowerShell 中运行 `. .\scripts\env.ps1`。**这个脚本只设置当前进程的环境变量。不要改回项目内的 `.tools\perl`，也不要把 Perl 的 `c\bin` 加入 PATH。
- 系统代理：`127.0.0.1:7890`。直连 GitHub 下载经常被重置，需要时用 `curl -x http://127.0.0.1:7890`。
- 没有安装 `gh` 和 `pnpm`，使用 npm。
- 在 Git Bash 里调用 PowerShell 时，建议把命令写进 `.ps1` 文件，再用 `powershell -NoProfile -ExecutionPolicy Bypass -File` 执行，避免引号转义问题。
- 用户人工测试的启动命令：
  ```powershell
  . .\scripts\env.ps1
  $env:CLIPBOARD_M0_DATA_DIR="$PWD\.test-data\manual"
  $env:CLIPBOARD_M0_DIAG="1"   # 可选：诊断日志，写入数据目录下的 diag.log
  npm run tauri dev
  ```
  `.test-data\manual` 是用户的测试数据，**不要删除或修改**。

## 六、协作方式

- 用户要求：**开发委派给 Coder（codeg assistant_id=2），测试委派给 Tester（assistant_id=3）**，通过 `codeg-mcp` 的 `delegate_to_agent` 调用，不要用原生子智能体。主对话负责需求、设计、拆分任务、复核和提交。
- Coder 的固定规则：**不 commit、不安装软件、不写项目以外的位置**。需要安装的东西由主对话或用户处理；提交由主对话在复核和用户验收通过后完成。
- Coder 遇到需要用户决定的问题时会在 Codeg 里停下来提问，要及时告诉用户。
- 委派任务时，写清楚环境约束（`env.ps1`、不动 `.test-data\manual`、不 commit）、当前测试基线、完成标准和需要返回的内容。
- 每轮流程：Coder 完成 → 主对话复跑 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、`npm run build`，并抽查关键代码 → 用户人工复测 → 提交。用户人工测试期间，不要让 Coder 改代码（开发模式会自动重新编译，打断测试）。
- 用户反馈问题时，优先读 `.test-data\manual\diag.log`（最后一个 `session` 之后的部分）来定位，不要靠猜。

## 七、M0 中踩过的坑（M1 不要退化）

| 问题 | 根因 | 现在的做法 |
|---|---|---|
| 快速面板会切换输入法 | 吞掉 V 之后，系统看到的是 Ctrl+Shift 单独按下 | 面板一呼出就立即注入掩码键 0xE8（`paster::mask_shortcut`） |
| Alt+V 会弹出系统菜单 | 热键用掉了 V，系统看到 Alt 单独按下又松开；**tao 的 `set_focus()` 失败时还会自己注入 Alt** | 热键触发后立即注入掩码键；**不再调用 `window.set_focus()`**，改用原生 `SetForegroundWindow`；主窗口刚显示后 300ms 内拦截由 Alt 触发的 `SC_KEYMENU` |
| 修饰键"卡死"，一直提示"请松开修饰键" | 钩子会丢失抬起事件，靠钩子事件累积出来的 `PHYSICAL` 状态会永久卡住 | 粘贴只依据 `GetAsyncKeyState` 判断；钩子状态机逐事件校正（`hook::corrected_modifiers`） |
| 粘贴后键盘锁死 | 旧代码粘贴前后会重新按下修饰键，并有一个修复循环 | 等所有修饰键都松开后，只发送干净的 Ctrl↓ V↓ V↑ Ctrl↑ |
| 大图粘贴慢，接着再粘贴时报错 | dev 构建没开优化；解密和写入都在关键路径上；置顶时重新计算哈希 | dev 构建优化依赖；先写入剪贴板再等待按键释放；粘贴代号机制，新操作使旧任务静默放弃；置顶用 `Storage::touch` |
| 误报"采集失败" | 目标程序长时间占用剪贴板，而程序在打开剪贴板之后才判断是否为自写回 | 先判断自写回；被占用时退避重试；同一序列号只处理一轮（`capture_policy.rs`） |
| 主窗口呼不出来 | Toggle 只判断"是否可见" | 只有在"可见、未最小化、是前台窗口"时才隐藏 |

## 八、代码结构速览（`src-tauri/src`）

| 模块 | 职责 |
|---|---|
| `lib.rs` | Tauri 启动、命令注册、托盘、`thumb://` 协议 |
| `runtime.rs` | 监听窗口与 broker 线程、worker（采集和粘贴任务）、面板控制线程、粘贴代号 |
| `hook.rs` | 低级键盘钩子、纯状态机 `Machine`、修饰键校正 |
| `paster.rs` | 等待修饰键松开、掩码键、SendInput、粘贴阶段编排 |
| `clipboard.rs` / `capture_policy.rs` | 多格式采集和写回、隐私标记、被占用时的重试策略 |
| `platform.rs` | DPAPI、数据目录、光标定位、不激活窗口、主窗口菜单拦截 |
| `storage/` | SQLCipher、去重、加密 blob、`touch` |
| `thumbnail.rs` | 图片解码、缩略图和预览 |
| `diag.rs` | 可选诊断日志（按键脱敏，不记录剪贴板内容） |
| `model.rs` | `Clip`、`ItemView`、哈希、CF_HDROP 编解码 |

前端：`src/App.vue`（主窗口和快速面板共用）、`src/style.css`。

测试：基线 **484 项通过，3 项 ignored**（T5 完成时）。存储层细节见 `docs/M1存储层说明.md`（当前 schema v4）。
