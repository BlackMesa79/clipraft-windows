# Windows 剪贴板管理器 M0

Tauri 2 + Rust + Vue 3 的 Windows 技术验证原型。界面启动时隐藏，通过托盘或 Alt+V 打开，不要求管理员权限。当前不提供安装包。

## 构建与运行

在项目根目录打开 PowerShell：

~~~powershell
# 每次 cargo / tauri 执行前，设置本次进程环境。
$env:CARGO_HOME='D:\DevTools\cargo'
$env:RUSTUP_HOME='D:\DevTools\rustup'
$env:PATH='D:\DevTools\cargo\bin;'+$env:PATH

# 包含上述设置，并加入 D:\DevTools\perl 的 Perl、初始化 MSVC、限制构建并发为 2。
. .\scripts\env.ps1
npm install
npm run build
. .\scripts\env.ps1
cargo build --manifest-path src-tauri\Cargo.toml
. .\scripts\env.ps1
cargo clippy --manifest-path src-tauri\Cargo.toml --all-targets -- -D warnings
. .\scripts\env.ps1
cargo test --manifest-path src-tauri\Cargo.toml -- --test-threads=2
. .\scripts\env.ps1
npm run tauri dev
~~~

npm run tauri dev 自带环境包装器，使用 D:\DevTools\BuildTools\VC\Auxiliary\Build\vcvars64.bat。脚本只设置子进程环境，不改变系统和用户环境变量。路径不可用时，应修正为已有 MSVC 安装的位置，不安装或修改工具链。

npm 缓存位于 node_modules/.cache/npm，Cargo 缓存位于 D:\DevTools\cargo，构建产物位于 src-tauri/target，临时文件位于 .tools/tmp。应保留 package-lock.json 与 Cargo.lock。本次任务不执行 Git 初始化、提交或推送。

SQLCipher 的指定特性需要 OpenSSL 源码构建，依赖完整 Perl。使用便携版 Strawberry Perl 5.42.3.1，位于 D:\DevTools\perl（整包 SHA256 已与官方发布页核对一致）。env.ps1 只把 perl\bin 加入当前进程 PATH，不加入 c\bin，以免 MinGW 干扰 MSVC。未修改注册表或全局 PATH。

## 目录与模块

~~~text
src/                       Vue 主窗口和快速面板
scripts/
  env.ps1                  Rust、Perl、MSVC 和项目临时目录
  run-tauri.ps1             Tauri CLI 环境包装器
  tauri.mjs                npm 入口
src-tauri/src/
  lib.rs                   Tauri 启动、命令、托盘、缩略图协议
  model.rs                 多格式模型、内容哈希、CF_HDROP 编解码
  hook.rs                  纯状态机、低级键盘钩子、物理修饰键状态
  clipboard.rs             Win32 采集、隐私过滤、多格式写回
  paster.rs                目标校验、焦点恢复、等待修饰键释放、掩码键和 Ctrl+V
  diag.rs                  可选脱敏诊断、专用日志线程
  platform.rs              DPAPI、数据目录、光标定位、不激活窗口
  runtime.rs               消息窗口、工作线程、历史快照、UI 协调
  thumbnail.rs             图片解码、缩略图、尺寸和失败占位
  storage/mod.rs           SQLCipher、去重和加密大对象
  storage/crypto.rs        HKDF、XChaCha20-Poly1305、随机数
src-tauri/capabilities/     本地窗口权限
src-tauri/icons/            Windows 图标
.test-data/                隔离测试数据，不提交
.tools/                    工具、构建日志和临时文件，不提交
~~~

Clip 保留同一记录的全部受支持格式，hash 对排序后的格式名称、长度和内容计算哈希。Storage::save/load/list 是存储接口。Machine::step 是不调用 Win32 的键盘状态机，release_decision 是等待修饰键释放的纯逻辑，toggle_decision 是主窗口显隐决策的纯逻辑，thumbnail::generate 可独立测试。

## 已实现范围

- Ctrl+Shift+V 打开快速面板，默认第二条，只有一条时选第一条。V 每次独立按下前进一格，按住 V 的自动重复不前进；↓/↑ 向后/向前且允许自动重复、循环选择，Esc 取消。必需修饰键组合首次不再满足时提交，例如先松开 Shift。提交后每 5ms 检查 8 个修饰键的 PHYSICAL 与 GetAsyncKeyState；全部释放后才粘贴，最多等待 1 秒，超时提示“请松开修饰键后重试”。正常粘贴仅发送 Ctrl↓、V↓、V↑、Ctrl↑，不重新按下修饰键。主窗口路径使用同一等待逻辑。
- 面板预创建并隐藏，设置 NOACTIVATE / TOPMOST / TOOLWINDOW，以不激活方式显示，忽略鼠标输入。没有使用 AttachThreadInput。优先定位系统插入光标，失败则使用鼠标位置，并限制在屏幕工作区。
- Open 到达 broker 后立即用 SendInput 发送未分配虚拟键 0xE8 的按下/抬起作为掩码，降低纯 Ctrl+Shift / Shift 被系统或输入法解释为切换命令的风险；钩子忽略注入事件，不更改物理键状态。实际输入法兼容性须人工复测。
- 键盘钩子拥有独立消息循环。回调只更新固定大小状态、读写原子变量和 PostThreadMessage，不执行数据库、图片解码或 UI 调用。
- 面板打开时固定最多九条的列表快照，选中状态由 Rust 事件推送。新采集记录不会改变当前选择。粘贴失败或队列繁忙时，以 NOACTIVATE 快速面板显示错误约 2 秒，随后清除提示并隐藏；重新开始选择会取消旧提示的关闭期限。
- Alt+V 仅在主窗口可见、未最小化且位于前台时隐藏；其他情况恢复最小化、显示、提升窗口并请求焦点。托盘采用相同决策。回车或双击粘贴，Shift+Enter 只写回已有纯文本格式；无纯文本则返回错误。关闭按钮和 Esc 隐藏主窗口，托盘菜单退出应用。
- 采集 CF_UNICODETEXT、HTML Format、Rich Text Format、PNG、CF_DIBV5、CF_DIB、CF_HDROP 和 Preferred DropEffect。文件规范化为 UTF-16 DROPFILES，缺失或不足 4 字节的 DropEffect 默认复制，写回时重新构造文件格式。
- 受支持的单个格式读取失败时，只记录固定格式名并跳过该格式；其他成功格式仍保存，全部失败则不记录。总大小超限仍拒绝整条记录；隐私标记不可读时保持不记录。
- 自写回通过私有标记和序列号识别，成功粘贴后将原记录置顶。监控排除标记直接忽略；CanIncludeInClipboardHistory 为零或格式损坏时忽略。日志不记录内容。
- 按 PNG、DIBV5、DIB 的顺序尝试解码；处理 32 位 DIBV5 位域透明度。失败显示格式、尺寸或“尺寸未知”，不会留下空白。
- SQLCipher 整库加密，随机 32 字节主密钥由当前用户 DPAPI 保护。原图、缩略图、预览图及超过 64 KiB 的格式数据经 HKDF 派生密钥和 XChaCha20-Poly1305 加密。对象名为内容哈希，并绑定到认证数据。
- 主窗口显示选中项预览：图片额外保存最长边不超过 800px 的 PNG（加密落盘），旧记录没有 preview 时回退到缩略图；文本显示前 20000 个 Unicode 字符；文件显示完整路径及当前存在状态，可用“刷新状态”重新检查。
- 新文件记录的第一个文件若为 png/jpg/jpeg/gif/bmp/webp 且不超过 50 MiB，会生成缩略图和预览。读取或解码失败保留记录并显示具体占位。预览是保存时快照，不随源文件修改自动更新。
- 缩略图通过自定义协议在内存中解密；协议只允许读取数据库 view 中 thumbnail 或 preview 字段登记的哈希，不开放任意文件或未登记对象，响应禁止缓存。

正常运行数据位于 %APPDATA%\ClipboardM0：key.bin、history.db、data/blobs/ 和 WebView 数据目录。不要单独删除 key.bin；存在数据库却缺少密钥时会拒绝生成替代密钥。启动时分别提示 DPAPI 解密失败（可能更换了 Windows 账户或电脑）、key.bin 长度异常和数据库密钥不匹配，并提醒不要删除数据。数据库损坏也可能产生与错密钥相同的 SQLCipher 错误，提示不会武断地排除损坏。

## 未实现与已知限制

- 不实现 M1 的搜索、分组、收藏、历史清理、开机自启、应用忽略名单、敏感内容识别和自动删除。
- 历史暂无自动清理，主界面只取最近 200 条。快捷键是源码常量，无设置界面；快速组合支持配置 Ctrl/Shift 与触发键，尚未开放 Alt/Win 组合配置；掩码机制已独立实现。
- 单次采集总量上限 128 MiB，最多 4096 个文件；图片解码有尺寸和内存上限，超过限制会提示。
- 文件记录保存路径和复制/剪切语义，不保存文件快照。源文件失效时阻止粘贴并报错，详情在选中、窗口获取焦点或点击“刷新状态”时检查存在性；不进行持续文件监控。剪切记录使用后可能因源路径消失而失效。
- 图片尾部填充仍可能影响去重（D-6）；本轮未处理 D-7、D-10 及 R 系列风险点。
- 不抓取 HTML 远程图片，不对 HEIC、SVG 生成预览，GIF/WebP 仅预览静态帧。支持的剪贴板图片解码失败仍保留原始数据并显示占位。
- 目标应用是否接受富文本、透明图片和文件取决于其格式支持。Word 专有格式、CF_BITMAP、OLE 对象不在本阶段范围。
- 普通权限进程遇到管理员目标会报错，不提升权限或绕过 UIPI。窗口关闭、前台变化或系统拒绝恢复焦点时取消粘贴。
- 剪贴板写回和目标程序消费之间没有事务保证。写回成功但按键发送失败时，剪贴板仍为选中记录。SendInput 成功也不表示目标应用一定接受内容。
- 暂无生产级钩子健康监测和掉钩自动恢复。16ms 显示延迟、多屏 DPI、远程桌面、休眠、输入法和修饰键竞态需人工实测。
- 加密保护落盘数据，不防护当前 Windows 用户下的恶意进程或进程内存读取。

## 人工验收步骤

使用非敏感合成内容。自动检查不接管键鼠、不向真实应用粘贴。以下操作由测试人员在合适时机执行。

1. **启动和托盘**：运行 npm run tauri dev，检查托盘；Alt+V 显示和隐藏，关闭主窗口后可再次呼出；托盘退出后快捷键恢复到目标应用原有行为。
2. **快速选择**：在记事本依次复制 A、B、C，历史应为 C、B、A。激活另一个编辑位置，按住 Ctrl+Shift 后按 V，默认选 B；V/↓ 选 A，↑ 回 B；松开修饰键应只粘贴一次。再次复制相同内容应置顶，不增加条数。
3. **取消与成对按键**：呼出后 Esc 应不粘贴。按住方向键自动重复，再先松 Shift、后松方向键，检查目标光标没有异常。测试左右 Ctrl/Shift、双 Ctrl，以及不同松键顺序。
4. **释放等待与输入恢复**：快速面板及 Shift+Enter 粘贴后输入普通字母，再测试 Ctrl+A、Shift+方向键、Alt 菜单与 Win 键。提交后保持 Ctrl 或 Shift 未松开，确认不会提前粘贴；在 1 秒内释放后应粘贴，超过 1 秒则取消并提示。粘贴后立即打字、退格、回车，确认无需 Alt+V 即可继续输入。连续至少 50 次，在中文输入法组合输入状态下重复。
5. **目标应用与位置**：在记事本、浏览器输入框、微信草稿框、VS Code 编辑器分别测试。面板期间目标光标、选区、输入法应保留。无系统插入光标时应在鼠标附近显示。测试双屏、负坐标、125%/150% DPI、屏幕四角。面板显示时切到其他窗口，确认不误粘贴到新窗口。
6. **主窗口**：连续 20 次快速粘贴后，Alt+V 和托盘仍能呼出主窗口；另测主窗口最小化及被其他窗口覆盖时能恢复。激活目标后 Alt+V，分别回车与双击粘贴。复制带 HTML/RTF 的文本，普通粘贴保留目标支持的样式，Shift+Enter 只有文本。无纯文本格式的图片/文件记录应提示，不能发送空内容。
7. **真实文件粘贴**：在项目 .test-data/manual/source 放两个可丢弃文件，复制后再复制文本以覆盖剪贴板。从历史选文件并粘贴到 .test-data/manual/destination，应生成真实文件。用测试副本重复剪切并确认移动语义。删除或改名源文件后粘贴应提示失效。微信等聊天应用只检查附件草稿，不发送消息。
8. **图片和富文本**：图片及从资源管理器复制的图片文件都应有大预览；检查旧记录缩略图回退、长文本截断、文件存在状态和读取失败占位。从截图工具、画图、浏览器和透明 PNG 来源复制，检查预览及粘贴的透明度。用受控测试程序分别提供 PNG、DIBV5、DIB 和多格式剪贴板。损坏的优先格式应尝试下一种，全部失败显示格式与尺寸占位。检查 HTML/RTF/文本共存的还原。
9. **隐私与自写回**：受控程序提供合成内容，设置 ExcludeClipboardContentFromMonitorProcessing 或 CanIncludeInClipboardHistory DWORD 0，历史不增加；设为 1 的普通内容可记录。重复粘贴已有记录不能产生重复条目。不要用真实密码测试。
10. **权限与异常**：向管理员终端粘贴，普通权限原型应报错而不发生卡键。粘贴前关闭目标、切换前台，或由测试进程短暂占用剪贴板，检查错误以不抢焦点的面板显示约 2 秒并自动隐藏，随后能继续复制和输入；提示期间重新呼出面板，不应被旧期限关闭。
11. **加密与重启**：退出并重启，应恢复历史及缩略图。仅在隔离目录检查数据库头不是明文 SQLite、blob 不含明文 PNG 头。备份完整测试数据后，用其他 Windows 账户打开副本，DPAPI 应失败。不要删除或替换真实历史密钥。
12. **性能与稳定性**：普通办公负载下记录面板延迟、快速连按、大图采集时的输入响应。高负载渲染或游戏期间不启动重型并发测试。报告包括应用版本、DPI、按键顺序和结果，不含敏感剪贴板内容。

设置 CLIPBOARD_M0_DATA_DIR 为本项目 .test-data 下的绝对路径，可隔离真实历史。CLIPBOARD_M0_SMOKE 仅在值为 1、true 或 yes（精确匹配）时启用；0、空值、其他值或未设置均不启用。冒烟模式禁用监听、钩子、托盘和粘贴；两个前端报告就绪后自动退出并写 smoke-result.json。冒烟结果同时要求缩略图与大预览协议加载成功。该模式只验证启动和协议加载，不代表视觉或真实粘贴验收。


## 可选诊断日志

在项目根目录的 PowerShell 中，仅为本次启动设置：

~~~powershell
$env:CLIPBOARD_M0_DATA_DIR = Join-Path (Get-Location).Path '.test-data\manual'
$env:CLIPBOARD_M0_DIAG = '1'
. .\scripts\env.ps1
npm run tauri dev
~~~

日志追加到数据目录的 `diag.log`。去掉 `CLIPBOARD_M0_DIAG` 或设为 `0` 后重新启动即可关闭；默认不记录。不要删除 `.test-data\manual` 或其中密钥、历史。分享前检查窗口类名是否涉及应用身份信息。

- 会记录：会话 Unix 毫秒时间与相对毫秒时间；限定时段内按键的按下/抬起、是否注入、是否吞掉、Open/Next/Previous/Commit/Cancel 动作；粘贴前后 8 个修饰键的异步与物理状态；前台窗口类名和 HKL；每次 SendInput 的请求数量及实际返回数量；Toggle 的可见/最小化/前台状态与恢复、显示、提升、set_focus、SetForegroundWindow 结果。
- 按键记录只在主窗口或快速面板打开期间及关闭后 2 秒内启用。只保留修饰键、V、方向键、Esc 的虚拟键码，其他键（包括掩码 0xE8）一律写 `other`；普通字符的键码不会进入日志消息队列。快速面板以钩子 Open/Commit/Cancel 时刻界定，确保掩码和提交后的输入能被观察。
- 不记录剪贴板文本、图片、文件路径、窗口标题、输入字符或密钥。日志为诊断元数据明文，不属于加密历史；HKL 只能反映键盘布局，不能单独证明输入法中英文模式没有变化。
- 钩子不做 IO、堆分配或同步等待，仅向日志线程 PostThreadMessage 投递脱敏定长数据。日志拥堵时丢弃并累计 `dropped`，不阻塞键盘；非钩子日志也使用有界队列。
- 验证输入法时分别选择微软拼音的中文/英文模式、不同键盘布局，交替先松 Ctrl/Shift、同时松键、Esc 取消，确认模式和布局均不切换。若出现异常，保留发生时间、按键顺序及相邻日志；不要仅凭推测认定 TSF 根因。


## 本轮严重问题修复验证（2026-11-10）

- `npm run build`、`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings` 全部通过；`cargo test` 当前共 130 项通过、0 失败。每次 cargo 命令前均在 PowerShell 加载 `scripts/env.ps1`，Rust 命令均指定 `--manifest-path src-tauri/Cargo.toml`。
- `cargo build --features custom-protocol` 成功；保留既有 vendored OpenSSL 缺少 `ossl_static.pdb` 的 LNK4099 非阻断警告。
- `CLIPBOARD_M0_SMOKE=1`、`CLIPBOARD_M0_DIAG=1` 在新建隔离目录启动，退出码 0。两个前端、缩略图与大预览协议均就绪；diag.log 已生成；监听器和钩子关闭。证据：`.tools/m0-fix-smoke-summary.json`。
- 使用独立 Edge 无头进程、合成 IPC 数据检查图片、图片文件、旧记录、加载失败占位、长文本、窄窗口和快速面板；7 项断言通过，另已检查大图、窄窗口和错误占位的压缩截图。此项验证前端渲染，不能证明原生焦点和真实粘贴。证据：`.tools/m0-ui/result.json` 与同目录图片。
- 新增/替换测试覆盖释放等待、掩码事件、干净 Ctrl+V、Toggle 八种组合、日志脱敏及时间窗、新数据目录日志追加、Unicode 截断、文件存在性、预览加密/登记/权限/重启、旧记录与图片文件失败路径。旧 reconciliation 测试已按新行为替换。
- 根因判断：旧代码重按修饰键和六轮修复、吞掉 V 且未发送掩码、Toggle 只判断可见性、缺少大图及文件图片预览均已由代码核实；TSF 卡住及输入法切换的实际因果仍需人工日志证实。未操作真实键鼠，未将真实桌面复测标为通过。
- 未安装软件、未 commit、未更改系统环境变量或 env.ps1 的 Perl 设置；未删除 `.test-data/manual`。原始修改文件备份在 `.test-data/fix-m0-backup`。

## 此前验证记录（2026-11-09）

本轮修复 D-1、D-2、D-3、D-4、D-5、D-8、D-9。每次 cargo 命令前均重新加载 scripts/env.ps1，未安装软件、未提交、未修改系统环境变量。应用验证只使用 .test-data 下的绝对路径。

| 检查 | 结果 | 证据 |
|---|---|---|
| npm run build | 通过：TypeScript 与 Vite 生产构建 | .tools/fix-frontend.log |
| cargo fmt --manifest-path src-tauri/Cargo.toml --check | 通过，无格式差异 | .tools/fix-fmt.log |
| cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings | 通过 | .tools/fix-clippy.log |
| cargo test --manifest-path src-tauri/Cargo.toml -- --test-threads=2 | 108 项通过，0 失败（原 96 项，新增 12 项） | .tools/fix-test.log |
| cargo build --manifest-path src-tauri/Cargo.toml --features custom-protocol | 通过，嵌入生产前端，用于无开发服务器的冒烟检查 | .tools/fix-build.log |
| CLIPBOARD_M0_SMOKE=1 隐藏启动 | 退出码 0；两个前端与缩略图协议全部就绪；监听和钩子关闭 | .tools/fix-smoke-summary.json、.tools/fix-smoke.stderr.log |
| CLIPBOARD_M0_SMOKE=0 判断 | 单元测试确认不进入冒烟模式；未以该值启动应用 | smoke_only_accepts_explicit_enabled_values |
| 三类数据启动失败 | 均退出 1，明确中文原因并提示不要删除数据；key.bin 未改写；错密钥后的数据库副本可用原密钥重新打开 | .tools/fix-startup-summary.json、.tools/verify-fixes.ps1 |
| 真实应用粘贴、焦点、错误提示视觉效果 | 未执行交互验收；本轮验证状态逻辑、构建和隔离启动 | 未接管键鼠，未发送真实粘贴按键 |

新增或修改的回归测试：

- D-1：runtime.rs 中 error_notice_expires_once_after_two_seconds、new_error_replaces_old_deadline_and_open_cancels_timeout，覆盖约 2 秒过期、连续错误续期、重新打开后取消旧期限。
- D-2：clipboard.rs 中 failed_format_keeps_other_formats、all_failed_or_only_metadata_does_not_record、total_limit_rejects_entire_record，覆盖部分成功、全部失败、仅元数据及总量边界；通过注入读取结果测试，不额外分配大块内存。
- D-3：tests/hook_machine.rs 将 trigger_repeat_cycles_selection 改为 trigger_repeat_does_not_advance_selection，同时覆盖首次呼出后长按、松开再按与成对吞键；原方向键重复测试保持通过。
- D-4：lib.rs 中 smoke_only_accepts_explicit_enabled_values，覆盖 1/true/yes、0/false/no、空值、未设置及其他值。
- D-5：clipboard.rs 中 short_drop_effect_is_discarded_and_files_default_to_copy、valid_drop_effect_retains_semantics_and_trims_padding、system_tests::stored_short_drop_effect_is_not_written_back，覆盖 0–3 字节丢弃、默认复制、有效复制/剪切值和旧历史写回。
- D-8：platform.rs 中 shell_blacklist_includes_popup_menu_and_all_shell_classes，覆盖 #32768 与原有外壳类名。
- D-9：platform.rs 新增 dpapi_failure_has_actionable_message_and_preserves_file、invalid_key_length_is_distinct_and_preserves_file；storage/mod.rs 的 encrypted_persistence_and_dedup 增加错密钥错误类别及中文提示断言，并保留使用正确密钥重开的验证。

现有 system_tests 会直接覆盖并清空系统剪贴板；它们未发送按键，也未向真实应用粘贴。D-1 的人工验收还需确认错误面板不抢焦点、约 2 秒隐藏，以及提示期间重新呼出面板不会被旧期限关闭。

已观察到的非阻断诊断：MSVC 链接器报告 vendored OpenSSL 缺失 ossl_static.pdb（LNK4099），构建成功但相关原生库缺少可用调试符号；隔离 WebView2 退出时出现一次窗口类注销错误 1412，应用退出码为 0。尚未确认后一诊断在正常交互退出时是否复现。

SQLCipher 错密钥单元测试会产生预期的 HMAC 验证失败诊断；该测试本身通过，日志不包含剪贴板内容或密钥。
