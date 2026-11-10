# Windows 剪贴板管理器 M0

Tauri 2 + Rust + Vue 3 的 Windows 技术验证原型。界面启动时隐藏，通过托盘或 Alt+V 打开，不要求管理员权限。当前已完成 M1 / T1 存储层演进与 T2 删除、自动清理，仍不提供安装包。

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

dev 配置为本 crate `opt-level = 1`、依赖 `[profile.dev.package."*"] opt-level = 3`，继续保留 `debug = 1`。首次切换会重编译依赖，耗时明显多于只重编本 crate；后续修改本 crate 复用依赖缓存，仍支持调试，但优化可能使逐行单步和局部变量观察不如 opt-level 0 直接。构建实测见下方“大图粘贴修复验证记录”。

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
  hook.rs                  纯状态机、低级键盘钩子、修饰键状态校正
  clipboard.rs             Win32 采集、隐私过滤、多格式写回
  paster.rs                代号与阶段编排、目标校验、焦点恢复、可取消等待、Ctrl+V
  paste_bench.rs           显式运行的合成大图基准，使用私有窗口站剪贴板
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

Clip 保留同一记录的全部受支持格式，hash 对排序后的格式名称、长度和内容计算哈希。Storage::save/load/list_page/touch 是存储接口，list 保留为第一页兼容接口；成功粘贴只按已有 id 更新 touched 和 used_at，不读取 blob。Machine::step_with_logical / corrected_modifiers 是不调用 Win32 的键盘状态机及修饰键校正逻辑，release_decision 是等待修饰键释放的纯逻辑，toggle_decision 是主窗口显隐决策的纯逻辑，thumbnail::generate 可独立测试。

## 已实现范围

- M1 / T2：主窗口单选/多选删除、Delete 键、HTML 右键菜单（删除/粘贴）和批量确认条；删除后重建已加载分页并选择后继。启动扫描孤儿 blob；默认 1000 条 / 1 GiB / 30 天自动清理；共享 blob 三处引用全部解除后才回收；启动及大批删除后按条件 VACUUM。列表与菜单已拆为独立组件。

- M1 / T1–T2：SQLCipher schema 版本 3、逐版事务迁移、迁移前加密备份和失败回滚；新增真实时间、大小、收藏/置顶/分组/来源/敏感字段与搜索文本，创建 groups/settings 表。主窗口分页并保持选中项和详情，图片采集先规范化尾部再哈希。最终建表语句和迁移说明见 [M1 存储层说明](docs/M1存储层说明.md)，人工复测见 [M1 人工验收清单](docs/M1人工验收清单.md)。

- Ctrl+Shift+V 打开快速面板，默认第二条，只有一条时选第一条。V 每次独立按下前进一格，按住 V 的自动重复不前进；↓/↑ 向后/向前且允许自动重复、循环选择，Esc 取消。必需修饰键组合首次不再满足时提交，例如先松开 Shift。提交后 worker 先加载记录、校验文件源路径并写回剪贴板，再每 5ms 检查 8 个修饰键的 GetAsyncKeyState 逻辑状态；全部释放后才准备目标并粘贴，从开始等待起最多 1 秒，只有当前代号确实超时才提示“请松开修饰键后重试”。正常粘贴仅发送 Ctrl↓、V↓、V↑、Ctrl↑，不重新按下修饰键。主窗口路径与 inject 前的最终检查使用同一逻辑；PHYSICAL 仅供诊断，不再阻塞粘贴。钩子逐事件采样并清除已逻辑释放的陈旧记录，再叠加当前真实事件；Open/Commit 使用校正后的状态。
- 面板预创建并隐藏，设置 NOACTIVATE / TOPMOST / TOOLWINDOW，以不激活方式显示，忽略鼠标输入。没有使用 AttachThreadInput。优先定位系统插入光标，失败则使用鼠标位置，并限制在屏幕工作区。
- Open 到达 broker 后立即用 SendInput 发送未分配虚拟键 0xE8 的按下/抬起作为掩码，降低纯 Ctrl+Shift / Shift 被系统或输入法解释为切换命令的风险；注入事件本身不更新对应键的真实按下记录，也不触发面板动作；其他键的陈旧状态仍可校正。实际输入法兼容性须人工复测。
- 键盘钩子拥有独立消息循环。回调只更新固定大小状态、读写原子变量和 PostThreadMessage，不执行数据库、图片解码或 UI 调用。
- 面板打开时固定最多九条的列表快照，选中状态由 Rust 事件推送。新采集记录不会改变当前选择。粘贴失败或队列繁忙时，以 NOACTIVATE 快速面板显示错误约 2 秒，随后清除提示并隐藏；仅错误提示时按内容高度缩小窗口，不显示空白列表，重新选择时恢复原尺寸。重新开始选择会取消旧提示的关闭期限。
- Alt+V 仅在主窗口可见、未最小化且位于前台时隐藏；其他情况恢复最小化、显示、提升窗口并请求焦点。托盘采用相同决策。回车或双击粘贴，Shift+Enter 只写回已有纯文本格式；无纯文本则返回错误。关闭按钮和 Esc 隐藏主窗口，托盘菜单退出应用。
- 采集 CF_UNICODETEXT、HTML Format、Rich Text Format、PNG、CF_DIBV5、CF_DIB、CF_HDROP 和 Preferred DropEffect。文件规范化为 UTF-16 DROPFILES，缺失或不足 4 字节的 DropEffect 默认复制，写回时重新构造文件格式。
- 受支持的单个格式读取失败时，只记录固定格式名并跳过该格式；其他成功格式仍保存；正文格式全部失败则不记录，并提示“剪贴板格式无效或数据不可读，本次复制未记录”。孤立的可选 DropEffect 元数据仍静默丢弃。总大小超限仍拒绝整条记录；隐私标记不可读时保持不记录。
- 自写回通过私有标记和序列号识别，成功粘贴后将原记录置顶。监控排除标记直接忽略；CanIncludeInClipboardHistory 为零或格式损坏时忽略。日志不记录内容。
- 按 PNG、DIBV5、DIB 的顺序尝试解码；处理 32 位 DIBV5 位域透明度。失败显示格式、尺寸或“尺寸未知”，不会留下空白。
- SQLCipher 整库加密，随机 32 字节主密钥由当前用户 DPAPI 保护。原图、缩略图、预览图及超过 64 KiB 的格式数据经 HKDF 派生密钥和 XChaCha20-Poly1305 加密。对象名为内容哈希，并绑定到认证数据。
- 主窗口显示选中项预览：图片额外保存最长边不超过 800px 的 PNG（加密落盘），旧记录没有 preview 时回退到缩略图；文本显示前 20000 个 Unicode 字符；文件显示完整路径及当前存在状态，可用“刷新状态”重新检查。
- 新文件记录的第一个文件若为 png/jpg/jpeg/gif/bmp/webp 且不超过 50 MiB，会生成缩略图和预览。读取或解码失败保留记录并显示具体占位。预览是保存时快照，不随源文件修改自动更新。
- 缩略图通过自定义协议在内存中解密；协议只允许读取数据库 items.thumbnail 或 items.preview 独立列登记的哈希，不开放任意文件或未登记对象，响应禁止缓存。

正常运行数据位于 %APPDATA%\ClipboardM0：key.bin、history.db、data/blobs/ 和 WebView 数据目录。不要单独删除 key.bin；存在数据库却缺少密钥时会拒绝生成替代密钥。启动时分别提示 DPAPI 解密失败（可能更换了 Windows 账户或电脑）、key.bin 长度异常和数据库密钥不匹配，并提醒不要删除数据。数据库损坏也可能产生与错密钥相同的 SQLCipher 错误，提示不会武断地排除损坏。

- 新意图优先：快速面板 Open 在钩子中递增 AtomicU64 代号，主窗口 Toggle 决定显示时也递增。消息及任务保留原代号；过期任务在加载前、各阶段之间、等待期间和注入前静默放弃，旧任务报错在控制线程再次按代号过滤。已经写入的剪贴板可保留；成功后用单条 UPDATE 置顶，不重新哈希。
- 写回只有 CF_DIBV5 的记录时只提供 CF_DIBV5，由 Windows 自动合成 CF_DIB / CF_BITMAP。旧记录同时具有两种 DIB 时沿用旧行为：仅当同时存在 PNG + DIBV5 时省略 DIB。普通格式直接从解密缓冲区复制到 HGLOBAL，文件列表仍重新编码。

## 未实现与已知限制

- **恢复风险：从 `history.db.bak-v*` 恢复旧库后，旧库引用的 blob 可能已经被回收。**迁移备份只含数据库；可恢复备份需在应用退出后同时保存 `history.db`、`key.bin` 和整个 `data/blobs`。不要用旧库覆盖正在使用的数据目录。
- 清理中置顶/收藏/分组记录不计数也不计空间；到期删除突破这些保护，但 touched 最新一条始终保留，可能使上限无法满足。size 是格式逻辑字节，不等同物理磁盘占用。手工删除不受这些保护限制。
- VACUUM 持有 storage 锁，可能延后采集保存、粘贴读取和预览；监听/钩子线程不持该锁。维护期间连续剪贴板变化可能合并。未测大库/高负载最坏延迟，不能承诺零阻塞。同一数据目录不支持多个实例同时执行 blob 写入和回收。

- 尚未实现 M1 的搜索、分组/收藏操作界面、设置持久化、开机自启、应用忽略名单及敏感规则。T2 已支持保留策略和 expires_at 到期清理入口，T7 再接入识别及到期调度。
- 自动清理仅在启动及新插入后的节流检查执行，没有到期定时器；主窗口每页 100 条滚动加载，快速面板缓存第一页 200 条，维持原有最多九条显示快照。快捷键是源码常量，无设置界面；快速组合支持配置 Ctrl/Shift 与触发键，尚未开放 Alt/Win 组合配置；掩码机制已独立实现。
- 单个对象及单次采集总量上限 512 MiB（536,870,912 字节），最多 4096 个文件；图片解码有尺寸和内存上限，超过限制会提示。采集超限和格式无效分别提示，不再与占用混用。
- 剪贴板占用最多退避 50/100/200/400/800ms（累计等待 1.55 秒，不含系统调用耗时）；耗尽只提示一次“剪贴板被其他程序长时间占用，本次复制未记录”。同序列失败后不再补采，需要新的复制变化；中途换序列放弃旧轮次，自写回无需打开剪贴板。退避发生在共享 worker，可能短暂延后其后排队的粘贴任务，不阻塞 broker/UI 线程。
- Alt+V 在 broker 立即发送 0xE8 掩码；主窗口显示不再调用 tao 的 set_focus，保留恢复、显示、提升及原生 SetForegroundWindow。显示流程开始至完成后 300ms 仅拦截键盘 Alt 的 SC_KEYMENU（lParam=0），Alt+空格和鼠标系统菜单仍可用；这段短时间内主动单按 Alt 也会被抑制。Windows 仍可能拒绝前台激活，结果会写日志；真实最小化恢复、遮挡恢复和高负载时序仍须人工验证。
- 文件记录保存路径和复制/剪切语义，不保存文件快照。源文件失效时阻止粘贴并报错，详情在选中、窗口获取焦点或点击“刷新状态”时检查存在性；不进行持续文件监控。剪切记录使用后可能因源路径消失而失效。
- D-6 已处理 PNG / DIB 可确定的尾部填充；文件记录新增路径集合与复制/剪切语义的去重键，忽略附加 DIB/HTML；非文件记录跨编码、格式集合差异不做像素级去重，异常头保留原字节，旧哈希不重写。同一图片以“文件”和“位图”两种形式主动复制时仍是两条记录（满足下述退出固化条件的更新除外）；迁移保留已有重复项，可由用户手工删除。D-10 已通过启动扫描处理；仍未处理 D-7 及 R 系列风险点。
- 退出固化识别（`owner_flush`）仅比较上一次成功保存的内存上下文：新内容无 owner（或确认进程已退出）、格式集合为子集、上次有真实 owner 且不是同一进程窗口、图像尺寸与 256 个 RGB 抽样全部相同（无图时比较完整 UTF-16 文本哈希），才跳过保存与 touch。CF_DIBV5/CF_BITMAP 按规则补入系统合成集合；未知 owner、格式枚举失败、图片无法解析都不跳过。上下文关联成功保存的实际记录 id（去重时为已有记录）；手动删除或自动清理只有实际删除该记录才清空上下文，未删除记录或只删除其他记录时保留，重启不恢复。抽样不是整图等价证明：只在未采样位置或 alpha 上变化的图像，满足其它条件时仍可能被误跳过；PNG 独有内容需要受内存上限约束的解码，DIB 仅抽样。它不修改原有 dedup_key，也不等于通用跨编码图片去重。
- 不抓取 HTML 远程图片，不对 HEIC、SVG 生成预览，GIF/WebP 仅预览静态帧。支持的剪贴板图片解码失败仍保留原始数据并显示占位。
- 目标应用是否接受富文本、透明图片和文件取决于其格式支持。Word 专有格式、CF_BITMAP、OLE 对象不在本阶段范围。
- 普通权限进程遇到管理员目标会报错，不提升权限或绕过 UIPI。窗口关闭、前台变化或系统拒绝恢复焦点时取消粘贴。
- 本轮未采用延迟渲染。当前由 worker 分配和写回，listener 消息窗口持有所有权；若改为 64 KiB 以上格式使用 `SetClipboardData(fmt, NULL)`，必须同步设计内存缓存、`WM_RENDERFORMAT` / `WM_RENDERALLFORMATS` / `WM_DESTROYCLIPBOARD` 生命周期、退出前完整渲染及自写回识别，并避免 worker 持锁时同步窗口消息造成互等。本轮优先优化已测得的解密瓶颈和拷贝，延期实施跨线程所有权改造。
- 取消是阶段间协作式取消，不中断已在执行的解密、系统写回或焦点准备。检查与 SendInput 之间仍有极短竞态；按键已成功发出后无法撤销目标应用的异步粘贴，大图解码/显示耗时取决于目标应用。
- 剪贴板写回和目标程序消费之间没有事务保证。写回成功但按键发送失败时，剪贴板仍为选中记录。SendInput 成功也不表示目标应用一定接受内容。
- 钩子丢失抬起事件后的陈旧修饰键记录会在后续事件中校正，粘贴等待不再依赖该记录；若没有后续事件，状态机不会自行触发 Commit。暂无生产级钩子健康监测和掉钩自动恢复。16ms 显示延迟、多屏 DPI、远程桌面、休眠、输入法和修饰键竞态需人工实测。
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

- 会记录：会话 Unix 毫秒时间与相对毫秒时间；限定时段内按键的按下/抬起、是否注入、是否吞掉、Open/Next/Previous/Commit/Cancel 动作；粘贴前后 8 个修饰键的异步与物理状态；前台窗口类名和 HKL；每次 SendInput 的请求数量及实际返回数量；Toggle 的可见/最小化/前台状态与恢复、显示、提升、SetForegroundWindow 结果（不再调用 set_focus），以及 Toggle 显示流程前后的 8 个修饰键快照（`toggle stage=before_show/after_show`）。
- 采集每轮记录 capture sequence/current/self_write/busy/retries/result/reason，result 为 ok/skipped/busy/limit/invalid，不记录内容；retries 为实际退避次数，busy 表示本轮是否遇到占用，superseded 表示新序列留给下一轮。Alt+V 记录 hotkey mask_ok 和底层 SendInput 数量，以及 toggle stage=hotkey_after_mask；短时拦截记录 toggle alt_menu_suppressed=true。
- 每次采集成功打开剪贴板后、任何数据读取前，记录 `capture formats sequence=N owner_pid=P owner_exe=xxx.exe list=[id:name, ...]`。清单只调用 `EnumClipboardFormats` 与格式名称查询，不调用 `GetClipboardData`；标准格式显示 `CF_` 常量名，注册格式使用 `GetClipboardFormatNameW`，未知名称为 `-`。来源通过 `GetClipboardOwner`、窗口 PID 与 `QueryFullProcessImageNameW` 查询，只保留 exe 文件名；无法取得的 PID 或程序名分别写 `-`，不记录完整程序路径。
- 采集结束（含提前返回）输出实际读取的 `capture read format=name bytes=N`，字节数是归一化前成功复制的数据长度；CF_HDROP 是现有路径读取及重编码后的字节数。可用格式的 `GetClipboardData` 返回 NULL、全局数据大小为零或文件列表为空时输出 `capture empty format=name`。隐私/自写回标记若实际读取，也只记录名称与字节数。不可用、因冗余/限额跳过、尚未尝试或其他读取错误的格式，不会被伪记为 empty；合成的默认 Preferred DropEffect 不算实际读取。
- 监听窗口每次收到 `WM_CLIPBOARDUPDATE`，在采集合并判断前记录 `clipboard_update sequence=N owner_exe=xxx.exe`；此处不开剪贴板、不读取数据。程序名是处理通知当时的 owner 快照；快速连续更新可能已推进到同一最新序列，owner 也可能退出，因此不能把它当成每次写入者的历史记录。格式清单和 read/empty 可帮助排查延迟渲染，但本身不能证明发生了 `SetClipboardData(NULL)` 或 `WM_RENDERALLFORMATS`。日志专用的格式名查询与采集明细只在诊断开启时执行；退出固化判断需要的格式 id、owner 与指纹在诊断关闭时仍会采集；格式名中的控制字符及清单分隔符会转义。
- 按键记录只在主窗口或快速面板打开期间及关闭后 2 秒内启用。只保留修饰键、V、方向键、Esc 的虚拟键码，其他键（包括掩码 0xE8）一律写 `other`；普通字符的键码不会进入日志消息队列。快速面板以钩子 Open/Commit/Cancel 时刻界定，确保掩码和提交后的输入能被观察。
- 每个执行到的粘贴阶段各记录一次 `paste generation=… phase=load/write/wait/prepare/inject/touch elapsed_ms=… status=ok/error/superseded`，单位毫秒，保留三位小数。过期任务未执行的阶段不会伪造耗时；`paste_superseded` 表示静默退出。`before_wait` 现在位于 load/write 之后，不能再与旧日志同名快照直接对比关键路径。load 阶段列出受支持格式名与原始字节数，`clipboard_write` 列出实际写入格式名与字节数，可确认 DIB 被省略。inject 耗时包含准备期间重新按键时的最终可取消等待。
- 不记录剪贴板文本、图片、文件路径、窗口标题、输入字符或密钥。日志为诊断元数据明文，不属于加密历史；HKL 只能反映键盘布局，不能单独证明输入法中英文模式没有变化。
- 钩子不做 IO、堆分配或同步等待，仅向日志线程 PostThreadMessage 投递脱敏定长数据。日志拥堵时丢弃并累计 `dropped`，不阻塞键盘；非钩子日志也使用有界队列。
- 验证输入法时分别选择微软拼音的中文/英文模式、不同键盘布局，交替先松 Ctrl/Shift、同时松键、Esc 取消，确认模式和布局均不切换。若出现异常，保留发生时间、按键顺序及相邻日志；不要仅凭推测认定 TSF 根因。
- 新内容无 owner（`is_ownerless`）但未跳过时输出 `capture owner_flush_miss sequence=N reason=...`，按判定顺序记录首个未满足条件：`no_previous`（无上次成功上下文）、`previous_not_real`（上次不是真实 owner）、`same_owner`（同一进程窗口）、`formats_not_subset`（格式枚举失败或不满足子集/系统合成规则）、`fingerprint_missing`（任一指纹缺失）、`fingerprint_mismatch`（指纹不一致）。此行不包含内容、记录 id 或指纹；有 owner 或来源未知时不输出此类 miss。
- 命中退出固化时输出 `capture sequence=N result=skipped reason=owner_flush`；该序列已经由原有采集策略标为已处理，不保存、不 touch，也不以无 owner 结果替换上次成功上下文。此前同序列的 `result=ok reason=read` 仅表示读取成功，是否保存以随后 worker 的 `owner_flush` 判断为准。日志不包含文本、像素或指纹；来源查询复用于识别，进程权限不足时按未知处理。

## 采集占用误报与 Alt+V 系统菜单修复（2026-11-10）

- 根因核实：修复前 open 为 8 次尝试、每次失败等待 8ms，capture 在打开后才识别 SELF_SEQUENCE；worker 只在成功时推进 LAST_SEQUENCE，250ms 定时器会再次排队。以上代码路径已确认；“QQ 持锁读取约 30MB”的实际时长及这次截图的具体消息顺序未进行现场测量，仍属待复测的触发条件。
- 本地 Cargo.lock 锁定 tao 0.37.2；缓存源码 D:/DevTools/cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tao-0.37.2/src/platform_impl/windows/window.rs:139 的 set_focus 调用 force_window_active，后者在 1514 行先尝试 SetForegroundWindow，失败后在 1524–1540 行构造并发送左 Alt 按下/抬起，再次激活。这是条件回退，不代表每次 set_focus 都注入；window_state.rs 的显示/恢复路径使用 ShowWindow，不走该 Alt 回退。
- 采集新增 Busy/Limit/Invalid 类型化结果、worker 退避和序列去重；在打开前、取得锁后、读取后及等待前后核对序列。成功、跳过和最终失败均确认对应序列，等待期间的新变化在本轮结束后补排。保留隐私标记、所有权标记、有效格式容错与 DropEffect 默认复制行为。
- Alt+V 的 WM_HOTKEY 在发送 Toggle 之前立即掩码并记录结果；保留 Toggle 原有“仅当前台、可见且非最小化时隐藏”的判定。移除 set_focus 的 Alt 回退，保留 unminimize/show/SetWindowPos/SetForegroundWindow；主线程安装主窗口子类化，仅在短时窗吞掉指定 SC_KEYMENU，销毁时移除，其余消息继续转发。
- 新增 15 项纯逻辑测试：13 项采集测试覆盖自写回不读取、占用后成功、等待前/中换序列、等待中变自写回、耗尽仅报告一次、最后一次失败再核对、成功/跳过/失败去重、序列回绕、读取中换序列、所有权确认和日志不含内容；2 项菜单保护测试覆盖 300ms 边界、低位掩码、Alt+空格、鼠标及其他命令不被拦截。更新正文格式全部失败的既有测试预期；原短 DropEffect 回归测试保留。
- 本轮不接管键鼠、不向 QQ 等应用实际粘贴；“连续粘贴大图 10 次”和“Alt+V 呼出 20 次无系统菜单”仍待人工执行，详见人工清单末节。未安装软件、未改系统/用户环境变量或 env.ps1、未 commit，未修改或删除 .test-data/manual。已有未提交改动保持；修改前相关文件备份在 .tools/capture-menu-backup，仅供对照，不应覆盖后续工作。

| 检查 | 结果 | 证据 |
|---|---|---|
| cargo fmt --manifest-path src-tauri/Cargo.toml --check | 通过 | .tools/capture-menu-fmt.log |
| cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings | 通过 | .tools/capture-menu-clippy.log |
| cargo test --manifest-path src-tauri/Cargo.toml -- --test-threads=2 | **172 项通过、0 失败、1 项基准默认忽略**（基线 157，新添 15） | .tools/capture-menu-test.log |
| npm run build | TypeScript 与 Vite 生产构建通过 | .tools/capture-menu-frontend.log |
| git diff --check | 通过 | 无空白错误 |

每次 cargo 前均在 PowerShell 加载 scripts/env.ps1。测试使用已有私有窗口站，不覆盖桌面剪贴板；未运行 ignored 大图基准。既有 OpenSSL LNK4099 链接警告仍存在，错密钥测试的 SQLCipher HMAC 诊断属于预期输出。


## 大图粘贴修复验证记录（2026-11-10）

- 保留 HEAD adcc989 上已有的未提交修复，本轮未 commit、未安装软件、未修改环境配置文件或系统/用户环境变量；每次 cargo 前均加载 scripts/env.ps1。未写入或删除 .test-data/manual，未接管键鼠、未向 QQ/codeg 发送实际粘贴。
- 日志旧样本：before_wait 129052 → before_send 134629 为 5577 ms，发送 134635 → paste_complete 135567 为 932 ms。旧日志没有分阶段计时，这两个值不能拆成精确的 load/write/touch；也不能直接与下面不同图片的合成测试比较。
- 修复路径统一为 load → 文件源校验/write → 可取消 wait → prepare → 最终代号与修饰键检查/inject → touch → 列表刷新。新 Open/主窗口显示使旧代号失效；完成的粘贴只用已有 id 更新 touched。既有修饰键校正、掩码、正常成对 Ctrl+V 和 UI 修复均保留。

性能数据使用同一固定种子的 3200×3200 RGB 噪声图片：PNG **30,725,608 B（约 29.3 MiB）**、DIBV5 **40,960,124 B**、DIB **30,720,040 B**，存储格式合计 **102,405,772 B（约 97.7 MiB）**。这是“约 30MB 图片”在多个格式中的完整存储载荷；写回后保留 PNG + DIBV5，减少 **30,720,040 B** 的显式分配与写入，并去掉普通格式的中间克隆。

同机顺序执行，私有 Windows 窗口站真实 OpenClipboard/SetClipboardData；每种构建先预热 1 次，再取 5 次样本的中位数，文件缓存为热态。各列独立取中位数，合计列按每轮 load + write 实测，不包含样本生成、首次保存/缩略图、目标应用解码显示或按键注入。

| 构建/实现 | load（ms） | write（ms） | load + write（ms） | 置顶（ms） |
|---|---:|---:|---:|---:|
| 未优化 dev / 旧实现 | 9991.748 | 48.833 | 10040.582 | 1362.053 |
| 优化 dev / 新实现 | 112.028 | 25.228 | 137.622 | 0.650 |
| release / 新实现 | 96.336 | 25.651 | 122.093 | 0.722 |

- 未优化 dev 基线使用修改前生产实现及 opt-level 0；dev 测试 profile 继承新 dev 配置（本 crate 1 / 依赖 3），release 使用标准优化配置。dev 的 load + write 约快 **73.0 倍**，耗时减少 **98.6%**；这是合成基准收益，不等于 QQ/codeg 输入框的端到端耗时。
- load + write 的 5 次测量范围：未优化 dev 9931.021–10220.821 ms；优化 dev 135.520–142.852 ms；release 119.977–125.963 ms。release 首次编译约 6 分 58 秒；本轮无先前 release 构建缓存。
- 新 dev profile 首次切换重编译耗时 **10 分 16 秒**（构建+基准总计约 618.6 秒）；原未优化基线只增量编译测试目标用了 3.27 秒，不能作为全冷编译对照。优化依赖和原生依赖的初次构建成本显著增加，后续本 crate 修改会复用依赖缓存。
- 每次优化后基准结束，在计时外读取 CF_DIB，确认 Windows 能提供兼容格式；随后 capture 确认仍跳过自写回。该检查不替代 QQ/codeg 中透明度、视觉效果及真实连续粘贴的人工验收。
- **未实现延迟渲染**。评估理由见“已知限制”：当前线程分工需要一并解决缓存、同步回调、所有权变更、退出完整渲染与自写回识别；本轮优化已大幅缩短主要耗时，暂不扩大到这项生命周期改造。

| 检查 | 结果 | 证据 |
|---|---|---|
| cargo fmt --manifest-path src-tauri/Cargo.toml --check | 通过 | .tools/paste-fmt.log |
| cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings | 通过 | .tools/paste-clippy.log |
| cargo test --manifest-path src-tauri/Cargo.toml -- --test-threads=2 | **157 项通过、0 失败、1 项基准默认忽略** | .tools/paste-test.log |
| npm run build | TypeScript 检查和 Vite 生产构建通过 | .tools/paste-frontend.log |
| dev / release 大图基准 | 各 1 项显式测试通过，兼容 DIB 与自写回识别检查通过 | .tools/paste-bench-dev.log、paste-bench-release.log |
| git diff --check | 通过 | 无空白错误 |

既有 vendored OpenSSL 缺少 ossl_static.pdb 的 LNK4099 链接警告仍存在，不阻断构建；错密钥测试的 SQLCipher HMAC 诊断为预期输出，测试通过。

新增 **20 项自动测试**：代号/等待/阶段顺序 11 项（等待中换代立即静默退出、超时边界、采样中换代优先、队列过期、各阶段取消、失败不置顶、已注入后仍置顶），文件源预检 2 项，touch 3 项（排序、元数据/blob 不变、无需读取对象、未知 id 无副作用），格式选择 4 项（PNG+DIBV5 省略 DIB、缺格式时保留 DIB、纯文本与过滤）。另新增 1 项默认 ignored 的性能基准，dev/release 分别显式运行。原有系统剪贴板测试现已使用私有窗口站，不覆盖用户桌面剪贴板；本进程切换窗口站串行执行。

复现基准（一次只运行一个构建，避免同时运行其他重型测试）：

~~~powershell
. .\scripts\env.ps1
cargo test --manifest-path src-tauri/Cargo.toml large_image_benchmark --lib -- --ignored --nocapture --test-threads=1
. .\scripts\env.ps1
cargo test --manifest-path src-tauri/Cargo.toml --release large_image_benchmark --lib -- --ignored --nocapture --test-threads=1
~~~

原始样本在每次打印的 .test-data/paste-bench-*/timings.csv；日志为 .tools/paste-bench-baseline.log、paste-bench-dev.log、paste-bench-release.log，统计为 .tools/paste-bench-summary.json。未优化源码快照位于 .tools/paste-fix-backup（不覆盖当前工作区恢复旧版本）。人工清单 D 节已增加“大图粘贴后立即重新打开面板选择另一条”及主窗口/快速面板交替复测，仍待真人在 QQ/codeg 执行。


## 修饰键卡住修复验证记录（2026-11-10）

- 根因已由只读日志与代码确认：`.test-data/manual/diag.log:1303–1307` 显示左 Alt 按下、Toggle 显示后，出现 `A4:async=0,physical=1` 并在约 1 秒后取消粘贴；旧 `(logical | physical)` 判定会持续超时。tao 注入 Alt 或抬起丢失的具体来源尚未证实，新增显示前后快照供人工追踪。
- `release_decision` 已移除 physical 参数；等待和注入前检查都仅依据系统逻辑状态。钩子使用同一纯函数校正 Machine 与 PHYSICAL，不把逻辑按下补记为真实按下，注入事件不触发 Open/Commit。
- `cargo fmt --manifest-path src-tauri/Cargo.toml --check`、`cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`、`cargo test --manifest-path src-tauri/Cargo.toml -- --test-threads=2`、`npm run build` 全部通过。Rust 共 **137 项通过、0 失败**（基线 130 项，新增 7 项），每次 cargo 前均加载 `scripts/env.ps1`。证据：`.tools/modifier-fmt.log`、`.tools/modifier-clippy.log`、`.tools/modifier-test.log`、`.tools/modifier-frontend.log`。
- 新增 7 项钩子回归测试覆盖：抬起丢失后单按 V 不 Open、后续真实事件 Commit 且仅一次、8 个当前事件覆盖旧采样、清除陈旧状态但不补记注入键、注入 Alt 不打开/提交、注入抬起不覆盖真实当前键、左右双修饰键与成对吞键。同步更新 4 项释放等待测试，覆盖逻辑按下、全部释放、逻辑滞后和 1 秒边界。
- 独立 Edge 无头渲染 5 项检查通过：短提示、多行提示增高、超长提示限高滚动、重新打开列表恢复 530px、再次提示缩小。短提示 420×106、多行提示 420×178，已目视检查无空白列表或裁字。证据：`.tools/modifier-ui/result.json`、`short.jpg`、`long.jpg`；脚本为 `.tools/modifier-ui-check.mjs`。此项使用合成 IPC，不代表原生窗口缩放、焦点、多屏 DPI 或真实粘贴已验收。
- 已在人工验收清单加入“Alt+V 呼出并关闭主窗口若干次，再连续快速粘贴 10 次”的复测项，当前仍待人工执行。未接管键鼠、未安装软件、未 commit、未修改系统/用户环境变量，未写入或删除 `.test-data/manual`。既有系统剪贴板测试会写入合成内容并清空系统剪贴板，不发送按键。

## 此前严重问题修复验证（2026-11-10）

- `npm run build`、`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings` 全部通过；该轮 `cargo test` 共 130 项通过、0 失败。每次 cargo 命令前均在 PowerShell 加载 `scripts/env.ps1`，Rust 命令均指定 `--manifest-path src-tauri/Cargo.toml`。
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

此前验证时的 system_tests 会直接覆盖并清空系统剪贴板；它们未发送按键，也未向真实应用粘贴。D-1 的人工验收还需确认错误面板不抢焦点、约 2 秒隐藏，以及提示期间重新呼出面板不会被旧期限关闭。

已观察到的非阻断诊断：MSVC 链接器报告 vendored OpenSSL 缺失 ossl_static.pdb（LNK4099），构建成功但相关原生库缺少可用调试符号；隔离 WebView2 退出时出现一次窗口类注销错误 1412，应用退出码为 0。尚未确认后一诊断在正常交互退出时是否复现。

SQLCipher 错密钥单元测试会产生预期的 HMAC 验证失败诊断；该测试本身通过，日志不包含剪贴板内容或密钥。


## M1 / T1 验证记录（2026-11-10）

| 检查 | 实际结果 |
|---|---|
| `cargo fmt --check` | exit 0，无差异 |
| `cargo clippy --all-targets -- -D warnings` | exit 0 |
| `cargo test` | exit 0；**184 passed / 0 failed / 1 ignored**，较基线增加 12 项 |
| `npm run build` | exit 0；vue-tsc + Vite，16 modules transformed |
| `node scripts/check-m1-ui.mjs` | 8 项通过：首屏、滚动末页、深页选择保持、详情竞态、失败重试、过期分页、窄窗口布局、快速面板缓存 |

四项必需检查均在 `scripts/env.ps1` 环境下执行；Rust 测试使用 `RUST_TEST_THREADS=2`。完整日志位于 `.tools/m1-{fmt,clippy,test,build}.log`，浏览器结果位于 `.test-data/m1-ui/result.json`。链接阶段仍有已有 OpenSSL 静态库缺少 PDB 的 LNK4099 信息，测试成功；错密钥测试的 SQLCipher HMAC 诊断属于预期。没有新增依赖、安装软件、提交代码或修改 `.test-data/manual`。

新增测试覆盖真实加密 M0 schema、内联/外部文本、图片 blob 与可解码缩略图/预览、文件路径、回填字段、同密钥打开备份、重开不重复迁移、备份重名不覆盖、注入失败回滚、数据库占用、新旧 schema 一致、版本拒绝降级、分页/过滤/时间更新、serde 兼容和图片规范化。Win32 剪贴板测试继续使用私有窗口站；浏览器使用独立配置和合成 IPC 数据，未操作桌面键鼠。

T1 的 groups/settings 仅建表，其他元数据仅提供存储基础；FTS5、清理、设置业务、来源和敏感规则留给后续任务。列表不传输内部 search_text，快速面板仍从缓存读取。主窗口尚无虚拟列表，加载大量历史后内存和 DOM 成本会增长，实时变化通过事件重建已加载范围。迁移不重新哈希旧记录，因此旧图片与首次规范化记录可能并存。真实跨程序图片复制、原生焦点/输入法和粘贴未在本轮人工验收。


## M1 / T1 文件记录去重修复（2026-11-10）

新增 `Clip::dedup_key()`：文件按规范化路径集合和复制/剪切语义计算 SHA-256，其他记录继续使用内容 hash。`items` 升级到 schema v2，增加 `dedup_key TEXT` 和非唯一索引 `items_dedup_key`；沿用加密备份和逐版事务，支持 v0/v1 升级。回填只读取所需内联格式，无法计算时使用旧 id，旧重复项全部保留。

保存先按去重键选取 touched 最大的一条，只更新 touched / used_at，并在生成预览或写 blob 前返回；保留首次保存的格式。自写回检测使用剪贴板序列号和本进程所有权标记，不依赖 id 与 hash 相同，本次未改动该检测、粘贴代号、修饰键逻辑或不读取 blob 的 `Storage::touch`。

| 检查 | 本次实际结果 |
|---|---|
| `cargo fmt --check` | exit 0，无差异 |
| `cargo clippy --all-targets -- -D warnings` | exit 0，Finished dev profile，3.45s |
| `cargo test` | exit 0；**208 passed / 0 failed / 1 ignored**，较 196 passed 基线新增 12 项 |
| `npm run build` | exit 0；vue-tsc + Vite，16 modules transformed，510ms |

四项均先加载 `scripts/env.ps1`；Rust 测试并发为 2，日志为 `.tools/dedup-{fmt,clippy,test,build}.log`。新增测试覆盖键规范化/格式无关性、复制与剪切、损坏 HDROP 回退、双向保存保留首次格式且不新增 blob、v0/v1 回填与旧重复保留、最新 touched 命中、重开幂等以及 v2 失败回滚。已有 OpenSSL LNK4099 和错密钥测试的 SQLCipher HMAC 诊断不影响通过。

未读取、写入或删除 `.test-data/manual`，未安装软件、未提交代码。本次没有执行桌面/QQ 人工操作，复测步骤见 [M1 人工验收清单](docs/M1人工验收清单.md)。


## M1 / T2 删除与自动清理验证（2026-11-10）

schema 从 v2 升到 v3，仅新增 `formats_blob`、`items_touched`、`items_expiry`、`items_retention` 四个索引，沿用事务迁移与加密备份，不改 T1 去重逻辑或 M0 输入/粘贴行为。删除按事务删行、提交后回收文件，三处引用共同判断；回收失败进入 report 和诊断，留给下次启动扫描。

默认策略为 1000 条、1 GiB、30 天；runtime 保存可替换策略，T2 不读 settings。启动先扫描孤儿，再清理并初始化缓存。之后只有新插入记录才检查“至少 30 秒或已新增 20 条”，去重/touch 不触发；清理有删除随保存一并 refresh。启动空闲页严格超过 25%，或者单次实际删除超过 200 条时执行 VACUUM；启动清理已经整理则不重复。小型隔离测试验证了 SQLCipher + WAL + secure_delete=ON 下整理后数据库仍加密、重新打开可读。实际整理耗时可在诊断日志 `vacuum elapsed_ms` 查看；本轮未对大库进行延迟基准。

以下命令均从项目根目录执行，cargo 每次先加载 `scripts/env.ps1`，使用 `--manifest-path src-tauri/Cargo.toml`；测试限制 `RUST_TEST_THREADS=2`。

| 检查 | 实际结果 | 日志 |
|---|---|---|
| `cargo fmt --check` | exit 0，无格式差异 | `.tools/t2-fmt.log` |
| `cargo clippy --all-targets -- -D warnings` | exit 0；Finished dev profile，2.17s | `.tools/t2-clippy.log` |
| `cargo test` | exit 0；**233 passed / 0 failed / 1 ignored** | `.tools/t2-test.log` |
| `npm run build` | exit 0；vue-tsc + Vite，21 modules transformed，659ms | `.tools/t2-build.log` |
| `node scripts/check-m1-t2-ui.mjs` | exit 0；18 项离屏 UI 检查通过 | `.tools/t2-ui.log` |

相对 210 passed / 1 ignored 基线新增 23 项 Rust 测试：22 项存储维护测试和 1 项节流测试；既有测试只更新当前/未来 schema 版本断言，未删减。库测试 163 passed / 1 ignored，五组集成测试分别 22、14、4、12、18 passed。离屏 UI 检查覆盖原有 8 项和新增 10 项删除/多选流程，结果在 `.test-data/m1-t2-ui/result.json`；确认条压缩预览 `.test-data/m1-t2-ui/confirmation.jpg` 已完成布局检查。

没有安装依赖或软件，没有提交代码，没有接管键鼠，也没有读取、写入或清理 `.test-data/manual`。Rust 测试使用项目内独立数据，原有 Win32 测试使用私有窗口站；离屏 Edge 使用独立 profile 和合成 IPC。已有 OpenSSL 静态库 PDB 缺失会产生 LNK4099 信息，错密钥测试的 SQLCipher HMAC 日志属于预期。原生跨应用、输入法、焦点和实际采集时序仍按 [M1 人工验收清单](docs/M1人工验收清单.md) 待用户验证。

## blob 瘦身 A / B 验证（2026-11-10）

新对象按内容选择版本：PNG、缩略图、预览图和收益不足的载荷使用 CM01；其他 blob 先做 zstd 级别 3 压缩，再用 XChaCha20-Poly1305 加密，存为 CM02。仅在计入额外 8 字节头部后仍小于明文的 95% 时采用压缩。CM02 布局为“标记 4 + 明文长度 u64 LE 8 + nonce 24 + 压缩密文 + tag 16”，AAD 为 UTF-8 的 `CM02:` + id，再追加同一长度字段；认证通过后按声明长度限制解压，最高 512 MiB。旧 CM01 继续可读；已有同哈希文件不覆盖，id 与 size 均仍按明文计算。

AMD Ryzen 7 9800X3D、Windows、现有优化 test/dev 构建，单线程基准，各 7 次中位数。两个合成 DIBV5 均为 **31,457,404 字节（约 30 MiB）**：

| 样本 | 最终 blob | 压缩耗时 | 解压耗时 |
|---|---:|---:|---:|
| 渐变/色块/细线截图式样本 | 511,168 字节（1.625%，约 0.49 MiB） | 19.654 ms | 17.332 ms |
| 高噪声 RGB + 固定 alpha | 26,877,531 字节（85.441%，约 25.63 MiB） | 50.298 ms | 33.141 ms |

截图式样本完整 save 为 157.198 ms，热缓存完整 load 中位 18.027 ms（16.207–20.444 ms）。高噪声样本的内存认证解密 + 解压中位 71.525 ms（68.782–100.460 ms，不含文件 IO）。两类纯解压最高 63.765 ms，未超过约 80 ms，保留级别 3；不能把纯解压时间当成完整粘贴耗时。真实图片压缩率和目标程序显示耗时仍须人工复测。

采集已采用锁内查重、锁外预览/压缩/加密、锁内复查与提交；锁外不写 blob 文件，兼容并发删除和孤儿扫描。采集与粘贴仍共用串行 worker，不能抢占正在进行的采集。旧的双 DIB 图片再次复制可能多一条记录，同一规范化格式集合只发生一次；旧 CM01 不重压，目录随后续新采集、删除与清理逐步瘦身。

四项检查通过：cargo fmt --check、cargo clippy --all-targets -- -D warnings、cargo test（**274 passed / 0 failed / 1 ignored**，较基线新增 24 项）、npm run build（vue-tsc + Vite，21 modules）。日志为 .tools/blob-fmt.log、blob-clippy.log、blob-test.log、blob-build.log；7 次性能采样在 .tools/blob-bench.log。私有窗口站验证仅 DIBV5 时 Windows 合成 DIB / BITMAP，透明预览和新旧对象存储测试通过；画图、Word、QQ、微信以及大图主观延迟仍待人工验收。

完整格式、时间范围、内存与锁的限制见 [M1 存储层说明](docs/M1存储层说明.md)，操作项见 [M1 人工验收清单](docs/M1人工验收清单.md)。


## 存储瘦身 D / E：自适应预览与 CM03（2026-11-10）

新缩略图（256×160）与预览（800×800）在缩放前检查源图 alpha。不透明图片转 RGB8，同时编码质量 85 的 JPEG 和 Best/Adaptive PNG，取更小者；有透明像素的图片使用 RGBA8 PNG。纯色截图通常选择 PNG。`thumb://` 在原有哈希及数据库引用白名单检查后按魔数返回 `image/png` 或 `image/jpeg`，保留 `nosniff` 与 `no-store`；PNG/JPEG 均以 CM01 存储。前端 `PreviewImage.vue` 的普通 img 已支持两种 MIME，无需修改。旧 PNG 仍可读取，不转换已有对象。

24/32 位 BI_RGB 或 BI_BITFIELDS DIB 尝试固定 Up 像素预测，再以 zstd 级别 3 压缩。64 MiB 以内同时与普通 zstd 比较，只有比 CM02 更小且含额外头部仍节省超过 5% 才选 CM03；超过 64 MiB 时不构造 CM02 对比候选，CM03 合格就直接使用，预测不适用或收益不足才回退 CM02/CM01。头部、掩码、调色板、行末填充及任意尾部逐字节保留；原始格式载荷无损还原，不使用 JPEG 保存原始位图。预测、预览与压缩仍在锁外；没有改变修饰键、掩码键、粘贴代号、touch 或 dedup_key。

CM03 独立版本便于绑定认证域、明确拒绝旧读取器误解码，不改变 CM02 定义。布局为 `CM03(4) + 原始长度 u64 LE(8) + nonce(24) + 密文 + tag(16)`；AAD 是 `CM03:` + id 的 UTF-8 字节，再追加同一长度字段。密文的明文是 zstd 流，解压得到原长度的预测 DIB 再追加一个全局滤波类型字节 `02`（所有行均 Up）。认证后按原长度加一解压，并验证精确长度、滤波类型和 DIB 范围；解压缓冲上限仍为 512 MiB，因此恰好达到上限的原文不选择 CM03。详细定义见 [M1 存储层说明](docs/M1存储层说明.md)。

本机 AMD Ryzen 7 9800X3D、Windows、现有 test/dev 优化（项目 1、依赖 3）、zstd 级别 3；单测试线程，三类合成图各 7 次。每张 3840×2048、32 位 DIBV5，原始 **31,457,404 字节**。表中大小包含 52 字节加密开销，耗时为中位数，单位 ms。

| 合成样本 | CM02 字节 | CM03 候选字节 | 最终选择 | CM02 压缩 / 解压 | CM03 预测压缩 / 解压逆滤波 |
|---|---:|---:|---|---:|---:|
| 截图：渐变、色块、细线 | 158,140 | 29,555 | CM03 | 6.941 / 7.511 | 26.206 / 25.276 |
| 照片模拟：曲面渐变 + 小幅随机噪声 | 26,520,858 | 15,558,194 | CM03 | 171.207 / 35.118 | 231.120 / 76.273 |
| 高噪点 RGB | 26,877,523 | 26,883,324 | CM02 | 43.938 / 29.966 | 66.938 / 49.143 |

CM03 解压加逆滤波范围分别为 23.835–27.479、71.521–81.171、46.113–53.617 ms，均低于 100 ms；高噪点实际选择 CM02。固定 Up 无需逐行比较五种滤波或执行 Paeth，满足本机解码目标，因此保持 zstd 级别 3。生产选择同时尝试两条压缩路径，中位耗时为 32.789 / 394.726 / 116.869 ms，不能把表中 CM03 单路径压缩耗时当作全部准备耗时。最终版本的内存认证解密加解码中位为 27.375 / 99.979 / 65.571 ms，照片最高 104.133 ms；不含 SQL、文件 IO、剪贴板写入或目标应用显示，**不是端到端粘贴低于 100 ms 的保证**。

同一合成不透明照片（800×600，RGB 与全不透明 RGBA 均验证）预览编码对比：

| 输出 | 旧默认 RGBA PNG | Best/Adaptive RGB PNG | 最终 JPEG 85 |
|---|---:|---:|---:|
| 缩略图（213×160） | 59,317 字节 | 52,986 字节 | 9,320 字节 |
| 大预览（800×600） | 1,141,131 字节 | 950,026 字节 | 182,326 字节 |

两者较旧 PNG 缩小约 84%；透明图保持 PNG 且逐像素 alpha 回归通过。数据来自合成图片，不代表用户 QQ 图片的实测结果。格式及缩略图清晰度、实际程序粘贴仍须人工验收。

已知限制：JPEG 预览是有损展示副本，细字/锐利彩色边界可能出现编码误差；原始载荷仍无损。**原图是 JPG 这类有损格式时，剪贴板中的原始像素无损保存后，体积通常仍然大于原 JPG**。高噪点与不支持的 DIB 不保证额外收益；纯色输入若普通 zstd 已经足够小，保底选择原方案，不强求 CM03 更小。固定 Up 不一定对所有方向纹理最优。旧记录不重压，同哈希对象不覆盖；增量比较需要未保存过的测试图片。64 MiB 以内双路压缩增加 CPU 和瞬时内存占用；超过 64 MiB 采用 CM03 优先并释放失败候选后回退，仍需原文、预测缓冲及压缩输出；worker 仍串行，正在采集大图时粘贴可能排队。

复现数据：`.tools/predict-bench.log`、`.tools/predict-bench-summary.json`、`.tools/predict-preview.log`。自动测试与人工步骤见 [M1 人工验收清单](docs/M1人工验收清单.md)。本轮未操作用户键鼠或真实应用粘贴，未访问 `.test-data/manual`，未新增依赖，未提交代码。


本轮最终检查（每次 cargo/npm 前均执行 `. .\scripts\env.ps1`）：

| 检查 | 结果 | 日志 |
|---|---|---|
| `cargo fmt --check` | exit 0，无格式差异 | `.tools/predict-fmt.log` |
| `cargo clippy --all-targets -- -D warnings` | exit 0 | `.tools/predict-clippy.log` |
| `cargo test` | **300 passed / 0 failed / 1 ignored** | `.tools/predict-test.log` |
| `npm run build` | exit 0；vue-tsc + Vite，21 modules，620 ms | `.tools/predict-build.log` |

相较 274 passed / 1 ignored 基线新增 26 项，未删除既有测试，tests/ 下的集成测试文件未改。库测试 230 passed / 1 ignored，五组集成测试 22 + 14 + 4 + 12 + 18 = 70 passed。新增覆盖 24/32 位、正负高度、1×1、宽单行、外置/内置位域掩码、调色板、行末 padding、声明额外区与 ICC/任意尾部；预测头部扰动与随机输入、认证后的无效滤波/头/截断、跨格式伪装、限长解压与长度不符；压缩收益和纯色/噪点回退；JPEG/PNG 选择、透明和高位深 alpha、MIME/nosniff、旧 PNG/CM02 读取、JPEG 白名单以及 CM03 删除/孤儿扫描。

测试单线程基准、普通测试线程数 2；原有 Win32 用例在私有窗口站运行。已有 OpenSSL PDB 缺失的 LNK4099 链接信息及错密钥测试的 SQLCipher HMAC 诊断仍存在，测试通过。`git diff --check` 通过。未执行 QQ、微信、Word、画图的人工复制粘贴或主观清晰度验收，不能用上述自动测试代替这些项目。


## 超大图片采集修复（2026-11-10）

单个对象与一次采集累计上限统一为 **512 MiB（536,870,912 字节）**，提示中的 MB 按 MiB 向上取整。`crypto::MAX_PLAIN_BYTES` 是共用来源，`clipboard::MAX_BYTES` 引用它；`read_raw_limited` 在 `GlobalSize` 后、`GlobalLock`/复制前按剩余预算拒绝。已成功取得 DIBV5 时，在读取回调之前跳过 CF_DIB，采集末尾去冗余 DIB 的规范化仍保留。DIBV5 超限则尝试 DIB；其他超限格式也逐项跳过，有文本等有效内容便保存。无有效内容才提示，例如“图片过大（约 600MB），超过 512MB 上限，未记录”。diag 仅记跳过的格式、字节数、预算与原因，不记录载荷。

CM02/CM03 在认证和解压分配之前检查头部声明长度，超过 512 MiB 立即拒绝；认证后仍严格限长解压并核对实际长度。CM03 另需一个滤波字节，因此原文恰好 512 MiB 时回退 CM02/CM01。超过 64 MiB 的可预测位图只先尝试 CM03，合格即使用，不再生成 CM02 比较候选；不能使用时释放候选后回退。加密直接在最终对象缓冲中完成，避免额外整份密文副本；保存借用原始 Clip，大图不走 `f.bytes.clone()`（该复制仅限至多 64 KiB 的内联数据）。

DIB 预览像素预算从 3200 万提高到 `512 MiB / 4`（约 1.34 亿像素），通用解码器 `max_alloc` 同步提高到 512 MiB；单边仍限 16384 像素，文件路径预览仍限 50 MiB。4835×7253 的 32 位图可正常生成 256×160 范围的缩略图和 800×800 范围的预览。仍采用完整解码，内存不足时可能失败；24 位等 BMP 解码路径还需补文件头的输入副本。JPEG 预览有损，原始 DIB 无损。

约 140MB 位图的内存预算：令原文 `N=140,273,144` 字节（133.77 MiB），压缩对象有效字节为 C。保存的主要缓冲区峰值约 `3N`（401.32 MiB）：原文、预测副本、zstd 最大输出容量；预览阶段约 `2N` 加小图，预测和预览不同时存活。粘贴先 load 再写 HGLOBAL，主要缓冲峰值为 `max(N+2C, 2N)`；本次两种样本均约 267.55 MiB。以上为缓冲区估算，另加 zstd/图像编解码器、数据库、分配器与应用基线；不含源应用、已有系统剪贴板、Windows 自动合成的 DIB/Bitmap、目标应用显示。512 MiB 上限不是进程内存上限，大图采集与粘贴仍可能在串行 worker 排队。

本机 dev 优化构建、单线程、4835×7253 合成 DIBV5，分别在独立测试进程单次测量：

| 样本 | 原始 / blob 字节 | 保存（含两级预览） | load | load + 私有剪贴板写回 | 整段测试峰值工作集 |
|---|---|---|---|---|---|
| 渐变、色块及透明像素 | 140,273,144 / 38,108 | 757.499 ms | 126.693 ms | 179.254 ms | 278.18 MiB |
| 曲面渐变加随机小噪声照片模拟 | 140,273,144 / 69,365,142 | 2057.532 ms | 460.394 ms | 510.882 ms | 407.95 MiB |

峰值是整段测试进程（包含逐字节校验）的观测值，不能分配到某个阶段；private bytes 每 50ms 采样的最大值分别约 272.0 / 407.44 MiB，可能漏掉短峰。保存和粘贴的分阶段数字以上述缓冲区模型估算。两种样本均逐字节还原、两级预览解码成功，私有剪贴板读回原始 DIBV5 也一致。此处“粘贴”止于写入私有剪贴板，不包括发送按键、目标应用读取/显示；不能当作真实 JPG 或 QQ/XnView 的性能保证。日志：`.test-data/large-image-bench.log`、`.test-data/large-image-photo-bench.log`。

最终检查：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、`npm run build` 均 exit 0；**313 passed / 0 failed / 2 ignored**，较 300 passed / 1 ignored 新增 13 项常规测试和 1 项可手动执行的大图测试。该 ignored 大图测试已分别执行两种样本并通过；原有 ignored 项保持不变。日志位于 `.test-data/large-image-{fmt,clippy-final,test-final,build}.log`。已有 OpenSSL LNK4099/PDB 与错密钥 SQLCipher 诊断不影响通过结果。未提交、未新增依赖、未访问 `.test-data/manual`、未接管键鼠；QQ/XnView 真实复制粘贴仍待人工验收。
