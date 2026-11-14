# lint 与结构优化进度

日期：2026-11-14。基线：`master` / `b420bdf`。计划内自动回归与文档已收口；原生人工验收边界保留，不推进 U3。

## 第二十三批：最终全量回归与文档收口（2026-11-14）

本轮核对 `master` / `b420bdf` 上的既有未提交工作区，沿用第二十二批结构审查决策，未发现自动回归阻断项；没有修改产品代码、依赖或检查配置。计划内 lint 与结构优化的自动回归及文档收口完成，后续不再把已明确保留的 11 个 Rust 长函数候选计作必改待办。以下历史批次中的“剩余/尚未完成”描述仅代表当时状态。

| 检查 | 本轮结果 |
| --- | --- |
| cargo fmt --check / clippy --all-targets -- -D warnings | 通过 |
| ESLint --max-warnings=0 | 通过，零警告 |
| vue-tsc / Vite build | 通过 |
| Rust 常规全量（test-threads=2） | 702 通过、0 失败、3 ignored |
| T1～T8 离屏 | 8/18/13/18/14/8/20/14，共 113 项通过 |
| U1 / U2 离屏 | 20 / 104 项通过，0 失败 |
| 详情/浮窗、快捷键录制状态 | 9 / 11 项通过，0 失败 |
| git diff --check | 通过 |

证据：`.tools/t9-regression-20261114-203515-949/result.json` 及同目录 fmt/clippy/rust/build/ui-t1～ui-t8 日志；补充日志 `.tools/lint-final-eslint.log`、`.tools/lint-final-check-{main-preview-state,hotkey-recording-state,u1-ui,u2-ui}.log`。常规入口为 `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check-m1.ps1`；补充运行 `npm run lint -- --max-warnings=0` 与上述四个同名 `scripts/*.mjs`。沿用项目环境脚本，构建并发限制为 2，Rust 测试线程 2，浏览器检查串行且使用独立无头 profile。

覆盖边界：离屏为实际浏览器页面渲染加合成 IPC，状态检查为真实 Vue 状态加合成 IPC/计时，不等于原生桌面验收。本轮未重跑原生录制/hook/焦点/混合 DPI/实际外部粘贴或 3 项 ignored 大图性能测试；既有 OpenSSL PDB 提示保留，B-01/B-02 仍未复现定位，不宣称修复。未接管键鼠、操作桌面剪贴板、修改 manual/真实偏好、提交或推进 U3。保留全部既有未提交修改；下一步按用户安排完成原生人工验收及提交，不继续无边界拆分。

## 第二十二批：Rust 最终职责与错误路径复核

- 逐项复核第十九批 14 个 >40 代码行候选，本批整改 `diag::init`、`clipboard_owner::window_process`、`sensitive::redact`；其余 11 项在结构审查表明确保留。候选数量不等于必须重构数量；生产根入口、状态装配、事务协调不再仅按文件长度拆分。快照 `.tools/lint-batch22-baseline/`，对照 `.tools/lint-batch22-review.diff`，最新词法扫描 `.tools/lint-batch22-inventory-final.json`。
- 诊断 writer 独立到 `diag/writer.rs`。正常仍建队列 → 发布 THREAD → ready → 读消息，每条消息仍 KEY → 文本队列 → dropped → flush。Publication RAII 在正常退出、ready 接收方丢失、GetMessage=-1、写入/flush 失败时清零 THREAD；诊断自身故障只向 stderr 报告，避免递归。重复初始化不再启动无法接收文本的新线程；write 只采样一次 THREAD，避免退出清零后向调用线程自身投递，唤醒失败计入 LOST。按键采集/编码/脱敏路径不改，startup_failure 写入/同步失败补 stderr。
- 来源查询拆出 `query_process`，保持 Absent/Unknown/Live/Exited、PID/文件名和查询顺序；CloseHandle 仍在 UTF-16 转换之前，失败只报告固定上下文。OpenProcess 失败直接分类返回错误的 HRESULT，权限拒绝仍 Unknown，PID 已不存在为 Exited。report 回调由上层传入，底层 owner 不依赖 diag，避免反向模块依赖；采集来源选择与 fallback 不改。
- HGLOBAL 所有权移至 `clipboard/memory.rs`。核对 windows 0.61.3 包装后，以已有 kernel32 的原生返回值判断：GlobalUnlock 非零或零且 last-error=0 均成功；GlobalFree 空返回才成功。解锁前清 last-error，错误立即读取；无新依赖/feature。分配、一次复制、成功 SetClipboardData 后移交所有权、Drop 仅释放未交付内存的顺序保持。CloseClipboard/解锁/释放失败补诊断，不重试、不改变正常采集/粘贴返回。
- 敏感打码按规则 match 分发，保留未知规则通用打码、8/9 字符分界、前4后2、PEM 头尾、unscanned_tail、命中顺序和重叠过滤。修复两处既有损坏元数据 panic：倒置范围与其他无效范围一样跳过；PEM 文本 byte5 不在 UTF-8 边界时使用 get 回退 header=0。正常 detect、格式接受范围和命中优先级不变。
- 新增 13 项长期 Rust 测试：writer 7 项覆盖 ready/退出清零、格式顺序、read/write/flush 错误和真实私有线程消息队列；owner 2 项覆盖状态分类/空窗口；独立 HGLOBAL 2 项覆盖最终解锁与字节/释放；敏感打码 2 项覆盖倒置范围和 UTF-8 损坏 PEM 元数据。没有安装键盘钩子或操作桌面剪贴板；OS CloseHandle/CloseClipboard/GlobalFree 失败未实注入，成功内存路径使用真实独立 HGLOBAL。
- 本批五个生产模块词法扫描 44 个函数候选，最多 34 个代码行；人工核对新增控制流不超过两层，无新增生产 unwrap/expect、模式布尔或 lint 豁免。扫描屏蔽注释/字符串/cfg(test)，不作为 AST 或圈复杂度证明；其余长函数的逐项理由见结构审查。
- 最终 fmt、全目标 clippy（`-D warnings`）、ESLint、类型检查/构建、差异检查通过；Rust **702 通过、0 失败、3 ignored**，详情/浮窗状态 **9 项**、录制状态 **11 项**通过。固定构建后 T2/T6/T7/T8/U2 离屏 **18/8/20/14/104 项通过**，U2 零失败。证据 `.tools/lint-batch22-{fmt-final,clippy-final,rust-final,eslint,build,preview-state,recording-state,t2,t6,t7,t8,u2,diff}.log`。既有 OpenSSL `ossl_static.pdb` LNK4099 提示保留。
- Rust 候选复核本批收口，**剩余 1 大部分：最终全量回归、覆盖边界与文档收口**。下一批执行既有全套轻量/离屏检查并核对记录；不把保留的 11 项重新计为必改待办。原生录制/hook/焦点/DPI/外部粘贴与 ignored 大图性能未重跑，B-01/B-02 不变。未接管键鼠、修改 manual/真实偏好、提交或推进 U3，全部此前修改保留。

## 第二十一批：设置录制状态职责

- 核对 handoff 并保留全部既有未提交修改，修改前快照 `.tools/lint-batch21-baseline/SettingsView.vue`。新增 `src/hotkeyRecording.ts`，将录制字段、在途 operations、行问题、字段修订号、timer、session/epoch、提前结果与 disposed 归到单个控制器实例；不新增依赖、状态框架、配置或 lint 豁免。
- 设置页仍持有草稿、验证/冲突提示、检测结果/序号/去抖与 checkAfterRecording，并负责监听注册、捕获键盘和卸载编排。控制器仅通过原 draft ref 与显式 scheduleHotkeyCheck 回调协作，设置页读取其 disposed 状态拒绝旧检测结果。没有新增自动 watcher/生命周期，避免改变 flush:sync 的观察时点。
- 保留停止顺序：先递增 epoch、捕获字段修订号、清 timer，再写 field=null/session=0/清提前结果，然后 operations++，最后 await 原 IPC 并在 finally operations--。开始仍先清本行问题/递增修订、清提前结果、设字段再加 operations；启动成功按 session 消费提前结果，null 与 undefined 继续区分；过期或卸载后的启动只追加原停止 IPC，不恢复录制。三十秒超时保留字段修订号门槛，未增加新的 epoch/disposed 条件改变接受范围。
- 录制结果仍先停止，再核对停止成功/disposed/非空结果/草稿/修订号，再校验快捷键并写草稿；相同值仍显式补做检测，不同值交给原 draft watcher。当前非法结果归为本行冲突，通信失败归为本行录制失败，过期错误不覆盖新修订。保留原提示优先级：格式/组合冲突 → 本行录制问题 → 检测错误 → 系统检测结果。
- 捕获阶段仍先 preventDefault/stopImmediatePropagation，再过滤重复/无字段、Esc、输入法/229，使用原物理键解析；停止在途也隔离输入，闲置时不拦截。卸载仍先 disposed，再取消检测 timer/递增检查序号、解除事件、移除 blur/捕获键盘监听、发停止并 emit(false)。监听注册在途遇到 disposed 仍立即解除新监听，不改变注册时点。
- 新增常规 `node scripts/check-hotkey-recording-state.mjs`，**11 项通过**：提前结果/session 与 null；旧 session 拒绝；相同值补检测；预设修订后拒绝迟到合法/非法值；开始/停止通信错误；非法结果冲突归属；卸载或 epoch 变化后的迟到启动清理；超时修订号；重复/输入法/Esc/停止在途/闲置输入；同步 watcher 的 field/operations 写入顺序及实例隔离。使用真实 Vue refs/watch 与真实 settings 解析，IPC/定时器合成，不安装钩子或注入键盘；临时转译沿用项目 TS 配置，完整类型检查另由 vue-tsc/build 完成。只清理本次创建且父目录校验后的 .tools 子目录。
- SettingsView **347 → 286 行**，录制模块 **154 行**。TypeScript AST 统计 10 个函数声明（不含控制器适配箭头），最多 **20 个非空行**，`.tools/lint-batch21-structure.json`。人工核对新增控制流不超过两层、参数不超过 3 个，无 any、模式布尔或全局状态单例；每个实例只持有自己的状态。模板、警告优先级及检测调度代码对照不变，检测函数仅以 isDisposed() 替代原变量；差异 `.tools/lint-batch21-settings-review.diff`。
- 最终 fmt、全目标 clippy（`-D warnings`）、ESLint（零警告）、类型检查/构建、差异检查通过；Rust **689 通过、0 失败、3 ignored**；录制状态 **11 项**、详情/浮窗状态 **9 项**通过；固定构建后的 T3/U1/U2 离屏 **13/20/104 项通过**，含录制失败、迟到结果、停止在途改预设和延期检测回归。证据 `.tools/lint-batch21-{fmt-final,clippy-final,rust-final,eslint,build,recording-state,preview-state,t3,u1,u2,diff}.log`。本批 Rust 未改，仍执行常规全量检查；既有 OpenSSL PDB 提示未用配置/源码掩盖。
- 计划内前端详情/浮窗/设置录制职责子项已完成；主窗口保留列表与窗口事件协调、设置页保留表单与检测协调，不继续仅按文件长度拆分。**剩余 2 大部分**：① 其余 Rust 职责/长文件及错误路径最终复核，按第十九批 14 个 >40 行候选逐项判定整改或有理由保留；② 最终全量回归、覆盖边界与文档收口。整体结构优化尚未结束；下一批转 Rust 最终复核。
- 未接管键鼠、操作桌面剪贴板、修改 manual/真实偏好、提交或推进 U3。状态脚本为合成边界，离屏为独立无头 profile 的页面渲染/合成 IPC；不替代原生录制/hook/焦点/DPI/外部粘贴验收。ignored 大图/性能未重跑，B-01/B-02 不变。

## 第二十批：前端详情与浮窗状态所有权

- 核对 handoff 并保留所有此前未提交修改。快照 `.tools/lint-batch20-baseline/{MainWindow,SettingsView}.vue`；本批仅修改 MainWindow，新增 `src/itemDetail.ts`、`src/compactPreview.ts` 与状态检查脚本。不新增依赖、状态框架、检查配置或 lint 豁免，沿用 Vue refs/nextTick 和既有 IPC。
- `createItemDetail` 独立持有详情、错误、loading、revealed 与代号。保留 conceal 递增并清正文/原文/loading、单独 invalidate 只递增的区别；普通详情仍按代号提交，reveal 仍同时核对代号与当前 ID。错误文案、过期 catch/finally 拒绝、紧凑模式隐藏、图片跳过文本读取、滚动归零时点均保持。列表筛选/变更/删除和卸载仍在原位置使详情失效，未将协议两种读取合并成模式参数。
- `createCompactPreview` 独立持有 floatingActive、内容请求代号、lastFloatingId、动画帧与几何签名。保留 nextTick 调度先于内容去重、相同 ID 省略 IPC、仅当前非空 ID 失败报告并允许重试、null 隐藏失败不污染提示，以及几何失败清签名重试。锚点仍裁剪到选中行与列表可见区域，记录元素查询继续复用主窗口原实现。
- MainWindow 保留列表选择、外观、焦点/隐藏事件、watch 注册与卸载编排；仅用明确控制器方法替换原直接代号/缓存访问。模板、监听注册和事件处理顺序不改，卸载仍在原位置取消既有帧、暂停浮窗、清详情、解绑并最后使代号失效。没有新增自动 watch/onUnmounted 或 disposed 门改变原调度。SettingsView 与快照逐字节一致；本批不把紧耦合的录制/修订号/检测状态混入详情拆分。
- 新增常规命令 `node scripts/check-main-preview-state.mjs`，**9 项通过**。使用真实 Vue refs/nextTick、现有 TypeScript 配置转译生产模块，仅替换 IPC 导入并模拟动画帧；类型正确性另由完整 vue-tsc/build 检查。覆盖迟到原文成功、旧 reveal/普通详情失败与新 loading/error 隔离、图片/紧凑短路、两个实例代号隔离、内容/几何去重、当前/过期浮窗失败、几何重试、隐藏 null 与取消既有帧。测试只创建项目 .tools 下独占临时模块，验证父目录且仅在本次 mkdir 成功后清理，不打开桌面窗口或操作剪贴板。
- 初次状态测试在终止清理后继续调度，错误假定帧取消入口可重新启用；调整为先校验隐藏几何、再验证最终取消，未改产品逻辑迎合测试。失败证据 `.tools/lint-batch20-preview-state-initial.log`，最终 `.tools/lint-batch20-preview-state.log`；没有重试相同错误假设或掩盖失败。
- MainWindow **654 → 608 行**，新增两个模块 **61/62 行**；TypeScript AST 统计 13 个函数声明（含两个状态工厂），最多 **39 个非空行**，`.tools/lint-batch20-structure.json`。人工复核新增控制流不超过两层，无 any、模式布尔或全局状态单例。主窗口尚承担列表/事件协调，不因仍较长而机械拆模板，也不宣称全部前端职责已审查完毕。差异 `.tools/lint-batch20-main-review.diff`；模板、SettingsView 与原 watch 注册不变已核对。
- 最终 fmt、全目标 clippy（`-D warnings`）、ESLint（零警告）、类型检查/构建、差异检查通过；Rust **689 通过、0 失败、3 ignored**；状态回归 **9 项通过**；固定构建后的 T3/T4/T7/U1/U2 离屏 **13/18/20/20/104 项通过**。日志 `.tools/lint-batch20-{fmt-final,clippy-final,rust-final,eslint,build,preview-state,t3,t4,t7,u1,u2,diff}.log`。本批 Rust 未修改，仍执行了常规全量检查；既有 OpenSSL 缺 ossl_static.pdb 的 LNK4099 提示未通过配置或源码掩盖。
- 当前仍剩 **3 大部分**：① 前端设置录制状态职责复核（详情/浮窗子项本批完成）；② 其余 Rust 职责/长文件及错误路径的最终审查，仍以第十九批 14 个长函数候选为线索，含明确保留项；③ 全量回归、覆盖边界和文档收口。下一批优先 SettingsView 的 session/epoch/revision/operations/disposed 所有权与检测延期，不改录制输入隔离和冲突优先级。
- 未接管键鼠、操作桌面剪贴板、修改 manual/真实偏好、提交或推进 U3。离屏使用独立无头 profile、实际页面渲染及合成 IPC，状态检查不是原生窗口验收；原生 hook/焦点/DPI/外部粘贴和 ignored 大图/性能本批未重跑，B-01/B-02 不变。整体优化仍未结束。

## 第十九批：hook 线程生命周期与错误处理

- 核对 handoff 并保留此前全部未提交修改，基线仍为 `master` / `b420bdf`。快照 `.tools/lint-batch19-baseline/hook.rs`；本批仅修改 hook 根模块，新增 `hook/lifecycle.rs` 与对应测试。未新增依赖、检查配置、lint 豁免或 IPC 字段。
- 线程装配、物理修饰键播种、钩子安装、线程队列发布、启动回复、消息循环及卸钩移至 lifecycle，根模块重导出 start/stop，保持外部路径。正常启动仍先发布 RECEIVER，在线程中采样/播种，再安装钩子、建立消息队列、Release 发布 THREAD、发送启动成功，之后才读取消息。GetModuleHandleW 失败仍允许原有 None 回退，但增加上下文诊断；安装错误保留原顶层中文提示及底层错误来源。
- Registration 通过 RAII 持有已安装钩子，serve 在启动回复和整个消息循环期间持有资源。原先启动回复接收方退出后仍留下线程/钩子的异常路径改为立即清理并记录错误；正常退出、读取失败和回复丢失都先 Release 清零 THREAD，再尝试 UnhookWindowsHookEx。卸钩失败只记录上下文，不重试、不触发 UI，不声称系统已成功释放资源。
- GetMessageW 的 -1 立即捕获系统错误并报告，0 正常结束，正值才分发。取消线程消息仍先于 TranslateMessage/DispatchMessage；PeekMessage=false 表示没有消息，TranslateMessage=false 表示未生成字符消息，二者不当作 API 失败。启动安装错误回复、成功回复丢失、退出投递、取消投递和卸钩失败均有诊断。
- 修正 THREAD=0 的退出边界：PostThreadMessageW(0, WM_QUIT) 会投递到调用线程自身，不能代表“线程未启动”。stop 仍先清 READY，再读取 THREAD；0 直接跳过投递，非零保持原消息/参数和一次投递。cancel_panel 仍先设置 CANCEL_PANEL，再读线程并投递；失败仅补诊断，标记仍供原回调兜底，不回滚、不清配置门。
- callback/event 与 Machine 段逐字对照本批快照未变，保持无新增分配/锁/IO/焦点修改/同步等待约束；没有把非回调诊断放进键盘处理路径。原子内存序、录制 session/ARMED、粘贴代号和取消标记的消费规则未改。
- 新增 **7 项长期测试**：启动成功回复先于读消息并最终释放资源；接收方退出时释放且不读消息；读取错误停止分发并释放资源；正常退出不分发旧消息；卸钩成功/失败都先清线程 ID；0 不投递且非零保留消息并传播错误；真实私有线程队列验证 0 不退出调用线程、取消消息先分发、WM_QUIT 正常结束。测试不安装钩子、不注入键盘；实际系统卸钩/安装故障用合成资源和回调测试其处理边界，未实注入 OS 故障。
- 最新词法扫描 `.tools/lint-batch19-inventory-final.json`；新增/修改生产函数 **15 个候选，最多 15 个代码行**，明细 `.tools/lint-batch19-structure.json`。人工复核新增控制流不超过两层，无新增生产 unwrap/expect、模式布尔或运行时动态分发。原 hook::start 不再为 >40 行候选，余下全项目候选 14 个，包含已有保留理由的迁移/声明表/事务/系统边界编排，不能视为 14 项确认违规。差异 `.tools/lint-batch19-hook-review.diff`。
- 最终 fmt、全目标 clippy（`-D warnings`）、ESLint、类型检查/构建、差异检查通过；Rust **689 通过、0 失败、3 ignored**；固定构建后的 T3/U1/U2 离屏 **13/20/104 项通过**。证据 `.tools/lint-batch19-{fmt-final,clippy-final,rust-final,eslint,build,t3,u1,u2,diff}.log`。首次 clippy 指出 PeekMessage must_use 与不必要的 Drop bound，修正为有语义注释的显式忽略及无 Drop bound 的泛型资源参数后重跑通过，保留 `clippy-initial.log`；未放宽检查。既有 OpenSSL 缺 ossl_static.pdb 的 LNK4099 提示仍在。
- 剩余仍为 **3 大部分**：前端详情/浮窗/设置录制状态职责；其余 Rust 职责/长文件与错误路径复核（本批 hook 线程生命周期子项已完成）；最终全量回归与记录收口。下一批优先前端独立状态所有权及异步代号/卸载清理，避免仅按文件长度搬移。未接管键鼠、操作桌面剪贴板、修改 manual/真实偏好、提交或推进 U3。私有消息队列和离屏不代表原生 hook/焦点/DPI/外部粘贴或大图性能验收；这些场景未重跑，B-01/B-02 不变。

## 第十八批：hook 回调与纯状态机专项

- 保留此前全部未提交修改，修改前快照 `.tools/lint-batch18-baseline/hook.rs`。本批只修改 `hook.rs` 并新增职责模块 `hook/event.rs`；未改 IPC、键位接受范围、线程生命周期或真实设置，未增依赖、配置或 lint 豁免。
- `Machine::step` 分出活动面板动作、吞键配对及投递失败后的配对撤销。保留先过滤注入/越界键、更新 down/释放配对、计算修饰键、处理活动/开启状态、最后补重复按下配对的顺序；修饰键缺失优先 Commit、Esc 优先于自定义触发键、触发键非重复才 Next、方向键可自动重复。录制与面板共用同一修饰键折叠算法，无新增键名表。
- 原 callback 拆入 `hook/event.rs`，固定大小 HookEvent 保存本次采样，分出状态编排、录制结果、动作投递和事件收尾。负 code 仍立即 CallNextHookEx；logical 采样后更新 PHYSICAL，然后线程局部读取/写回 Machine。取消标记 → 配置门 → 空闲时读快捷键 → 先记旧 down → session/前台录制检查 → logical 校正及状态机 → 录制覆盖 → panel 诊断 → 动作投递 → panel/keyboard 诊断 → 按 reserved/active 释放门 → 写回状态的外部可观察顺序保持。
- Open 仍在消息投递前递增粘贴代号，其他动作读取现有代号；投递失败只关闭面板、清本键 swallowed，并使本次透传，其他已吞键仍等待配对释放。录制投递拒绝仍吞本次录制键并清面板动作；publish_recording 的会话/ARMED 比较与失败重试行为未改。回调新增路径只有栈值、引用和静态分发，没有堆分配、锁、文件 IO、窗口/焦点修改或同步等待；保留既有无阻塞诊断消息路径，没有添加通用日志。
- 新增 5 项长期回归：Open 投递失败后重复/释放透传；导航失败只清本键配对；成功/无动作/越界事件不改变投递规则；Commit 失败保留另一已吞键的释放；旧录制会话拒绝后仍吞键并覆盖面板动作。测试只运行纯状态机、注入的 post 回调和受 CONFIG_TEST_LOCK 保护的录制原子值，不安装钩子、不注入键盘。
- 临时新旧纯状态机对照 **64 条轨迹、262,144 个事件**，逐步比较 swallow/action、active、全部 down/swallowed 数组，穿插录制事件编码；结果一致。覆盖不同修饰组合、自定义触发键、注入/禁用/逻辑修正、重复与越界键。证据 `.tools/lint-batch18-equivalence.log`；旧片段与入口副本 `.tools/lint-batch18-{old-machine,equivalence}.rs`。临时入口已从源码移除，不计入正式测试数；有限轨迹不能证明所有输入，也不替代原生 hook 时序/性能验收。
- 复用第十三批词法扫描器更新 `.tools/lint-batch18-inventory-final.json`，屏蔽字符串/注释及 cfg(test) 条目；新增/修改生产函数 12 个候选最多 **39 个代码行**，`.tools/lint-batch18-structure.json`。人工核对新增控制流不超过两层；无新增生产 unwrap/expect 或模式布尔参数。事件模块仍在 300 行建议内；根 hook 的测试与线程职责未借机全量搬移。对照差异 `.tools/lint-batch18-hook-review.diff`。
- 最终 fmt、全目标 clippy（`-D warnings`）、ESLint、类型检查/构建通过；正式 Rust **682 通过、0 失败、3 ignored**；固定构建后的 T3/U1/U2 离屏 **13/20/104 项通过**；差异检查通过。日志 `.tools/lint-batch18-{fmt-final,clippy-final,rust-final,eslint,build,t3,u1,u2,diff}.log`。正式检查在临时对照入口移除后运行；既有 OpenSSL 缺 ossl_static.pdb 的 LNK4099 提示保留。
- **本批后剩余 3 大部分**：① 前端详情/浮窗状态与设置录制状态职责复核；② 其余 Rust 长文件/职责及错误路径复核，包含 hook 线程 start/stop 的静默发送、消息读取和资源释放，尚未声称整个 hook 完成；③ 最终全量回归、覆盖边界核对与文档收口。词法扫描仍有 15 个 >40 代码行函数候选，含已明确保留的迁移/声明表/事务和系统边界编排；不是 15 项确认违规，更不是 15 批，后续需逐项标为整改或有理由保留，不机械拆分。
- 未接管键鼠、使用桌面剪贴板、修改 manual/真实偏好、提交或推进 U3。原生剪贴板测试沿用私有窗口站，离屏使用独立无头 profile 与合成 IPC；未重新原生焦点/DPI/实际外部粘贴或 ignored 大图/性能验收，B-01/B-02 不变。下一批建议先复核 hook 线程生命周期错误链，再转前端独立状态职责；整体优化尚未完成。

## 第十七批：详情、密钥、粘贴焦点与录制入口

- 继续保留 `master` / `b420bdf` 工作区此前全部未提交修改，不新增配置、依赖或 lint 豁免。修改前快照 `.tools/lint-batch17-baseline/`；只格式化本批涉及文件。
- 新增 `storage/detail.rs`，普通详情 `display_detail` 与显式原文 `reveal_detail` 分开，共用先读取 sensitive/view、后加载载荷的顺序。敏感坏 JSON 普通详情仍报错、显式原文不解析；非敏感坏 JSON 仍可显示。原敏感命中解析/手工全量回退、20000 字符截断及敏感文件路径隐藏保持；reveal 的主窗口/non-smoke 权限、IPC 字段、三态/TTL/索引均不改。
- DPAPI 与主密钥持久化移至 `platform/key.rs`，通过根模块重导出保持原 API 路径。加密/解密明确为两个入口，共享 blob 构造和输出复制/清零/释放；保留 UI 禁止标记、描述、错误文案、32 字节验证、已有数据库缺 key 拒绝生成、create_new/write_all/sync_all。LocalFree 失败只补无密钥内容的诊断，不改变成功结果；未注入真实释放失败。原六项 DPAPI/文件保留测试随职责迁移，测试目录改为 PID + 纳秒并拒绝已存在目录。
- 粘贴明确区分保持当前前台与恢复目标焦点，仍在 Prepare 阶段读取 main，并先验证外部目标、再短路权限检查、按请求恢复、最后轮询；十轮失败的最后一次仍等待 10ms。阶段顺序与粘贴代号校验不变，IPC restore/plain 字段保留。当前代号的 PasteReport 发送失败补上下文诊断，不重试、不记录错误内容；过时代号仍不发送。
- 录制明确分为 start/stop，仍先递增 session。开始依次写入 UNTIL/ARMED/OWNER，停止依次清 OWNER/ARMED 且不清 UNTIL；原 Acquire/Release/AcqRel 保持。命令权限、配置保留门、取消旧粘贴、停止前掩码及日志顺序未变，未改无分配/锁/IO 的 hook 回调。
- 新增 3 项长期测试：坏 JSON 详情/reveal 差异、缺失 ID 和状态/索引不变；首次/延迟/末轮成功与十轮失败的 probe/pause 顺序；录制开始/停止代号、原子状态与停止保留期限。录制测试使用原 CONFIG_TEST_LOCK，断言前恢复全部录制原子状态；焦点测试只驱动回调，无输入注入。
- 新拆模块及粘贴任务共 16 个生产函数词法候选，最多 29 个非空行，记录 `.tools/lint-batch17-structure.json`；不是 AST/圈复杂度证明。人工对照新增控制流不超过两层，无新增生产 unwrap/expect、模式布尔入口或重复键名/详情转换实现。hook/paster 对照保存在 `.tools/lint-batch17-*-review.diff`。
- 最终 fmt、全目标 clippy（`-D warnings`）、ESLint、类型检查/构建通过；Rust **677 通过、0 失败、3 ignored**；固定构建后的 T3/T7/U1/U2 离屏 **13/20/20/104 项通过**；差异检查通过。证据 `.tools/lint-batch17-{fmt-final,clippy-final,rust-final,eslint,build,t3,t7,u1,u2,diff}.log`。首次 clippy 发现新增测试的 useless_vec，改用数组后重跑通过，保留 `clippy-initial.log`。Rust 链接仍有 OpenSSL 缺 `ossl_static.pdb` 的既有 LNK4099 提示，未改工具链/源码掩盖。
- 未接管键鼠、使用桌面剪贴板、修改 manual/真实偏好或提交；Rust 原生剪贴板用例在私有窗口站，离屏为隔离无头 profile 与合成 IPC。未重跑原生焦点/DPI/外部粘贴、ignored 大图性能，不将离屏视为这些验收。B-01/B-02 不变，不推进 U3；下一批仍需专项审查 hook callback/Machine（严格保持无分配/锁/IO）和组件详情/浮窗状态职责，整体结构优化与最终收尾尚未完成。

## 第十六批：采集预算与存储批量事务

- 核对第十五批 handoff、检查证据与未提交工作区，保留全部既有修改；沿用原依赖、工具与 lint 配置，不推进 U3。
- 将单轮原生采集与格式收集合并分别拆到 `clipboard/capture.rs` / `collection.rs`。FormatBudget 持有 total/limit/has_v5，选择入口不再传五个散参；读取错误与读取后预算失败复用最大超限记录更新。格式顺序、规范化前计费、V5 成功后才跳过 DIB、失败回退、Preferred DropEffect 短数据/普通错误、CF_HDROP 预算内补 Copy，以及超限/Invalid/None 优先级保持。
- 原生路径保持打开前自写回/LAST_SEQUENCE 与序列门、一次 OpenClipboard、打开后重采样、格式/来源元数据、忽略程序门、位图标志、排除/历史/所有权标记及内容读取顺序。隐私历史标记不可读仍跳过；所有权标记不可读仍仅跳过标记。OpenGuard/CaptureLog 释放顺序、worker 退避、采集后的序列确认与上下文装配未改变；日志只用固定格式和元数据。
- 敏感状态接口保留 value 为目标状态，内部明确拆为整批预读与 PreparedSensitivity、标记、取消标记及摘要恢复。取消标记也先完整读取/检测；同一批次仍先去重/跳过缺失，再开一个事务、取单个 now 并统一提交。保留 JSON 对象校验、三态/TTL、搜索/拼音索引和摘要/命中回滚；未把原摘要与保存时空文件列表回退行为粗略合并。
- 删除拆为事务删行与提交后维护，仍逐 ID 参数绑定，不受 IN 数目限制，重复 ID 只计实际删除。候选在事务内收集、语句释放后提交，再按 BTreeSet 顺序复核共享引用并回收文件，严格 >200 才 VACUUM。提交后的引用检查/文件/VACUUM 错误进入 DeleteReport，不变成 Err 或事务内删文件；保护策略及已删除上下文报告未变。
- 新增 4 项长期测试：规范化前预算与剩余预算；最大超限/并列首次类型优先于 Invalid；载荷预读失败时标记/取消标记均不开始写入；在独立 SQLCipher 连接用 authorizer 拒绝 VACUUM 所需 ATTACH，验证维护失败只进报告、删除已提交且重开后仍已删除。原有事务回滚、共享 blob、锁定文件及私有窗口站采集用例继续通过。
- 修改前快照 `.tools/lint-batch16-baseline/`。临时纯合并对照共 **14,336 组**，读取顺序、剩余预算、保留格式字节及错误类型/actual/limit/image 均一致；证据 `.tools/lint-batch16-equivalence-final.log`、`.tools/lint-batch16-equivalence-result.json`。入口已从 tests 移除，副本 `.tools/lint-batch16-equivalence.rs` 保留，1 项对照不计入正式测试数。对照中的诊断为无 IO 接收器，原生来源/时序由代码核对及私有窗口站用例验证；有限样本不代表全输入形式化证明。
- 新模块 126/173 行；27 个新增/修改生产函数词法候选最多 34 个有效行，`.tools/lint-batch16-structure.json`。人工核对新增控制流未超过两层，未新增生产 unwrap/expect 或布尔模式入口；只是保留目标状态和 has_v5 等实际数据。仅格式化本批文件，未全项目改写。
- 最终 fmt、全目标 clippy（`-D warnings`）、ESLint（0 错误/0 警告）、类型检查/构建通过；正式 Rust **674 通过、0 失败、3 ignored**；固定构建后的 T2/T6/T7/U2 离屏 **18/8/20/104 项通过**，差异检查通过。最终检查和正式全量 Rust 均在临时对照入口移除后完成，证据 `.tools/lint-batch16-{fmt-final,clippy-final,rust-final,eslint,build,t2,t6,t7,u2,diff}.log`。
- 初次临时对照编译失败：用 path 导入 image_normalize.rs 时，其子模块 dib 被相对测试入口解析，未找到文件。只修正临时夹具的路径，并使用本地生成的生产片段显式指向真实 DIB 模块；未改产品源码迎合夹具。失败日志 `.tools/lint-batch16-equivalence.log` 保留，最终对照通过。Rust 链接仍有缺少 `ossl_static.pdb` 的既有 LNK4099 提示，未改工具链或检查规则掩盖。
- 没有接管键鼠、使用桌面剪贴板、修改 manual/真实偏好或提交。Rust 原生剪贴板用例使用私有窗口站；离屏使用隔离无头 profile 与合成 IPC。没有重新原生焦点/DPI/实际外部粘贴或 ignored 大图/性能验收，这些覆盖边界不因本批通过而关闭。B-01/B-02 不变，整体仍有其他结构整改与最终收尾回归；下一批继续详情/reveal、加解密、焦点恢复等内部模式，hook 回调按无分配/锁/IO 专项约束处理，组件职责仍待审查。

## 第十五批：DIB 解析与敏感检测

- 核对第十四批 handoff、工作区和检查日志，保留已有未提交改动；沿用原工具链，不新增依赖、配置或 lint 豁免，不推进 U3。
- DIB 规范化与预测布局拆到 `image_normalize/dib.rs`，通过 Header 分离 Core/Info、掩码、布局/载荷和 ICC；移除 parse_dib 的 predict 模式参数。length / pixels 分别保持规范化与预测接受范围：24 位位域仅供预测。保留各压缩/位深组合、正负高度、平面数、调色板项数/字节数、掩码连续/不重叠/位宽、行对齐、声明载荷和 ICC/溢出校验。
- 缩略图相关解码移到 `thumbnail/decode.rs`，独立拆分受限头读取、调色板/掩码偏移、32 位像素转换和 BMP 包装回退。继续拒绝 Core 头，保留 16384 边长与现有 MAX_PLAIN_BYTES 预算、自上/下行向、alpha 掩码和默认值；不把规范化对 planes/sizeImage 的校验强加到原 32 位解码路径。图像格式优先级、损坏回退和预览编码未改变。
- 敏感检测入口负责有界扫描与高熵补充，`sensitive/rules.rs` 负责 PEM/前缀/JWT。前缀使用长度/字符集枚举替代 exact 布尔与数字标记，严格只校验首个已识别前缀，失败不得尝试较短前缀；保持 PEM → 前缀 → JWT 顺序、扫描上限/UTF-8 取整、基于原文的左右词边界。JWT 对象/alg/签名、两点与第三点拒绝、base64url padding bits 均不变；高熵阈值、排除、重叠过滤及排序保持。
- 新增 6 项长期边界测试：24 位位域仅预测且仍验证掩码/载荷；预测行布局排除声明尾部/ICC 但验证整体范围；解码接受范围独立；长前缀失败不回退；PEM 优先且高熵只补非重叠命中；多字节扫描边界后的 PEM footer 不被消费。
- 修改前快照 `.tools/lint-batch15-baseline/`，临时对照检查副本 `.tools/lint-batch15-equivalence.rs`。最终对照 16,377 组 DIB 输入的裁剪字节/预测布局/解码 RGBA 像素一致；7,161 组敏感文本在高熵开关两种状态下规则、范围及脱敏结果一致；2 项对照测试通过，日志 `.tools/lint-batch15-equivalence-final.log`。临时入口已移出 tests，不计入正式测试数；这些有限样本不是所有字节输入的形式化等价证明。
- 新增三个职责模块为 207/170/260 行；本批新增/修改的 40 个生产函数词法候选最大 31 个有效行，`.tools/lint-batch15-structure.json`。人工核对新增控制流未超过两层，无新生产 unwrap/expect 或布尔模式参数；high_entropy 保留为用户配置数据。只格式化本批涉及文件，没有全项目改写或顺带重构 redact/编码路径。
- 最终 fmt、全目标 clippy（`-D warnings`）、ESLint（0 错误/0 警告）、类型检查/构建通过；常规 Rust **670 通过、0 失败、3 ignored**；固定构建后的 T1/T7/U2 离屏 **8/20/104 项通过**，差异检查通过。证据 `.tools/lint-batch15-{fmt-final,clippy-final,rust-final,eslint,build,t1,t7,u2,diff}.log`。对照入口移除后完成最终检查与正式全量 Rust 测试，未把对照项重复计数。
- 初次 clippy 检出移动缩略图解码时漏带局部 u32_at，已恢复对应模块的读取实现并检查范围加法，不修改检查配置；失败记录 `.tools/lint-batch15-clippy.log` 保留。最后 JWT 点计数降低嵌套后重新运行新旧对照通过。Rust 链接仍有缺少 `ossl_static.pdb` 的既有 LNK4099 提示，未改源码或工具链掩盖。
- 没有读取图片、接管键鼠、注册真实热键、使用桌面剪贴板或修改 manual/真实偏好。未重跑 ignored 大图/性能、原生图像剪贴板、焦点/DPI 验收；离屏只验证现有合成 IPC 页面，真实解析行为由 Rust 与对照用例覆盖。未提交，B-01/B-02 不变；整体结构整改与最终收尾回归仍未结束，下一批优先 clipboard 采集/预算合并、存储批量事务与内部模式，hook 回调按专项约束处理。

## 第十四批：broker 生命周期与消息分发

- 开始前核对 handoff、第十三批审查及验证日志、`master` / `b420bdf` 和未提交工作区，未发现阻塞；全部保留已有改动，不新增配置、依赖或 lint 豁免。
- 将长 broker 函数拆到 `runtime/broker.rs`（线程启动、窗口/监听/定时器/热键登记、消息循环与析构）和 `runtime/broker_messages.rs`（设置请求、录制及动作消息）；原生主热键 Registrar 随生命周期归入 broker。复核 `runtime::start` 的 State 声明与装配顺序后保留入口，不复制测试 State 构造或改变初始化时序。两个新模块共 17 个生产函数，词法候选最大 31 个有效行，记录 `.tools/lint-batch14-structure.json`；该口径不构成 AST/圈复杂度证明。
- 保留 250ms 定时器、注册初始热键后回复启动、设置请求 FIFO、取消请求不登记/不回复、冲突检测使用当前已登记热键、录制 session/active 门及掩码失败 packed=0。动作仍先解析再比较粘贴代号，Open 掩码先于控制派发，掩码失败先 Cancel 后 Report；非 Open 不注入掩码。
- 修复异常资源路径：按已取得资源回收部分初始化，定时器创建失败返回启动错误；启动回复接收方消失时清理退出；`GetMessageW=-1` 明确诊断，`0` 正常退出。临时探针先析构，再按监听 → 已登记主热键 → 定时器 → 窗口的原顺序清理，最后注销窗口类；退出清零发布句柄。清理错误记录后继续剩余释放，不用 UI 投递或恢复逻辑掩盖失败。
- 复用既有 `app_events::record/send_control`，补齐启动/设置请求回复、broker 控制消息、主热键 Toggle、错误/历史通知、文件状态事件及退出投递诊断。热键检测探针最终释放失败增加诊断。`TranslateMessage=false` 并非错误，保留丢弃并注释原因。发送失败不重试、不重新排队，不改变后续事件顺序。
- 新增 4 项 Rust 隔离测试：取消替换请求不调用原生登记/不回复；旧代号与未知动作不掩码/不派发；掩码先于 Open、失败先取消后报告及 Cancel 不掩码；控制通道缺失/断开返回错误。复用现有设置集成夹具与热键冲突/回滚测试，未创建产品窗口、注册真实热键、发送键盘事件或使用真实剪贴板。
- 最终 fmt、全目标 clippy（`-D warnings`）、ESLint（0 错误/0 警告）、类型检查/构建通过；Rust **664 通过、0 失败、3 ignored**，T3/U1/T8/U2 离屏 **13/20/14/104 项通过**。最终 Rust 在原生 Registrar 归位后全量复跑。证据 `.tools/lint-batch14-{fmt-final,clippy-final,rust-final,eslint,build,t3,u1,t8,u2}.log`；差异检查 `.tools/lint-batch14-diff.log`。未对整个项目格式化，仅处理本批 Rust 文件。
- 初次 clippy 检出移走 broker 后的生产未使用导入，改为测试条件导入；随后发现新增探针诊断需要 String，已转换，不放宽规则。早期记录 `.tools/lint-batch14-clippy.log` 保留；第一次失败的 `clippy-final` 路径后来由最终成功记录覆盖，本条明确保留失败事实。原生 Registrar 最终归位后的 clippy 等待已有测试构建锁，未并行启动第二套测试。最终链接仍有缺少 `ossl_static.pdb` 的既有 LNK4099 警告。
- 系统级窗口/监听/定时器创建、启动回复及资源释放故障没有逐项实注入，资源路径为代码审查；没有新增原生焦点、多显示器 DPI、smoke 窗口或实际外部粘贴验收，未重跑 ignored 重型项。离屏和隔离测试不能替代上述覆盖。未接管键鼠、修改真实偏好、提交或推进 U3；B-01/B-02 不变。
- 本批 broker 整改完成，整体仍剩结构整改与最终收尾回归。下一批优先 `sensitive::detect` / DIB 规范化与解码，再按审查清单处理内部模式、hook/采集/存储及组件职责；不得把零告警或本批通过视为整体优化结束。

## 工具接入与第一批

- 用户已授权补充 ESLint 及依赖，暂不引入 Prettier。
- 新增 `eslint.config.js`、`npm run lint`，采用 JS/TypeScript 推荐规则及 Vue essential 规则，支持 Vue 模板和 TypeScript 脚本解析。
- 结构指标暂为警告：复杂度 10、块深度 3、参数 4、函数 50 行（忽略空行和注释）；测试脚本不套用结构指标。未关闭基础错误规则。
- 初始全量结果实际为 **51 错误、10 警告**；口头进度中的 52 错误为计数误差，以 JSON 为准。
- 删除测试脚本中 32 个未使用的辅助函数，清理 12 处无用转义。未修改产品逻辑。
- 当前全量结果为 **8 错误、10 警告**；`scripts` 与 ESLint 配置检查通过。基线与当前明细位于 `.tools/lint-{baseline,current}-eslint.json`。
- 前端构建通过，T3/T4/T6/T7/T8 离屏脚本通过，日志为 `.tools/lint-batch1-{build,t3,t4,t6,t7,t8}.log`。U2 结果另行补记。
- U2 全量离屏回归退出码 0，日志 `.tools/lint-batch1-u2.log`。
- 本轮尚未重跑 Rust 测试。先前只读基线 fmt、全目标 clippy、前端构建通过，不能等同于满足全部结构规范。

## 待处理清单

### 剩余范围（第十三批更新）

按职责拆为 5 部分，第七批完成第 1 部分，第八至十批完成第 2 部分本轮错误链与职责拆分，第十一至十二批完成第 3 部分计划内入口及其他生产错误链；当前剩余第 4～5 共 2 部分。部分大小不同，不等于剩余批次数，也不据此估算完成百分比；原生覆盖边界仍保留，整体手工审查仍归第 4 部分。

1. 浮窗状态与定位（本批完成）：清除生产 unwrap/expect、传播锁错误、保持 generation 和锁外回调，拆开定位与内容加载；最终验证见第七批记录。
2. 运行时错误链与分工：第八批处理设置保存与回滚，第九批处理采集/清理/粘贴线程，第十批处理面板控制线程、快照发布及启动错误链。生产 unwrap/expect 已清零；保持 capture_context → storage 顺序，不在系统设置变更后漏掉回滚。验证与尚未覆盖的原生异常见各批记录。
3. 应用入口及其余生产错误处理（本轮计划已完成）：第十一批处理入口读取，第十二批处理托盘通道、窗口事件、smoke 错误、设置字段容错解析与快捷键、已保存敏感 JSON 转换，并补运行时窗口回调错误处理。详见下方覆盖边界。`paste_bench.rs` 是显式运行的基准工具，归手工审查独立区分，不当成产品路径机械清零。
4. 整体结构与手工规范审查（第十三批已盘点并完成第一组整改，仍未收口）：前后端长函数/文件职责、布尔模式参数、重复实现、嵌套和吞错。详细已改、保留和待整改见 [结构与手工审查](lint结构与手工审查.md)。下一批优先 broker 生命周期/消息分发及仍静默处理的发送/清理，再处理 DIB、敏感检测等长函数与内部模式；工具零告警不等于本项完成。
5. 最终回归与收尾：最终代码统一运行格式、lint、类型/构建、Rust 和适当离屏回归，更新 handoff；原生 DPI、ignored 性能项沿用覆盖边界，B-01/B-02 不变。

### 第十三批进度：结构盘点与启动职责整理

- 核对第十二批 handoff 和工作区，保留已有修改。前端用现有 TypeScript/Vue 编译器盘点函数，Rust 用屏蔽字符串/注释/测试条目的词法候选扫描并人工阅读关键函数；初始按首个 cfg(test) 截断的统计已废弃，最终清单 `.tools/lint-batch13-inventory-final.json`、前端清单 `.tools/lint-batch13-frontend-inventory.json`，详细结论见 [结构与手工审查](lint结构与手工审查.md)。不宣称词法扫描证明圈复杂度或全局依赖图无环。
- `lib.rs::run` 分出数据准备、声明式注册和启动装配；`app_setup` 管窗口创建，`app_smoke` 管合成图片/就绪等待/报告，`app_tray` 管菜单与事件注册，`tray_status` 管图标和状态渲染。正常与 smoke 初始化采用独立函数。运行时暂停更新直接依赖渲染模块，不经入口转发；渲染模块不导入 runtime/app_events。三种窗口 builder 属性作拆分前后文本对照均一致，初始化顺序与原生准备顺序保持。
- 详情类型与转换移至 `item_detail.rs`，存储和浮窗直接依赖领域详情模块；保持字段、截断与文件存在性检查。`Shortcut` 拆完整标签/修饰键标签并复用片段；托盘拆普通/暂停图标。设置页拆取消录制与退出、首次热键检测与去抖检测，复用准备逻辑，保留 session/代号/延期语义。主窗恢复锚点和预览定位复用同一按 ID 查 DOM 的函数。
- 新模块非声明式函数最长约 28 个有效行，run 约 27 行；48 行 application_builder 主要为命令声明表，明确保留。前端函数候选最长约 27 行，长组件仍需职责审查，不据此认定全面合规。broker、DIB、检测器、采集预算等长函数与 protect/reveal/predict 等内部模式已记录为待整改，迁移与事务边界明确保留理由；未机械合并接受范围不同的两套 DIB 逻辑。
- 首次 clippy 发现搬移后的 preview 隐藏监听闭包返回 u64，已恢复原有分号丢弃返回值；保留 `.tools/lint-batch13-clippy.log`。后续无 lint 豁免或配置变更。既有 Rust 回归验证标签、详情与图标像素；不为代码搬移新增镜像测试。
- 最终 fmt、全目标 clippy（`-D warnings`）、ESLint（0 错误/警告）、类型/构建、差异检查通过；Rust **660 通过、0 失败、3 ignored**；T3 **13 项**、U1 **20 项**、T8 **14 项**、U2 **104 项/0 失败**离屏通过。最终日志 `.tools/lint-batch13-{fmt-final,clippy-final3,rust-final3,eslint-final,build,t3,u1,t8,u2,diff}.log`。早期 rust、rust-final、rust-final2 记录为逐步整改中的结果，最终以 final3 为准。
- OpenSSL 缺失 `ossl_static.pdb` 的 LNK4099 提示保持。未执行原生 smoke 窗口、焦点/DPI/真实外部粘贴或 ignored 重型测试；配置/顺序对照与离屏验证不替代原生验收。整体仍剩结构整改和最终回归两部分，未接管键鼠、修改真实偏好或提交，不推进 U3，B-01/B-02 不变。

### 第十二批进度：托盘、窗口事件及剩余生产转换

- 核对第十一批交接并保留已有改动。新增 `app_events.rs` 承接托盘控制发送、主窗移动/缩放/焦点/关闭事件；通道断开、窗口查询、定位、隐藏、通知与失焦主线程派发错误进入诊断。失败不盲目重试，不把窗口查询失败当作 false，隐藏失败不发布 `main-hidden`；保留延迟 120ms 复核前台归属、锁外窗口操作及原事件分工。
- 托盘提示/暂停菜单更新、录制结束掩码失败补诊断；前端 ready 的设置通知失败返回 IPC 错误并保留待打开标志。smoke 用可失败的命名线程创建并向 setup 传播；报告写入失败诊断且退出码为 1，不再仅凭 ready 返回成功。
- 补 `runtime/main_window.rs`：设置显示失败记录错误，通知成功才清除待打开设置标志；Toggle 查询错误返回，显示/恢复失败保留原激活尝试顺序后返回错误；隐藏成功才通知前端。运行时 Hide 在主线程执行 Tauri 隐藏后再通知，派发失败不提前发隐藏事件；提示显示失败保留待确认错误，快速面板取消发送失败明确诊断。Win32 `ShowWindow` 返回值表示先前可见性，未将其机械当作失败状态；原生激活拒绝仍按既有策略诊断，不重试抢焦点。
- 设置加载拆至 `settings/rows.rs`，`from_rows` 返回 `Result` 并由 `Storage::load_settings` 传播内部转换错误。用户损坏/缺失字段仍逐项回退，忽略名单保留独立条目校验、大小写去重、256 项上限及原日志顺序；旧 unified 热键迁移保留主热键，找不到替代组合返回错误。ASCII 键名使用范围内无失败转换。生产 unwrap/expect 清零。
- 敏感标记与取消标记将已存视图解析为 JSON Map，拒绝非对象并返回上下文错误；使用 insert/remove 修改键，错误传播使批量事务回滚。新增通道发送/断开测试，以及数组/null/数字/字符串损坏视图的批量取消标记回滚测试；原有设置字段矩阵、热键迁移和敏感读写回归继续通过。
- 首次 clippy 暴露拆分后 BTreeMap 导入遗漏，后续发现外观集成测试需解包新 Result，均已修正。首次 Rust 全量因 Map 转换后标记写入仍用索引新增键而发生 43 项相关失败，已改为 insert 后全量通过；保留 `.tools/lint-batch12-rust.log`，不将初次结果当通过。
- 最终 fmt、全目标 clippy（`-D warnings`）、ESLint（0 错误/警告）、类型/构建、差异检查通过；Rust **660 通过、0 失败、3 ignored**；T3 **13 项**、T7 **20 项**、T8 **14 项**、U2 **104 项/0 失败**离屏通过。最终日志 `.tools/lint-batch12-{fmt-final,clippy-final2,rust-final,eslint,build,t3,t7,t8,u2,diff}.log`。新增模块生产各 119 行，修改的运行时窗口模块 238 行，函数有效代码行均未超过 40；整项目长文件与布尔模式参数仍留整体审查。
- OpenSSL LNK4099 缺失 `ossl_static.pdb` 提示仍存在。没有注入真实窗口显示/派发、smoke 创建/写入系统故障；原生焦点/DPI/实际粘贴和 ignored 性能未重新验收，诊断在启用时落盘，不声称这些系统异常均经实测。未接管键鼠、修改真实偏好或提交，B-01/B-02 不变，不推进 U3。

### 第十一批进度：入口读取与初始化错误链

- 核对 handoff 与已有未提交修改后继续入口子批。`get_settings/startup_error/panel_state/last_error` 返回 `Result`，锁中毒通过 IPC 错误传给现有前端捕获；正常成功载荷不变，错误消费仍只取一次，不恢复中毒锁。
- `frontend_ready` 先读取条目再发布 quick ready 标志，避免读取失败后部分更新；粘贴缺少目标时的热键提示、主窗布局读取传播设置锁错误。初始化的启动缓存锁和图标解析失败向 setup 返回；托盘图标更新、隐藏控制发送、缩略图读取/响应构建、smoke 报告写入失败增加诊断。HTTP 构建失败返回 500 空响应；正常 MIME、缓存和 nosniff 语义通过原有测试。
- `lib.rs` 生产 unwrap/expect 清零；没有宣称托盘和窗口事件所有错误链已收口。未处理的控制发送、窗口事件 best-effort 调用、smoke 线程创建/报告失败退出语义，以及 settings/storage sensitivity 的解析，保留下一子批审查；整体长入口职责审查仍属第 4 部分。
- 复用唯一 Tauri 测试应用，新增四类入口锁中毒和一次性错误消费场景，保留持有存储/采集锁时发布及读取面板的超时检查。首次全量测试发现直接序列化新 Result 的旧测试不匹配，改为解包成功载荷后通过；新增场景并入已有测试，因此条目数不增加。
- 初次未加载 `scripts/env.ps1` 导致 OpenSSL 找不到 Perl，随后按 handoff 加载当前进程环境并完成依赖重建，未修改工具链配置。最终 fmt、全目标 clippy（`-D warnings`）、ESLint（零警告）、类型/构建通过；Rust **658 通过、0 失败、3 ignored**，U1 **20 项**、T3 **13 项**离屏通过。日志 `.tools/lint-batch11-{fmt,clippy,eslint,build,u1,t3}.log`；最终 Rust 为 `.tools/lint-batch11-rust-final.log`（包含新增异常场景），此前 `rust.log` 为新增场景前复跑。
- OpenSSL 缺少 `ossl_static.pdb` 的 LNK4099 提示仍存在。未注入系统图标解析/HTTP builder/文件写入故障，未重新原生 DPI/焦点/外部粘贴验收或运行 ignored 性能项；未接管键鼠、修改真实偏好或提交。B-01/B-02 不变，不推进 U3。

### 第十批进度：面板控制、快照发布及启动错误链

- 从运行时拆出 `runtime/panel_controller.rs`（控制事件、面板会话和提示期限）与 `runtime/main_window.rs`（主窗口/快速面板/提示窗口的主线程操作），事件按枚举穷尽分发。沿用原窗口显示/激活顺序、无 Alt 注入的原生激活方式、旧热键录制会话拒绝和提示期限。
- 面板快照、历史快照、错误缓存、采集门/上下文及启动错误缓存的锁中毒均返回具体错误，不恢复中毒锁。`runtime.rs` 和新增模块生产 unwrap/expect 清零。快照先在内存锁内复制，释放锁后 emit；不读数据库、不等待采集/存储锁，原有持锁时发布/读取快照的有超时测试继续验证。
- 错误确认仅清除仍匹配的消息，窗口回调在原生调用之后再短暂获取错误缓存锁；暂停先取得所需锁再改变状态，释放采集门和上下文锁后更新托盘。主线程派发失败不再静默丢弃，返回调用处处理；启动初始化的错误存储向上返回错误。
- 新增异常行为：控制事件失败后取消本地会话、清空条目和提示期限、更新粘贴代号，再尝试发布空快照与安排隐藏；避免旧 Commit 使用失效条目。排队显示回调因新代号失效，失败隐藏回调也检查代号，避免迟到时隐藏后续新面板。无法派发/发布时记录诊断，不循环投递同一失败。
- 新增 2 项独立 Rust 测试，覆盖快照中毒、错误确认不清除新消息、错误/启动缓存中毒；复用唯一 Tauri 测试应用的控制夹具覆盖旧粘贴错误/Commit 拒绝、当前错误及 Toggle 的错误缓存中毒、历史快照中毒、导航发布中毒、失败取消后的旧 Commit 不入队、暂停上下文中毒与采集门释放。夹具不创建产品窗口，不调用真实窗口显示/焦点函数或读取真实剪贴板。
- 新模块生产代码为 `main_window.rs` 207 行、`panel_controller.rs` 292 行，函数最多 29/34 个有效代码行；统计 `.tools/lint-batch10-structure.txt`。根运行时的长 broker/初始化等仍需在整体结构审查中结合职责判断，不因生产 unwrap/expect 清零宣称结构审查已完成。
- 首次 clippy 检出拆分后 `Manager` 仅在测试使用，已改为测试条件导入，未新增 lint 豁免。最终复跑曾因旧设置夹具仅按 PID 命名而撞上已存在目录，保护断言在进入产品逻辑前拒绝覆盖；保留 `.tools/lint-batch10-rust-final.log` 失败证据，改测试目录为 PID + 时间戳，未删除或覆盖旧目录。
- 最终 fmt、全目标 clippy（`-D warnings`）、ESLint（0 错误/0 警告）、类型/构建通过；Rust **658 通过、0 失败、3 ignored**，固定构建后的 U1 **20 项**、U2 **104 项/0 失败**、T8 **14 项**离屏回归通过，差异检查通过。证据 `.tools/lint-batch10-{fmt,eslint,build,u1,u2,t8}.log`；Rust 最新证据 `.tools/lint-batch10-{clippy,rust}-final2.log`，包含失败隐藏回调代号检查及夹具目录修正。
- Rust 链接仍有 OpenSSL `ossl_static.pdb` 缺失的 LNK4099 提示。没有实注入 broker/hook 启动或主线程派发系统故障，没有重新验收原生焦点/多显示器 DPI/提示激活/外部程序粘贴，未重跑 ignored 性能/大图；代码检查、隔离测试与离屏回归不替代这些原生覆盖。未接管键鼠、修改真实偏好或提交，不推进 U3，B-01/B-02 不变。

### 第九批进度：采集、清理与粘贴线程

- 从 `runtime.rs` 按职责拆出 `runtime/worker.rs`（采集与任务循环）、`runtime/maintenance.rs`（采集后清理与敏感过期线程）、`runtime/paste_job.rs`（粘贴阶段）。模块生产代码分别为 109/105/119 行，函数最多 30/20/24 个有效代码行；新增模块生产 unwrap/expect 为 0。测试状态构造移至仅测试编译的 `runtime/test_support.rs`，复用原设置测试夹具，不复制 State 初始化。
- 采集保存的上下文锁、维护策略/上下文/存储锁、粘贴读取/touch 锁改为返回错误；不恢复中毒锁。保留策略 → capture_context → storage 顺序、指纹锁外计算、保存成功后更新采集上下文，以及维护失败后重置计数和节流、保存/清理合并一次刷新。
- 隐私设置读取改为可失败回调，保留每次采集重试读取最新忽略程序名单；设置锁中毒返回新增内部错误 `SettingsUnavailable`，明确跳过本次复制，不使用空名单替代。策略层将该序列记为已处理且不对同序列重试；私有窗口站测试核对失败时载荷读取次数为 0。正常采集行为、产品 IPC 和配置格式未变。
- 保留粘贴六阶段顺序及原 `run_paste`：新代号优先、过期错误不报告、注入成功后的 touch 仍执行、读取和 touch 错误向阶段编排传播。格式诊断仍只输出固定支持格式名/长度，不输出正文或记录 ID。
- 采集门中毒时释放 pending 标记并报告，不立即反复排队；暂停路径及正常完成后的补排保持原语义。错误缓存自身中毒时 `report` 记录诊断后返回，避免工作线程在报告错误时再次 panic；控制线程内其余错误锁仍属于下一子批，不能据此称所有运行时错误链已完成。
- 新增 7 项 Rust 测试：维护锁中毒与健康锁释放、维护失败后节流、清理后采集上下文失效与空过期查询、粘贴读取失败在 Write 前停止及 touch 错误、旧代号停止且不报告旧错误、动态忽略名单及失败拒绝采集、保存锁中毒不提交上下文。原策略失败用例与私有剪贴板忽略用例各补充设置不可用场景，不计为额外测试条目。
- 首次检查发现新错误枚举漏补策略诊断匹配分支，已补全，无通配豁免。最终 fmt、全目标 clippy（`-D warnings`）、ESLint（0 错误/0 警告）、类型/构建通过；Rust **656 通过、0 失败、3 ignored**，固定构建后的 T4 **18 项**、T6 **8 项**、T7 **20 项**离屏通过，差异检查通过。日志 `.tools/lint-batch9-{fmt,eslint,build,t4,t6,t7}.log`、`.tools/lint-batch9-{clippy,rust}-verified.log`，结构统计 `.tools/lint-batch9-structure.txt`。
- Rust 链接仍有 OpenSSL `ossl_static.pdb` 缺失的 LNK4099 提示。未重跑 ignored 性能/大图、原生 DPI/焦点或实际外部程序粘贴；离屏及隔离 Rust 测试不替代这些原生验收。未发送键盘事件、使用真实剪贴板/偏好或注册真实热键；未提交、不推进 U3，B-01/B-02 不变。

### 第八批进度：运行时设置保存与回滚

- 将设置更新从 `runtime.rs` 按职责移入 `runtime/settings_update.rs`，沿用模块内状态及现有 broker/存储接口，未新增依赖或新功能。拆分系统设置变更、回滚、存储/策略提交、清理和保存通知；新模块 217 行，函数最多 38 个有效代码行。
- 设置与启动错误状态锁在系统变更前检查并持有；策略 → capture_context → storage 按原顺序获取，任一锁中毒或落库失败均先释放本次获取的存储侧 guard，再回滚已改动的自启动/热键。存储侧 guard 按 storage → capture_context → policy 顺序释放，策略锁持续覆盖清理。
- 本路径全部生产 unwrap/expect 清零，不恢复中毒锁。正常成功流程保留配置门、热键冲突预检、内存/持久化一致提交和清理失败只报告的行为。异常诊断补强：自启动变更失败且热键恢复失败时保留两层原因并报告启动错误；落库失败后如两种系统回滚均失败，同时保留两种错误，不丢失自启恢复原因。
- 复用原唯一 Tauri 应用与假 broker 的运行时设置测试，增加 7 个场景：设置/启动错误/策略/上下文/存储共 5 类锁中毒，策略锁中毒且热键恢复失败，以及成功提交并清除启动错误。核对旧热键恢复、数据库/内存/策略未被部分提交、健康锁及配置门已释放；成功路径再保存默认设置恢复测试中的全局配置。
- 锁中毒场景使用 `.test-data` 下独立子目录，不覆盖原数据库失败夹具；测试不注册真实热键、不发送键盘事件、不写真实 Run 项。自启动系统失败未实注入，原生自启动验收不据此判定通过。测试场景并入原有单项测试，因此 Rust 测试条目数不会增加 7。
- 首次 clippy 检出 `manual_inspect`，已改为 `inspect_err`，未豁免规则。最终 fmt、全目标 clippy（`-D warnings`）、ESLint（0 错误/0 警告）、类型/构建通过；Rust **649 通过、0 失败、3 ignored**（新增 7 场景并入原单项），T3 设置离屏 **13 项通过**，差异检查通过。日志 `.tools/lint-batch8-{fmt,clippy,eslint,build,rust,t3}.log`；结构统计 `.tools/lint-batch8-structure.txt`。Rust 链接仍有 OpenSSL `ossl_static.pdb` 缺失的 LNK4099 提示。
- 本批未改前端产品代码，未重复全量 U2 或原生窗口/DPI、ignored 性能测试。整体还剩 4 个职责部分，其中运行时部分本批只完成设置保存/回滚子项；不推进 U3，B-01/B-02 保持原状态，未提交。

### 第七批进度

- `compact_preview.rs` 全部生产 unwrap/expect 改为错误返回。IPC 返回具体状态错误；主线程回调、窗口事件和隐藏入口记录诊断，不恢复中毒锁，隐藏入口即使清空快照失败仍继续隐藏原生窗口。
- 保持旧 generation 结果拒绝、内容锁外回调、锁外窗口/事件调用、已报告高度记忆和几何去重。几何比较与写入改为一次锁内完成，方法返回后再通知窗口。
- 拆出窗口几何读取、纯锚点计算、布局应用、存储内容读取及主线程展示，复用原 placement 公式；显示侧读取设置后立即释放 guard，避免跨窗口调用持锁。
- 新增内容锁中毒、高度锁中毒、几何去重/中毒及多缩放左右布局/箭头边界 4 项测试；原有回调重入、旧内容与旧高度拒绝测试继续保留。
- 最终 fmt、全目标 clippy（`-D warnings`）、ESLint（0 错误/0 警告）、类型检查/构建通过；最终 Rust **649 通过、0 失败、3 ignored**，U2 离屏 **104 通过、0 失败**，差异检查通过。最终 Rust 证据是 `.tools/lint-batch7-{clippy,rust}-final.log`；`verified` 是最后一次隐藏入口卫语句调整前的记录。其他证据为 `.tools/lint-batch7-{fmt,eslint,build,u2}.log`。
- 结构自查：浮窗生产 19 个函数，最多 37 个有效代码行，生产 unwrap/expect 为 0；隐藏入口改为卫语句，减少嵌套。明细 `.tools/lint-batch7-structure.txt`。其余候选盘点保存在 `.tools/lint-batch7-unwrap-inventory.json`，该文本扫描不能替代后续逐处语义审查。
- Rust 链接仍有 OpenSSL `ossl_static.pdb` 缺失的 LNK4099 提示；没有改源码/工具链掩盖。未重跑 ignored 性能/大图、未做新的原生多显示器 DPI 及焦点验收，离屏和纯几何测试不能替代这些原生覆盖。整体优化尚未完成；未推进 U3、未提交、未接管键鼠、未修改真实偏好。

### 第六批进度

- 开始前核对 `master` / `b420bdf`、工作区、handoff 和第五批 verified 日志，未发现继续优化的阻塞；保留全部已有未提交改动。沿用 ESLint/Rust 工具链，不新增配置或依赖。
- `file_status.rs` 移除全部生产 unwrap/expect：线程启动返回带上下文的 `Result`，缓存读取和排队传播锁中毒错误，后台写回失败记录诊断并退出，不恢复中毒锁。列表命令与运行时初始化/刷新同步传播错误；刷新函数的存储、历史快照锁也改为返回错误。
- 保留 128 项队列、4096 项缓存、30 秒过期、同路径去重、敏感记录过滤和锁外通知。队列满时撤回 pending，后续可重试；线程断开时撤回 pending 并明确报错。异常路径由原先 panic/忽略断开改为已有错误通道返回，正常路径行为保持。
- 缓存读写统一采用 entries → pending 顺序；文件系统检查在锁外，通知在写回函数返回、全部 guard 释放后执行；未修改 capture_context → storage 顺序或浮窗回调。
- 新增 3 项异常测试：线程断开不残留 pending、entries 中毒拒绝读取/写回、pending 中毒拒绝排队/写回且不部分更新缓存。保留原有满队列、过期、去重、容量及真实后台通知测试。
- 本批 fmt、全目标 clippy（`-D warnings`）、全量 ESLint（0 错误/0 警告）、TypeScript/Vue 检查与构建通过；Rust **645 通过、0 失败、3 ignored**（含新增 3 项异常测试）；固定构建后的 T8 **14 项**、U2 **104 项/0 失败**离屏回归通过，`git diff --check` 通过。日志为 `.tools/lint-batch6-{fmt,clippy,eslint,build,rust,t8,u2}.log`。Rust 链接仍有 OpenSSL 缺少 `ossl_static.pdb` 的 LNK4099 警告，未修改工具链或源码掩盖。仅格式化本批涉及的 3 个 Rust 文件，未全项目改写。
- 未重跑 ignored 性能/大图测试或原生混合 DPI 验收；离屏使用隔离无头浏览器和合成 IPC，不将其视为原生文件系统/窗口验收。真实后台缓存行为由 Rust 测试覆盖；线程创建失败的系统级故障未实注入，线程断开与锁中毒路径已测试。
- 仍待处理：`runtime.rs` 其余采集/清理/设置/控制线程错误链、`compact_preview.rs` 浮窗状态锁、`lib.rs` 其余命令与初始化 unwrap，以及其他生产代码和长函数/长文件职责审查。整体优化尚未完成，未推进 U3，B-01/B-02 保持原状态；未接管键鼠、修改真实偏好或提交。

### 第五批进度

- `appearance.rs` 将工作区尺寸约束与位置约束独立成函数；布局锁中毒时经现有 Tauri 错误通道返回，不再 panic。保留原尺寸计算、窗口调用顺序和状态更新时点。
- 密钥派生返回 `Result`，存储初始化传播派生错误；设置对象序列化、字段读取和忽略路径解析增加上下文错误，移除对应生产 unwrap/expect。
- 同步调整密钥派生的单元、迁移与加密集成测试调用。早期检查曾发现误改测试断言及漏改集成测试调用，已修正；以 `verified` 日志为最终证据，不能把早期失败记录当作通过。
- 最终 fmt、全目标 clippy 通过；Rust **642 通过、0 失败、3 ignored**，日志 `.tools/lint-batch5-{clippy,rust}-verified.log` 与 `.tools/lint-batch5-fmt.log`。前端全量 lint 仍为 0 错误、0 警告。
- 本批未改前端，未重复离屏回归，未接管键鼠；窗口布局拆分未进行新的原生混合 DPI 验收。
- 仍待处理：`runtime.rs`、`compact_preview.rs`、`file_status.rs`、`lib.rs` 的锁与线程错误链；其他生产 unwrap 和长函数/长文件职责审查。必须保持 capture_context → storage 锁顺序，以及浮窗锁外窗口回调，不以无条件恢复中毒锁掩盖状态问题。整体任务尚未完成，未提交。

### 第四批进度

- 前端全量 ESLint 已达到 **0 错误、0 警告**，未放宽阈值。历史刷新拆成分页读取、选择恢复和滚动恢复；新增异步边界后在提交前复核刷新代号。键盘处理拆分窗口/输入/记录层，动作采用映射；录制失败处理保留会话与字段修订号校验。
- 前端构建通过；固定构建后的 T4/T5/U1/U2 回归通过，U2 为 104 项、0 失败。日志 `.tools/lint-batch4-{build,t4,t5,u1,u2}.log`。
- Rust 首批移除 clipboard/model/storage crypto 的 5 处生产 unwrap，保留短数据校验及既有解码语义。一次文本替换误碰测试断言导致编译失败，已还原测试断言并重跑成功。
- Rust fmt、全目标 clippy、常规测试退出码均为 0；日志 `.tools/lint-batch4-{fmt,clippy,rust}.log`。测试链接有 OpenSSL 静态库缺少 ossl_static.pdb 的 LNK4099 警告，未改源码掩盖工具链提示。
- 整体优化仍未完成：Rust 锁/线程错误传播、长函数与职责审查仍待处理；前端 ESLint 清零不代表文件长度、所有布尔模式参数和手工规范审查均已完成。未提交。

### 第二、三批进度

- 快捷键：拆分完整标签与修饰键标签，移除布尔模式参数；键位范围和标点采用表驱动；解析和录制编码拆分职责。与 HEAD 原实现比较 9,408 组输入，标签、校验及编码一致。
- 菜单：键盘动作改为按键映射，独立处理焦点移动与激活；相关文件 ESLint 无错误和警告。
- 设置页：拆出按字段生成冲突提示的逻辑，保留原提示优先级；文件名控制字符校验保留，仅对该行添加有原因的 no-control-regex 豁免。
- 主窗口：移除嵌套三元表达式，拆出 Enter 焦点判断、记录导航、键盘动作分发。该部分仍需进一步降低复杂度，未宣称结构优化完成。
- 当前全量 ESLint：0 错误、9 警告。警告数量因原长函数拆成多个仍超阈值的函数而变化，不能仅据数量判断改善；具体见 `.tools/lint-batch3-eslint.log`。
- 当前前端构建通过；T4、T5、U1、U2（104 通过、0 失败）回归通过，日志 `.tools/lint-batch3-{t4,t5,u1,u2}.log`。前一轮 U2 曾有 1 项页面 ready 超时，运行与重新构建重叠，原因尚不能确定；保留 `.tools/lint-batch2-u2.log`，不隐去失败。
- Rust 仅进行候选位置检索，尚未完成生产/内联测试区分与结构审查，未修改 Rust，未重跑 Rust 测试。

以下清单中的快捷键及菜单已完成本批处理；主窗口、录制函数及 Rust 为后续重点。

1. `settings.ts`：快捷键解析、标签和校验，拆除布尔模式参数及嵌套三元表达式，保留键位语义。
2. `ContextMenu.vue`：菜单键盘事件按键分发，保留焦点及组合输入保护。
3. `MainWindow.vue`：列表刷新、选择、键盘交互的复杂函数；按职责拆分并验证异步代号与选择稳定性。
4. `SettingsView.vue`：录制、冲突显示、应用名校验；控制字符正则用于输入校验，不能为消除 lint 直接移除校验。
5. Rust：区分内联测试和生产代码，审查 unwrap/expect、锁边界、长函数及重复实现。尚未完成全量清单，不能将文件总行数直接当作违规。
6. 最终全量 lint、格式、类型/构建、Rust 测试及离屏回归；更新 handoff，保持 B-01/B-02 原状态。

未接管键鼠、修改真实偏好或提交。用户原有 handoff 修改保留。
