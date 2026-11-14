# lint 结构与手工审查（第十三至二十三批）

日期：2026-11-14。基线为第十二批未提交工作区。本批完成前后端结构候选盘点、重点人工审查和第一组整改；**此为第十三批历史状态；当前计划内结构复核及最终自动回归已收口，见 lint优化进度.md 第二十三批**，不推进 U3。

## 范围与方法

- 检查生产 Rust、Vue/TypeScript 的长函数、职责混合、布尔模式参数和重复实现，沿用现有 ESLint/clippy 配置；没有提高阈值或新增 lint 豁免。
- Rust 使用本地词法候选扫描，屏蔽字符串、注释及 `cfg(test)` 条目，保留测试块之后的生产代码；结果 `.tools/lint-batch13-inventory-final.json`。这不是 AST、圈复杂度或完整语义证明。初始仅按第一个 `cfg(test)` 截断的扫描会漏掉 runtime/storage 等模块，已废弃该统计口径，不能引用初始结果作为全量结论。
- 前端使用项目现有 TypeScript/Vue 编译器遍历函数候选，结果 `.tools/lint-batch13-frontend-inventory.json`；最长候选约 27 个非空、非独立注释行，仍需关注单行多语句和职责耦合，不能据此认定所有函数均简单。
- 测试、基准、迁移脚本和声明表不机械按生产函数长度整改。`paste_bench.rs` 是显式隔离基准，未运行其重型场景；基准断言不作为产品 panic 点。

## 本批完成的整改

| 发现 | 修改 | 保持的约束 |
| --- | --- | --- |
| `lib.rs::run` 混合存储初始化、IPC 注册、窗口/托盘装配、smoke 等待 | 分出 `open_storage`、`application_builder`、`app_setup`、`app_smoke`、`app_tray` | 数据错误仍在 setup 之前处理；main → preview → 监听 → quick → 原生准备顺序保持 |
| 托盘创建与运行时暂停更新共处入口，简单搬移会保留反向调用 | 独立 `tray_status` 负责图标与状态渲染；运行时直接调用它，`app_tray` 只装配菜单/控制动作 | 渲染模块不导入 runtime/app_events；保持图标像素、菜单顺序和暂停更新 |
| 存储和浮窗依赖入口中的详情类型/转换 | 移至 `item_detail.rs`，调用方直接依赖该模块 | 字段名、20000 字符上限、文件存在性语义不变；避免存储为共享 DTO 依赖入口 |
| `Shortcut::label(modifiers_only)` 布尔模式 | 明确 `label`、`hold_label`，共享修饰键片段和按键标签 | 修饰键顺序与名称不变；不复制键名表 |
| 设置页 `cancel(leave)` 混合停止录制与退出 | 分成 `cancel` 和 `leaveSettings` | Esc/底部取消在录制中只停止录制；返回列表仍退出并丢弃草稿 |
| 设置页 `scheduleHotkeyCheck(immediate)` 混合首次检测与去抖 | 分成首次检测和调度，共享准备逻辑 | 保留计时器取消、序号递增、录制期间延期及校验门槛 |
| `tray_icon(paused)` 用布尔选择两个图标 | 分为普通图标与暂停图标函数 | 更新入口的 paused 是实际状态数据；渲染仍由状态选择 |
| 恢复滚动锚点与浮窗锚点重复按 ID 查 DOM | 同组件内复用 `recordElement` | 不改选择状态、异步代号或滚动位置算法；不建通用 utils |

启动主函数缩短到约 27 个有效行；新增模块内的非声明式函数保持在建议长度内。`application_builder` 约 48 行，其中大部分为已有命令注册名单，属于可核对的声明表，明确保留；不为行数把注册表拆成多个隐藏入口。

## 长函数与文件职责审查结论

以下长度为候选口径，不能直接作为违规结论。旧代码指标不触发无关全项目重构；“待整改”明确表示尚未关闭的问题。

| 区域 | 人工结论 | 后续动作/保留理由 |
| --- | --- | --- |
| 原 `runtime::start_broker`，约 151 行 | 第十四批已拆为 `runtime/broker.rs` 生命周期与 `runtime/broker_messages.rs` 消息分发 | 已补回复/控制发送及资源清理诊断；保持启动回复、取消标记、录制会话、代号校验、掩码先于 UI 调度及探针优先释放，见下方补记 |
| `runtime::start`，约 59 行（第十三批候选口径） | 第十四批复核：主要为 State 字段清单与顺序装配，保留这一编排入口；文件状态事件发送补诊断 | 不拆散状态所有权；保持设置/清理/列表读取 → State 构造 → 到期/worker/控制线程 → 全局通道 → 原生初始化顺序，不复制测试夹具 |
| 原 `sensitive::detect`，约 139 行 | 第十五批已拆为有界扫描、独立规则与高熵补充，新增 `sensitive/rules.rs` | PEM → 首个已识别前缀 → JWT 顺序及高熵重叠排除保持；长前缀失败不退回短前缀，仍使用原文检查 UTF-8/词边界 |
| 原 `image_normalize::parse_dib`，约 128 行 | 第十五批已拆到 `image_normalize/dib.rs`，分离 Core/Info、掩码、布局/载荷与 ICC；移除 predict 布尔模式 | 规范化 length 与预测 pixels 分别设定接受范围：24 位位域只供预测；保留调色板、行对齐、声明载荷、ICC 和溢出校验 |
| 原 `thumbnail::decode_dib`，约 98 行 | 第十五批已拆到 `thumbnail/decode.rs`，分离受限头读取、调色板/掩码偏移、32 位像素转换及 BMP 回退 | 与规范化保持独立；Core 头不新增解码支持，planes/sizeImage 不收紧既有 32 位解码范围，保留尺寸/分配限制、行向与 alpha 行为 |
| 原 `hook::callback`，约 88 行；`Machine::step`，约 55 行 | 第十八批已拆分 `hook/event.rs` 的状态编排/录制/投递/收尾及 Machine 活动动作/配对 | 回调未新增分配、锁、IO、焦点修改和同步等待；64 条轨迹、262,144 个事件对照一致，新增投递失败配对测试；hook start/stop 生命周期错误链已在第十九批处理，见下方补记 |
| 原 `clipboard::collect_formats_with_budget`，约 85 行；`capture_at`，约 75 行 | 第十六批拆到 `clipboard/collection.rs` 与 `clipboard/capture.rs`，合并超限记录更新，预算状态显式持有 | 保留规范化前预算、V5 成功后才跳过 DIB、CF_HDROP 补充、最终错误优先级，以及打开前后序列/来源/隐私门与固定元数据日志 |
| 原 `storage::set_sensitive`，约 68 行 | 第十六批分出整批预读、标记、取消标记及摘要恢复；边界 value 保留为目标状态 | 两种目标状态都预读整批，去重/跳过缺失后只开一个事务，单个 now，统一提交；错误保留整批回滚，不拆逐条事务 |
| 原 `storage::delete`，约 57 行 | 第十六批分出事务删行、提交后 blob 回收及 VACUUM；语句仍先释放再提交 | 保留重复/缺失 ID 计数、候选去重顺序、共享对象保护及严格 >200 阈值；维护错误只进入报告，不能变成删除失败 |
| `storage::commit_prepared`，约 45 行 | 二次去重、先安装对象、单事务写记录与格式 | 本轮保留事务编排；以后提取时不改变锁外准备后的再次查重及敏感搜索清空 |
| 迁移 `upgrade/migrate_v1/migrate_v4` | SQL 与备份/版本过渡语义占主要篇幅 | 按迁移代码保留，禁止仅按行数拆开恢复边界；不改已发布 schema |
| `main_position`、`platform::position_quick`、`file_actions::shell_action` | 多为系统边界读取、坐标/窗口结构填写和系统资源生命周期 | 本轮保留；后续只按明确职责抽取，原生 DPI、句柄释放与 Shell 行为不能由静态检查替代 |
| `lib.rs`、`runtime.rs`、`storage/mod.rs` 等长文件 | 除测试外仍有较多生产行，主入口已移走装配和详情职责 | 仍需按 IPC 家族、broker、存储读写职责分批整理；不为每个命令新建文件 |
| `MainWindow.vue`，第二十批 654→608 行 | 详情/浮窗状态已移至 itemDetail.ts / compactPreview.ts，主窗保留列表和窗口事件协调 | 保留 refresh/detail/appearance 代号、watch 注册及卸载调用顺序；9 项状态回归与相关离屏通过。设置录制职责仍待处理，不能据此宣称全部前端审查完成 |
| `SettingsView.vue`，第二十一批 347→286 行 | hotkeyRecording.ts 独立持有录制核心状态，设置页保留表单/验证/系统检测协调 | 保留 session/epoch/revision/operations/disposed、同步 watcher、输入隔离和卸载顺序；11 项录制状态回归与 T3/U1/U2 通过，计划内前端职责子项收口 |

## 布尔参数：数据与行为分开判断

- 本批已移除的行为开关见上表；不是所有 bool 都应消灭。
- 保留状态/数据：置顶与敏感标记的目标值、自动启动开关、是否可见/最小化/前台等纯判定输入、键盘 down/injected/enabled 事件属性、IPC 中 plain/notice 等现有字段。`MainWindow.paste(plain)` 当前仅把数据传给 IPC，没有在前端切换两套流程；协议不能为结构指标随意改名。
- **第十七批已处理的内部模式**：`platform/key.rs` 明确 protect/unprotect；`storage/detail.rs` 明确 display_detail/reveal_detail；paster 明确 prepare_current_target/prepare_restored_target；hook 明确 start_recording/stop_recording。共享原有读取、验证、轮询和清理步骤；没有以对象/枚举隐藏模式。IPC 的 active/restore 等状态字段保留，不改变协议。
- `runtime::start(..., smoke)` 属于已有启动边界，本批调用端已分为正常与 smoke 初始化；后台入口的模式职责仍留运行时装配审查。`sensitive::detect(..., high_entropy)` 是配置值驱动的可选规则，第十五批保留这一数据入口；DIB 的 predict 行为参数已由规范化 length / 预测 pixels 两个明确入口取代。

## 重复实现与依赖边界

- 已合并：同主窗口中两处按 ID 查找记录元素；快捷键修饰键片段；设置页取消的退出步骤和检测准备步骤。
- 不合并：DIB 规范化和缩略图解码接受范围不同；前后端快捷键描述分别在 Rust/TypeScript 运行，继续靠现有边界与回归验证，不引入生成器或跨语言运行依赖。
- 不抽出“万能锁读取/失败吞掉”工具：锁错误上下文、一次性消费和快照复制语义不同；保留各职责模块的明确接口。
- 暂保留 Rust 声明/常量表及测试中的重复夹具布局；不复制新的运行时 State 构造，不额外创建 Tauri 测试应用。
- 本批移除详情对入口的依赖，并使托盘状态渲染不反向依赖运行时。既有 hook/runtime/diag 的全局耦合仍在，不能宣称已证明整个依赖图无环。

## 验证与覆盖边界

最终检查结果和日志以 `lint优化进度.md` 第十三批记录为准。使用现有 Rust 测试、T3/U1/U2/T8 离屏回归验证标签、设置操作、窗口 IPC、锚点和托盘像素；本批没有为纯搬移增加镜像实现测试。

首次 clippy 检出提取监听闭包时返回了 `compact_preview::hide` 的 u64 代号而不是 `()`，已恢复原来的分号丢弃语义。未修改检查配置掩盖错误。OpenSSL 缺失 PDB 的既有链接提示仍记录在日志中。

没有接管键鼠、启动产品窗口或覆盖真实配置；没有重新执行原生焦点/DPI、smoke 窗口或 ignored 重型测试。窗口属性与创建顺序经过代码对照，不能称为原生视觉验收。B-01/B-02 保持原状态，未提交。

## 下一批入口

broker、DIB、敏感检测、clipboard 采集/预算、存储批量事务、详情/reveal、加解密、焦点恢复/录制入口和 hook callback/Machine 已分批整改；当前剩余前端状态职责、其余 Rust 职责/错误路径复核及最终全量收尾，见第十八批补记。每批单独验证既有错误链与时序约束。整体结构整改和最终回归仍是两项未完成工作，不将本报告视为优化结束。


## 第十四批补记：broker 生命周期与消息分发

- `broker.rs` 持有窗口类、消息窗口、剪贴板监听、定时器及主热键登记状态，按取得的资源清理；`broker_messages.rs` 分发设置请求、录制和动作消息。新增两个模块共 17 个生产函数，词法候选最大 31 个有效行，详见 `.tools/lint-batch14-structure.json`；人工复核新增控制流未超过两层，不新增模式布尔参数或通用工具模块。
- 正常路径保持 250ms 定时器、热键初始注册后启动回复、设置请求 FIFO/取消标记、当前登记主热键传入冲突检测、录制 session/active 校验及 packed=0 的掩码失败语义。动作先解码再读取当前粘贴代号；Open 先掩码再派发，失败仍先 Cancel 后 Report；其他动作不增加掩码。
- `messages` 中的临时检测探针先析构，随后依次移除监听、注销已登记主热键、移除定时器、销毁窗口，最后释放窗口类。清理失败只记录诊断，继续处理剩余资源，不触发 UI 调度或重试；退出时清零 broker/listener 句柄发布值。
- 明确异常行为：定时器创建失败现在返回启动错误；窗口/监听/定时器的部分初始化失败回收已取得资源。启动回复接收方已退出时停止消息循环并清理。`GetMessageW=-1` 记录错误后清理，`0` 正常退出。没有改变正常启动回复时点或引入自动重启。
- 复用现有 `app_events::record/send_control`，为请求回复、broker 控制消息、主热键 Toggle、运行时错误/历史通知、文件状态事件、退出投递补诊断；发送失败不重排或重新入队。`TranslateMessage=false` 不是失败，保留丢弃返回值并解释原因。热键探针最终释放失败也记录诊断。
- 新增 4 项隔离测试：取消请求不登记/不回复；旧代号与未知动作不掩码/不派发；Open 掩码在派发前、掩码失败先取消后报告、Cancel 不掩码；控制通道缺失或断开返回错误。测试不创建原生产品窗口、不注册真实热键、不发送按键、不使用桌面剪贴板。
- 原生窗口创建/监听/定时器/启动回复/清理系统故障未逐项实注入，资源路径依靠代码对照审查；没有新增原生焦点/DPI/smoke 窗口验收，不重跑 ignored 重型测试。静态、隔离与离屏结果不替代这些原生覆盖；未接管键鼠、修改真实偏好或提交。其余结构整改与最终全量收尾仍未完成，不推进 U3，B-01/B-02 不变。

本批最终 fmt、全目标 clippy、ESLint、类型/构建及差异检查通过；Rust 664 通过/0 失败/3 ignored，T3/U1/T8/U2 离屏 13/20/14/104 项通过。以 `.tools/lint-batch14-{fmt-final,clippy-final,rust-final}.log` 及同批前端日志为准，失败与覆盖边界详见进度文档第十四批。


## 第十五批补记：DIB 解析与敏感检测

- `image_normalize/dib.rs` 207 行，负责 packed DIB 的头部、掩码、调色板、行布局、声明载荷及 ICC 校验；规范化 length 与预测 pixels 复用校验但分别约束范围，移除原 predict 布尔模式。`thumbnail/decode.rs` 170 行，独立负责尺寸探测、受限图像解码和 DIB/BMP 路径，不复用规范化的接受规则。`sensitive/rules.rs` 260 行，负责 PEM、类型化前缀长度/字符集和 JWT；入口保留扫描上限与可选高熵规则。
- 新增/修改的 40 个生产函数词法候选最多 31 个有效行，记录 `.tools/lint-batch15-structure.json`。该扫描并非 AST 或复杂度证明；人工复核规则扫描、JWT 点计数、像素循环与掩码校验不超过两层控制流。未为短函数机械拆分原 redact 或无关编码路径。
- 保持规范化接受 12/40/52/56/108/124 头、原位深/压缩组合、RLE/JPEG/PNG 正高度与声明长度要求；24 位位域仅预测接受。掩码仍须非零、不交叠、连续且不超位深；保留 Core 3 字节调色板、其他头 4 字节调色板、stride/checked 算术、非零 sizeImage 保守保留、ICC 位于声明像素载荷后及最终长度界限。
- 解码保持只接受 40/52/56/108/124 头、16384 边长与原内存预算；32 位直接解码的 planes/sizeImage、掩码、缺省 alpha 语义不因新结构而收紧。其余位深仍补 BMP 头并交给原受限解码器，格式优先级和损坏格式回退不变。移动已有字节读取时沿用局部职责，没有创建通用工具模块或扩大规范化/解码的共享范围。
- 敏感检测保持 64KiB、UTF-8 向前取整、原文左右词边界、PEM → 首个已识别前缀 → JWT 的规则顺序。前缀长度/字符集用枚举替代数字标记，长前缀校验失败不得尝试短前缀。JWT 仍要求两点、JSON 对象且头含 alg、非空合法签名及正确末尾 padding bits；第三点拒绝不变。高熵仍在规则命中后追加、不覆盖已命中区间，最后按原起点稳定排序；阈值与排除条件保持。
- 长期新增 6 项规格测试：24 位位域仅预测且仍校验掩码/载荷；预测布局不包含声明尾部/ICC 但必须校验它们；解码接受范围独立于规范化；长前缀失败不回退；PEM 优先且高熵仅补非重叠命中；PEM 页尾越过多字节扫描边界不被消费。
- 修改前源文件快照保存在 `.tools/lint-batch15-baseline/`。使用临时对照入口将新旧实现对比：16,377 组 DIB 输入的裁剪字节、预测布局、解码 RGBA 像素一致；7,161 组敏感输入在高熵开关两种状态下命中规则、范围及脱敏结果一致。最终证据 `.tools/lint-batch15-equivalence-final.log`，2 项对照测试通过；入口已从 tests 移除，副本 `.tools/lint-batch15-equivalence.rs` 保留，不纳入正式测试数。样本覆盖是回归证据，不是所有字节输入的形式化等价证明。
- 未读取图片或接管键鼠；没有注册真实热键、使用桌面剪贴板、修改 manual/真实偏好或提交。未新增原生焦点/DPI、真实大图/ignored 性能验收；前端离屏使用隔离无头 profile 与合成 IPC，不替代原生图像剪贴板验收。B-01/B-02 不变，不推进 U3；其余结构整改及最终收尾回归仍待完成。

本批最终 fmt、全目标 clippy、ESLint、类型/构建与差异检查通过；正式 Rust 670 通过/0 失败/3 ignored（不含临时对照 2 项），T1/T7/U2 离屏 8/20/104 项通过。证据 `.tools/lint-batch15-{fmt-final,clippy-final,rust-final}.log` 及同批对照/前端日志；初次漏带局部读取函数的错误已修复，失败日志保留，详见进度文档第十五批。


## 第十六批补记：采集预算与存储批量事务

- `clipboard/capture.rs` 126 行，负责单轮序列/来源/隐私门与上下文装配；`clipboard/collection.rs` 173 行，负责格式顺序、剩余预算、读取结果合并、最大超限记录、补充 DropEffect 及最终结果。`FormatBudget` 将 total/limit/has_v5 作为状态，选择函数不再接收五个散参。27 个新增/修改生产函数词法候选最多 34 个有效行，`.tools/lint-batch16-structure.json`；非 AST 或圈复杂度证明，人工复核新增控制流不超过两层。
- 规范化前按原始字节扣除预算；读取失败/超限不占预算，只有 V5 成功保留后才在读前跳过 DIB，V5 失败仍允许 DIB 回退。超限 actual 继续加已保留 total 并饱和，统一为整次 limit，最大 actual 优先、并列保留首次类型；没有有效内容时保持超限 → Invalid → None，已成功的内容不因其他格式失败丢弃。Preferred DropEffect 的短数据/普通错误不算内容无效，CF_HDROP 只在余量容纳四字节时补 Copy，格式顺序与旧记录写回不变。
- 采集前自写回确认与 LAST_SEQUENCE、打开后再次采样、格式枚举/owner/source、忽略程序门、位图提供标志、排除标志、历史隐私标记、所有权标记及内容读取的先后保持。忽略命中仍不请求任何载荷；历史标记不可读时跳过，所有权标记不可读仅跳过该标记。OpenGuard/CaptureLog 的释放顺序保持，采集仍只尝试打开一次，worker 退避与采集后序列保护未改变。
- 敏感状态变更将预读/去重/缺失跳过、检测与 PreparedSensitivity、标记与取消标记写入分开，保留边界 bool 为目标状态。两种状态都先加载完整批次，再进入一个事务、使用单个 now；JSON 对象校验、三态/TTL、搜索索引、摘要/命中恢复与回滚语义保持。没有为取消标记省略原来的预读与检测；摘要恢复的空文件列表行为单独保留，未与保存时摘要回退粗略合并。
- 删除仍先在单事务中收集候选并逐 ID 删 formats/items，语句释放后提交；随后按候选排序进行共享引用复核/文件回收，最后严格超过 200 条才 VACUUM。提交后维护函数返回报告而不传播删除错误，引用查询/文件删除/VACUUM 失败继续保留原分类和诊断，未加事务内文件删除或改变清理保护策略。
- 新增 4 项长期测试：文本填充按规范化前计预算；最大超限及并列首次类型优先于 Invalid；整批载荷不可读时标记/取消标记都不开始写入；独立 SQLCipher 连接通过 authorizer 拒绝 VACUUM 所需 ATTACH，验证删除已经提交、维护失败进入报告且重开后仍已删除。复用原私有窗口站与事务回滚/共享 blob/锁定文件用例，不增加原生产品窗口。
- 修改前快照 `.tools/lint-batch16-baseline/`；临时对照纯格式合并的 14,336 组输入，读取顺序/剩余预算、保留格式字节及错误种类/大小/limit/image 均一致。`.tools/lint-batch16-equivalence-final.log` 与 `lint-batch16-equivalence-result.json` 为证据，临时入口已从 tests 移除，副本 `.tools/lint-batch16-equivalence.rs` 保留，不计入正式测试数。诊断输出在此纯对照中使用空接收器，原生来源/时序由代码对照及原私有窗口站测试负责；有限样本不证明所有输入等价。
- 未接管键鼠或使用桌面剪贴板，未修改 manual/真实偏好、提交或推进 U3。原生焦点/DPI、实际外部粘贴和 ignored 大图/性能未重新验收，离屏不替代这些覆盖；B-01/B-02 不变。整体结构整改与最终收尾回归仍待继续。

本批最终 fmt、全目标 clippy、ESLint、类型/构建及差异检查通过；正式 Rust 674 通过/0 失败/3 ignored（不含临时对照 1 项），T2/T6/T7/U2 离屏 18/8/20/104 项通过。日志 `.tools/lint-batch16-{fmt-final,clippy-final,rust-final}.log` 与同批对照/前端记录；临时夹具首次路径错误与覆盖边界见进度文档第十六批。

## 第十七批补记

- 详情与 DPAPI 职责分别迁入 storage/detail.rs、platform/key.rs；根平台重导出密钥 API。详情仍先 SQL 再载荷，坏 JSON 的敏感/非敏感/reveal 分歧、命中回退、字符截断及隐藏文件保持；显式 reveal 的权限检查未移动。
- 焦点准备仍在原 Prepare 阶段，main 的 Acquire 读取时点、权限短路、恢复失败错误、十轮检查与末轮 sleep 保持。录制 start/stop 的 session 递增及 UNTIL/ARMED/OWNER 写入顺序、停止不清 UNTIL 保持；钩子回调和 Machine 未修改。
- PasteReport 断开发送补固定上下文诊断，仍先核对当前代号且不重试；DPAPI 输出先复制、清零、LocalFree，释放失败只记固定诊断。未实注入系统释放失败。
- 新增详情坏 JSON/缺失 ID/状态索引、轮询节奏、录制原子生命周期 3 项长期测试，原 DPAPI 与密钥文件保留测试迁移后继续运行。新拆模块与粘贴任务 16 个生产函数词法候选最多 29 个非空行；人工核对新增控制流不超过两层，无新增生产 unwrap/expect。hook callback/Machine 与前端组件职责仍待专项处理，不宣称整体结构优化完成。
- 最终 fmt、全目标 clippy、ESLint、类型/构建及差异检查通过；Rust 677 通过/0 失败/3 ignored，T3/T7/U1/U2 离屏 13/20/20/104 项通过。日志与首次 clippy 修正见进度文档第十七批。OpenSSL 缺 PDB 提示保留；未重新原生焦点/DPI/外部粘贴或 ignored 大图性能验收，B-01/B-02 不变。未接管键鼠、修改真实偏好或提交，不推进 U3。

## 第十八批补记与剩余范围

- callback/Machine 专项已完成本批整改，见上表；新增/修改 12 个生产函数词法候选最多 39 个代码行，人工复核新增控制流不超过两层。hook/event.rs 在 300 行建议内；固定大小事件和静态分发未新增堆分配或动态调度。回调保持原非阻塞诊断路径，不增加通用日志。
- 人工逐段核对原子顺序和取消/配置/录制/投递/诊断/释放门/写回边界；旧/新纯状态机 64 条轨迹、262,144 个事件的决策、active、down/swallowed 一致。5 项新增长期测试覆盖动作投递失败及录制拒绝后的配对/优先级。对照只覆盖纯状态机，不声称已运行真实回调或证明原生性能/全部输入。
- 最终 fmt、全目标 clippy、ESLint、类型/构建及差异检查通过；正式 Rust 682 通过/0 失败/3 ignored，T3/U1/U2 离屏 13/20/104 项通过；临时对照入口已从源码移除。证据 `.tools/lint-batch18-*`，细节见进度文档。
- 当前剩余 **3 大部分**：前端详情/浮窗/设置录制职责；其余 Rust 长文件/职责与错误链复核（含 hook 线程 start/stop）；最终全量回归与记录收口。最新词法扫描 `.tools/lint-batch18-inventory-final.json` 仍有 15 个 >40 代码行候选，其中迁移、声明清单、事务与系统边界编排已有保留理由。后续逐项复核，而不是认定 15 项违规或无限拆分。
- 既有 OpenSSL 缺 PDB 提示保留；未重新原生焦点/DPI/外部粘贴或 ignored 大图性能验收，离屏不能关闭这些覆盖边界。B-01/B-02 不变，未接管键鼠、修改真实偏好、提交或推进 U3。

## 第十九批补记：hook 线程生命周期

- `hook/lifecycle.rs` 独立安装、队列发布、启动回复、消息循环和 Registration 释放，根模块重导出 API。15 个新增/修改生产函数词法候选最多 15 个代码行；人工核对新增控制流不超过两层，没有新增生产 unwrap/expect 或模式布尔。hook start 从 >40 代码行候选移除，最新扫描余 14 个候选，不等于待改项数量。
- 正常顺序仍为 RECEIVER → 线程修饰键采样/播种 → 安装 → 建队列 → THREAD → 回复 → 读消息。取消仍先设置标记再投递，失败保留回调兜底；停止仍先清 READY 再读取 THREAD。清理先清 THREAD 再卸钩；安装/回复/消息错误及释放失败补诊断。callback/Machine 段与本批前快照逐字一致，不改变其时序或无分配/锁/IO 约束。
- 明确修正两个异常行为：启动成功回复接收方已退出时不再遗留线程/钩子，改为清理；THREAD=0 时不再向调用线程自身发送 WM_QUIT。非零线程 ID 的投递失败记录且不重试，卸钩失败诊断不等同于释放成功。GetModuleHandleW 的原有 None 回退保留并增加诊断；GetMessageW=-1 和正常 0 分开。
- 7 项长期测试覆盖回复顺序与丢失、读取错误/正常退出、清零先于失败释放、0/非零投递及真实私有线程消息队列。没有安装真实钩子或注入键盘，未实注入 OS 安装/卸钩故障。最终 fmt、全目标 clippy、ESLint、类型/构建及差异检查通过；Rust 689 通过/0 失败/3 ignored，T3/U1/U2 离屏 13/20/104 项通过，证据 `.tools/lint-batch19-*`。
- 当前仍余前端状态职责、其他 Rust 职责/错误链复核、最终全量收尾 **3 大部分**；hook 生命周期子项已完成，下一批建议前端状态所有权、异步代号与卸载清理。既有 PDB 提示、B-01/B-02 与原生焦点/DPI/外部粘贴/大图性能覆盖边界不变，整体优化未结束；未接管键鼠、修改真实偏好、提交或推进 U3。

## 第二十批补记：前端详情与浮窗

- 详情 refs/失效代号由 itemDetail.ts 持有，浮窗 active/请求代号/去重与几何缓存由 compactPreview.ts 持有；主窗口显式调用，未引入自动生命周期或新状态框架。列表和窗口事件/watch 的所有权仍在 MainWindow；录制状态与草稿/检测的耦合留下一批专项处理，SettingsView 本批逐字节未变。
- 两个模块 61/62 行，主窗 654→608 行；TypeScript AST 扫描 13 个函数声明最多 39 个非空行，`.tools/lint-batch20-structure.json`。人工核对新增控制流不超过两层，无 any/模式布尔/全局单例；模板、watch 注册和卸载调用顺序保持。
- 保留普通详情仅按代号提交、reveal 另核对当前 ID、旧 catch/finally 拒绝、紧凑与图片短路、conceal 与只失效代号的区别；浮窗 nextTick 先于 ID 去重、当前非空 ID 才报告失败、几何失败清签名、null 隐藏及锚点裁剪保持。取消帧入口仍只承担原卸载清理，不额外改变后续调度语义。
- 新增长期 `node scripts/check-main-preview-state.mjs`，真实 Vue refs/nextTick、合成 IPC/帧和现有 TS 配置转译，9 项通过；类型另由 vue-tsc 验证。首次用例错误假定最终清理后可重新调度，修正测试顺序而未改产品行为，失败日志保留。临时模块只清理本次创建、父目录经验证的 .tools 子目录。
- 最终 fmt、全目标 clippy、ESLint、类型/构建及差异检查通过；Rust 689 通过/0 失败/3 ignored，状态 9 项，T3/T4/T7/U1/U2 离屏 13/18/20/20/104 项通过。日志 `.tools/lint-batch20-*`。剩余为设置录制状态职责、其余 Rust 最终审查、全量收尾 3 大部分；原生焦点/DPI/外部粘贴和大图性能覆盖未扩大，B-01/B-02 不变，未接管键鼠、修改真实偏好、提交或推进 U3。

## 第二十一批补记：设置录制状态与前端子项收口

- hotkeyRecording.ts 持有每个实例的字段/operations、行问题/修订号、session/epoch、timer、提前结果及 disposed。父组件保留原 draft ref、验证/冲突优先级、系统检测与 checkAfterRecording、watch/监听/卸载；通过显式 draft 和 scheduleHotkeyCheck 协作，无新增框架、自动生命周期或全局单例。
- 关键顺序保持：停止先递增 epoch 和捕获修订、清 timer，再 field=null/session=0/清提前结果、operations++、IPC、finally 减计数；结果先 await 停止再核对修订并写草稿。提前 null 与未收到结果区分，旧 session/epoch/修订/卸载结果继续拒绝；相同值仍补检测。捕获按键、Esc、输入法/重复/停止在途的隔离和卸载解除监听顺序不变。
- SettingsView 347→286 行，录制模块154行；AST 统计 10 个函数声明（不含适配箭头）最多20个非空行。人工核对新增嵌套不超过两层、参数不超过3个，无 any/模式布尔。模板、警告优先级及检测调度保持，差异 `.tools/lint-batch21-settings-review.diff`。
- 新增 `node scripts/check-hotkey-recording-state.mjs`，11项真实 Vue refs/watch + 合成 IPC/timer 回归通过；状态写入顺序和实例隔离直接验证。原详情/浮窗9项、T3/U1/U2离屏13/20/104项通过，Rust689通过/0失败/3ignored，fmt、全目标clippy、ESLint、类型/构建及差异检查通过；日志 `.tools/lint-batch21-*`。
- 计划内前端详情、浮窗、设置录制职责子项已收口。主窗保留列表/窗口协调、设置页保留表单/检测协调，不机械继续拆文件；整体剩余 **Rust最终职责/错误路径复核、最终全量回归与文档收口2大部分**。14个Rust长函数候选包含保留项，下一批逐项决策。原生录制/hook/焦点/DPI/外部粘贴和大图性能未重跑，B-01/B-02不变；未接管键鼠、修改真实偏好、提交或推进U3。

## 第二十二批：Rust 候选最终决策

第十九批的 14 个长函数候选逐项复核如下。代码行数为屏蔽注释/字符串/cfg(test) 后的词法统计，不等于复杂度或违规判断。

| 候选 | 决策 | 理由与边界 |
| --- | --- | --- |
| `diag::init`（原54） | 整改 | writer 独立，ready/消息/写入失败退出清零，日志格式和正常写入顺序保持。 |
| `clipboard_owner::window_process`（原44） | 整改 | 拆来源查询，直接分类返回错误，释放失败由上层 report 接收；来源状态/fallback 保持。 |
| `sensitive::redact`（原42） | 整改 | 规则 match 分发，补损坏元数据边界；正常命中优先级和输出保持。 |
| `storage/migrations::upgrade`（68） | 保留 | 备份/独占锁、跨版本事务、版本更新和显式 rollback 构成恢复边界，避免拆散事务编排。 |
| `runtime::start`（62） | 保留 | State 字段和顺序装配；先设置/清理/list，再 State、线程/全局通道和原生初始化，错误路径已处理。 |
| `main_position::apply`（53） | 保留 | DPI 前后尺寸/位置两次约束有时序依赖，错误已传播；纯测试不替代原生 DPI 验收。 |
| `platform::position_quick`（53） | 保留 | caret 失败回退鼠标、monitor 约束及无激活定位构成系统边界；ShowWindow 返回旧可见状态不视为失败。 |
| `file_actions::shell_action`（52） | 保留 | 专用 STA/COM RAII、open 或 PIDL reveal 与 ILFree 已构成 Shell 边界并映射错误；本批不真实打开外部程序。 |
| `lib::application_builder`（48） | 保留 | 主要为命令注册声明表，保留完整协议名单。 |
| `capture_policy::run`（46） | 保留 | 有界五段退避；self 优先/序列/读取后重采样/耗尽确认/最后 ack 与日志为时序协议，已有纯测试。 |
| `storage::commit_prepared`（45） | 保留 | 二次去重、先安装 blob、单事务统一记录/格式/敏感索引提交，保留事务边界。 |
| `clipboard::read_format_limited`（44） | 保留 | CF_HDROP 系统边界的数量/长度限制及错误/diagnostics 顺序；内存释放职责已另拆。 |
| `clipboard::write`（44） | 保留 | 先准备全部内存、owner marker 首位、一次 open/empty、逐格式所有权移交、最后 SELF_SEQUENCE；Allocation 已独立。 |
| `main_position::capture`（43） | 保留 | 鼠标/monitor/DPI 读取及 96 DPI 回退属于边界；None 回退不额外改成错误。 |

- 最新 `.tools/lint-batch22-inventory-final.json` 余 11 个 >40 代码行候选，全部在上表明确保留；不再作为必改清单。lib/runtime/storage 根文件保留 API、装配与事务协调职责，不因总行数继续扩大范围。
- 本批 `diag.rs`、`diag/writer.rs`、`clipboard_owner.rs`、`clipboard/memory.rs`、`sensitive.rs` 共 44 个生产函数词法候选最多34代码行；新增控制流人工复核不超过两层，无新增生产 unwrap/expect 或模式布尔。错误处理与顺序对照 `.tools/lint-batch22-review.diff`；按键采集/编码/脱敏、来源选择、剪贴板写入移交和打码正常语义保持。
- 明确异常行为变化：诊断 ready 丢失/读取或写入失败后清 THREAD，重复初始化报错，投递失败计 LOST；CloseClipboard/CloseHandle/内存清理失败补诊断；倒置范围跳过、损坏 PEM UTF-8 元数据不 panic。GlobalUnlock/GlobalFree 按原生返回约定判断，避免包装产生误报；不新增依赖或配置。
- 新增 13 项 Rust 测试后 **702通过/0失败/3ignored**；fmt/clippy/ESLint/类型构建/diff 通过，状态检查9/11项、T2/T6/T7/T8/U2离屏18/8/20/14/104项通过。writer故障使用合成注入，私有线程队列和独立HGLOBAL使用真实API；OS释放失败未实注入。证据与详细范围见进度文档第二十二批。
- Rust 最终职责/错误路径候选复核收口；剩 **最终全量回归与文档收口1大部分**，不是整体任务已结束。原生录制/hook/焦点/DPI/实际外部粘贴和 ignored 大图性能未重跑；B-01/B-02、OpenSSL PDB 提示保持。未接管键鼠、修改manual/真实偏好、提交或推进U3。
