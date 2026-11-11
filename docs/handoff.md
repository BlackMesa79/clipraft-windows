# 交接文档（Handoff）

> 更新时间：2026-11-12
> 用途：在新对话中继续开发时，先读这份文档，再读下面列出的资料。

## 一、项目一句话

Windows 剪贴板管理器，目标是集 Ditto（功能全）、CopyQ（分组和脚本）、Clippi（界面好看）的优点，并修掉它们的缺陷：快速粘贴面板（按住快捷键、方向键选择、松开即粘贴）、文件记录可以直接粘贴成文件、所有图片都有预览、不卡输入法、本地数据加密。

## 二、当前进度

- **M0 技术验证：已完成并通过用户人工验收**（`90d12ff`）。
- **M1：T1～T7 已完成、通过用户人工验收并提交。下一步：T8 托盘与系统集成**，之后是 T9 回归测试与人工验收。任务说明见 `docs/M1开发计划.md`。
- M1 完成后：统一打磨界面，并实现垂直 3D 滚轮快速面板（规格见设计文档第三节）。用户明确要求按这个顺序进行。

| 提交 | 内容 | 人工验收备注 |
|---|---|---|
| `1f477ad` | T1 存储层演进：schema 版本迁移、分页列表、去重修复 | 通过 |
| `28c013c` | T2 删除与自动清理：多选删除、blob 引用回收、保留策略、VACUUM | 通过 |
| `ac01c82` / `e577bcb` / `5ee2460` | 存储瘦身（zstd、位图预测压缩、JPEG 预览）、超大图片（512MiB 上限与降级）、OLE 退出固化重复位图 | 通过 |
| `ed942da` | T3 设置系统：强类型设置、即时生效、快捷键切换、清理预估 | 通过 |
| `29c3d3b` | T4 搜索与筛选：FTS5 trigram、拼音、类型筛选（schema v4） | 通过 |
| `84aeea8` | T5 置顶、收藏与分组；右键菜单键盘操作 | 通过 |
| `cc61528` | T6 忽略程序与来源记录 | 通过；密码管理器（用户未安装）与无 owner 分支（无工具）未测 |
| `18aaac3` | T7 敏感内容识别：规则、三态、到期删除、后端打码 | 通过 5 个重点项，其余细项由自动化覆盖 |

**当前测试基线**：`cargo test -- --test-threads=2` 共 **596 passed / 0 failed / 3 ignored**（3 项 ignored 是需要单独运行的性能或大图测试）。离屏 UI 脚本（先 `npm run build`，再 `node scripts/check-m1-tN-ui.mjs`）：T4 17 项、T5 15 项、T6 8 项、T7 20 项。schema 仍为 **v4**。

### T8 要点（下一步）

按 `docs/M1开发计划.md` 的 T8 一节执行，并注意以下几点：
- 设置里已有 `autostart`、`pause_on_start`，但只存储，**还没有执行逻辑**。开机自启有两种做法：写 `HKCU\...\Run`（`windows` crate 已有，可能需要加 `Win32_System_Registry` feature），或使用 `tauri-plugin-autostart`。后者是新依赖，**新增依赖前要先问用户**。
- 暂停监听：托盘图标和提示文字要有区别，暂停期间完全不采集。注意与采集序列号的配合：恢复后不要补采暂停期间的内容。
- 文件记录失效标注：在后台检查并缓存结果，不能卡列表。
- 托盘菜单：显示/隐藏、暂停/恢复监听、设置、退出。
- 之后的 T9 交 Tester 做全量回归，并整理 M1 人工验收清单。`docs/M1开发计划.md` 第四节列出的 M0 遗留问题（D-7、钩子健康监测、启动失败模态框等）可以在 T9 前后由用户决定是否处理。

## 三、必读资料

| 文件 | 内容 |
|---|---|
| `docs/需求整理与选型评估.md` | 需求、功能优先级、技术根因、快速面板交互和滚轮视觉规格、技术选型、加密方案 |
| `docs/M1开发计划.md` | M1 任务拆分（T1～T9）、依赖关系、M0 遗留问题、统一完成标准 |
| `docs/M1存储层说明.md` | schema v4、迁移、去重、清理、搜索、分组、来源、敏感三态等存储细节 |
| `docs/M1人工验收清单.md` | 各任务的人工验收步骤和结果记录 |
| `docs/M1-T4验证报告.md`、`docs/M1-T7测试与审查报告.md` | T4、T7 的专项验证和审查记录 |
| `README.md` | 构建运行、模块说明、已知限制、每个任务的规则说明和验证记录（最权威的实现说明） |
| `docs/M0人工验收清单.md` | M0 验收步骤（含各轮修复的复测项） |

## 四、已确定的决策（不要再重新讨论）

- 技术栈：**Tauri 2 + Rust 核心 + Vue 3 + TypeScript + Vite**，第一阶段**只做 Windows**。
- 加密：SQLCipher 整库加密（`rusqlite` 的 `bundled-sqlcipher-vendored-openssl` 特性）；主密钥 32 字节，由 DPAPI 保护，存放在 `key.bin`；blob 使用 XChaCha20-Poly1305 加密；缩略图和预览只在内存中解密，通过 `thumb://` 自定义协议提供给前端。
- 快速面板最终形态：无背景的垂直 3D 滚轮，默认可见 5 条（以后可在设置中调整），**可以翻完整个历史**，到头后接回另一头。
- 清理保护（T2/T5）：置顶、收藏、属于分组的记录不参与容量和天数清理；普通策略中最新一条豁免。
- 来源与忽略（T6）：剪贴板 owner 优先，取不到时回退到前台窗口（排除本程序）；忽略判定发生在**任何 `GetClipboardData` 之前**，命中后不读取、不保存、不 touch。
- 敏感内容（T7）：
  - `sensitive` 列为三态：0 未标记，1 敏感，-1 用户确认不敏感（再次复制不会重标）。
  - **打码在后端完成**，只有 `reveal` 返回原文。
  - 敏感记录不进入搜索索引。
  - 置顶、收藏、移入分组时清除到期时间；之后取消这些状态也不恢复倒计时。
  - 到期删除不豁免最新一条，到期后 5 秒内删除。
  - 修改 TTL 不影响已有记录。
  - 到期时不清空系统剪贴板。

## 五、环境（重要）

- Rust 工具链在 `D:\DevTools\cargo` 和 `D:\DevTools\rustup`，**没有加入全局环境变量**；MSVC 构建工具在 `D:\DevTools\BuildTools`；Strawberry Perl（编译 OpenSSL 用）在 `D:\DevTools\perl`。
- **每次执行 cargo 或 npm tauri 命令前，必须先在 PowerShell 中运行 `. .\scripts\env.ps1`。**这个脚本只设置当前进程的环境变量。不要改回项目内的 `.tools\perl`，也不要把 Perl 的 `c\bin` 加入 PATH。cargo 命令统一加 `--manifest-path src-tauri/Cargo.toml`，测试加 `-- --test-threads=2`。
- 系统代理：`127.0.0.1:7890`。直连 GitHub 下载经常被重置，需要时用 `curl -x http://127.0.0.1:7890`。
- 没有安装 `gh` 和 `pnpm`，使用 npm。
- 在 Git Bash 里调用 PowerShell 时，把命令写进 `.ps1` 文件，再用 `powershell -NoProfile -ExecutionPolicy Bypass -File` 执行，避免引号转义问题。
- 用户人工测试的启动命令：
  ```powershell
  . .\scripts\env.ps1
  $env:CLIPBOARD_M0_DATA_DIR="$PWD\.test-data\manual"
  $env:CLIPBOARD_M0_DIAG="1"   # 可选：诊断日志，写入数据目录下的 diag.log
  npm run tauri dev
  ```
  `.test-data\manual` 是用户的测试数据，**不要删除或修改**。
- 测试辅助数据：`.test-data\t7-samples\`（T7 合成假 token 样本，以及一键复制脚本 `copy-sample.ps1`），`.test-data\` 已被 git 忽略。

## 六、协作方式

- 用户要求：**开发委派给 Coder（codeg assistant_id=2），测试委派给 Tester（assistant_id=3）**，通过 `codeg-mcp` 的 `delegate_to_agent` 调用（传 `assistant_id`，不传 `agent_type`），不要用原生子智能体。主对话负责需求、设计、拆分任务、复核和提交。
- Coder 和 Tester 的固定规则：**不 commit、不安装软件、不新增依赖（需要时先问）、不写项目以外的位置、不碰 `.test-data\manual`、不接管真实键鼠和桌面剪贴板**（需要剪贴板的测试使用私有窗口站，见 `test_support.rs`）。Tester 只补测试、不改产品代码；发现缺陷时写 `#[ignore = "T? 待修：..."]` 测试，由主对话交给 Coder 修复。
- Coder 遇到需要用户决定的问题时会在 Codeg 里停下来提问，要及时告诉用户。
- 委派任务时，写清楚背景资料、环境约束、当前测试基线、已有基础、需求或规格、完成标准（日志写到 `.tools/tN-*.log`，并更新 README 和人工验收清单），以及需要返回的内容。子智能体看不到本对话，任务描述必须自包含。
- 每轮流程：
  1. Coder 完成开发。
  2. 主对话复跑 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、`npm run build` 和各离屏脚本，并抽查关键代码。
  3. 交 Tester 审查补测；发现问题时交回 Coder 修复，修完主对话再复跑一遍。
  4. 用户人工验收。
  5. 主对话在 README 和验收清单中记录结果，然后提交。
- 用户人工测试期间，不要让 Coder 改代码（开发模式会自动重新编译，打断测试）。
- 用户反馈问题时，优先读 `.test-data\manual\diag.log`（最后一个 `session` 之后的部分）来定位，不要靠猜。
- 用户偏好：先把基本功能做扎实，可以为人工验收准备测试数据或脚本；全程使用简体中文。

## 七、踩过的坑（不要退化）

### M0

| 问题 | 根因 | 现在的做法 |
|---|---|---|
| 快速面板会切换输入法 | 吞掉 V 之后，系统看到的是 Ctrl+Shift 单独按下 | 面板一呼出就立即注入掩码键 0xE8（`paster::mask_shortcut`） |
| Alt+V 会弹出系统菜单 | 热键用掉了 V，系统看到 Alt 单独按下又松开；**tao 的 `set_focus()` 失败时还会自己注入 Alt** | 热键触发后立即注入掩码键；**不再调用 `window.set_focus()`**，改用原生 `SetForegroundWindow`；主窗口刚显示后 300ms 内拦截由 Alt 触发的 `SC_KEYMENU` |
| 修饰键"卡死"，一直提示"请松开修饰键" | 钩子会丢失抬起事件，靠钩子事件累积出来的 `PHYSICAL` 状态会永久卡住 | 粘贴只依据 `GetAsyncKeyState` 判断；钩子状态机逐事件校正（`hook::corrected_modifiers`） |
| 粘贴后键盘锁死 | 旧代码粘贴前后会重新按下修饰键，并有一个修复循环 | 等所有修饰键都松开后，只发送干净的 Ctrl↓ V↓ V↑ Ctrl↑ |
| 大图粘贴慢，接着再粘贴时报错 | dev 构建没开优化；解密和写入都在关键路径上；置顶时重新计算哈希 | dev 构建优化依赖；先写入剪贴板再等待按键释放；粘贴代号机制，新操作使旧任务静默放弃；置顶用 `Storage::touch` |
| 误报"采集失败" | 目标程序长时间占用剪贴板，而程序在打开剪贴板之后才判断是否为自写回 | 先判断自写回；被占用时退避重试；同一序列号只处理一轮（`capture_policy.rs`） |
| 主窗口呼不出来 | Toggle 只判断"是否可见" | 只有在"可见、未最小化、是前台窗口"时才隐藏 |

### M1

| 问题 | 做法 |
|---|---|
| 快速面板发布路径访问存储锁（T7 初版） | `publish()` 和 `panel_state` **不能获取存储锁或执行 SQL**，否则 VACUUM 或清理时面板会卡住。历史变化时，用内存中 `state.items` 的最新对象替换快照 |
| 同时需要两把锁 | 一律**先 `capture_context` 后 `storage`**（保存、删除、清理、到期线程都遵守），禁止反向嵌套 |
| Tester 拒绝"验证泄露/复现漏洞"类措辞的任务 | 按"功能规格 + 补充测试"的中性方式描述，例如"打码完整性"，不要写"泄露面""复现缺陷" |
| PowerShell 用 `*>` 重定向的日志是 UTF-16LE | 用 `iconv -f UTF-16LE -t UTF-8` 读取后再 grep |
| 在 Git Bash 用 heredoc 或 `python -c` 写含反斜杠或反引号的内容会被改写（例如 `\t` 变成制表符） | 含 Windows 路径或 PowerShell 反引号的文件用 Write 工具写 |
| VACUUM 会改写隐式 rowid | FTS 映射使用显式 INTEGER PRIMARY KEY（T4） |

## 八、代码结构速览

### 后端 `src-tauri/src`

| 模块 | 职责 |
|---|---|
| `lib.rs` | Tauri 启动、命令注册、托盘、`thumb://` 协议（拒绝敏感记录） |
| `runtime.rs` | 监听窗口与 broker 线程、worker（采集、粘贴、节流清理）、面板控制线程、粘贴代号、`clipboard-expiry` 到期线程、设置即时生效 |
| `hook.rs` | 低级键盘钩子、纯状态机 `Machine`、修饰键校正、可配置触发键 |
| `paster.rs` | 等待修饰键松开、掩码键、SendInput、粘贴阶段编排 |
| `clipboard.rs` / `capture_policy.rs` | 多格式采集和写回、隐私标记、忽略判定入口、被占用时的重试策略 |
| `clipboard_owner.rs` / `capture_source.rs` | owner 与窗口进程查询；来源选择与忽略匹配（T6） |
| `owner_flush.rs` | 识别 OLE 程序退出时固化剪贴板产生的重复记录（只用内存上下文） |
| `clipboard_diag.rs` / `diag.rs` | 采集诊断（只记元数据）；可选诊断日志（按键脱敏，不记内容） |
| `image_normalize.rs` / `thumbnail.rs` | 去除 PNG/DIB 尾部填充；图片解码、缩略图和预览 |
| `sensitive.rs` | T7 规则匹配、打码、到期调度计算（纯函数） |
| `settings.rs` | 强类型设置、默认值、校验、快捷键候选 |
| `platform.rs` | DPAPI、数据目录、光标定位、不激活窗口、主窗口菜单拦截 |
| `model.rs` | `Clip`、`ItemView`、哈希、CF_HDROP 编解码 |
| `test_support.rs` / `paste_bench.rs` | 私有窗口站测试环境；需要显式运行的大图基准 |
| `storage/` | `mod.rs`（保存、查重、列表）、`migrations.rs`（v0→v4）、`crypto.rs`、`predict.rs`（位图预测压缩）、`maintenance.rs`（删除、清理、VACUUM、孤儿扫描）、`search.rs`（FTS 与拼音）、`groups.rs`、`settings.rs`、`sensitivity.rs`（三态、打码出口、到期删除）；各自配有 `*_tests.rs` |

集成测试 `src-tauri/tests/`：`hook_machine`、`model_hdrop`、`paster_modifiers`、`sensitive_rules`、`storage_encryption`、`thumbnail_decode`。

### 前端 `src`

`App.vue`（主窗口和快速面板共用的入口与状态）、`HistoryList.vue`（列表、标记、倒计时）、`ContextMenu.vue`（记录和分组的右键菜单，支持键盘操作）、`GroupSidebar.vue`（顶部分组栏）、`SettingsView.vue`、`PreviewImage.vue`、`history.ts`、`settings.ts`、`style.css`。

离屏 UI 检查脚本：`scripts/check-m1-ui.mjs`、`check-m1-t2`～`t7-ui.mjs`（使用独立 Edge 无头 profile 和合成 IPC 数据，`settings-ui-mock.mjs` 是公共的设置模拟）。
