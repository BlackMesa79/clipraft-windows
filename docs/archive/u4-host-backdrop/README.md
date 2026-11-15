# 原生 Host Backdrop 成功实现归档（供未来 Fluent 主题参考）

归档日期：2026-11-15。**用户已通过真实快速面板实测明确确认“原生背景模糊倒是生效了”，并提供截图。** 该结论指 HostBackdropBrush 路径；早期单纯启用 DWMSBT_TRANSIENTWINDOW 的 Acrylic 版本仅显示实色，不能混为成功案例。

本次用户要求移除快速面板两种背景，统一采用卡片阴影。故将已验证代码从产品编译路径移至本目录，以 `.rs.txt` 等只读参考快照保存，不作为当前产品模块。没有存档用户剪贴板内容、桌面截图、数据库或偏好。`manifest.json` 记录来源路径、快照文件名和 SHA-256，归档后逐项核验一致。

## 可复用路径

- `src-tauri__src__quick_material.rs.txt`：系统 Acrylic 关闭、host backdrop 标志、调用错误处理、显示/隐藏和尺寸更新入口。
- `src-tauri__src__quick_material__surface.rs.txt`：WinRT/DispatcherQueue、Compositor、DesktopWindowTarget、HostBackdropBrush、低透明度染色、圆角裁剪、尺寸同步及资源释放。
- `src-tauri__src__quick_material__tests.rs.txt`：隐藏窗口探测，验证创建、清理、样式标志和未成为前台。此探测自身不是视觉证明；视觉成功来自用户另行实测。
- `src-tauri__src__runtime__main_window.rs.txt`：原生显示和错误提示清理的调用时机。
- `src-tauri__src__app_setup.rs.txt`：transparent、no_redirection_bitmap、focusable(false)、鼠标穿透等装配。
- `src-tauri__src__settings.rs.txt`：当时的模式和主题类型，仅作上下文参考，不建议恢复整个旧设置文件。
- `src-tauri__Cargo.toml.txt`：Windows Composition/WinRT 等所需功能特性；当时没有新增第三方依赖。

重要分层：DesktopWindowTarget 的 isTopmost=false，背景层位于 WebView 子窗口下，避免模糊卡片文字；先把 DWMWA_SYSTEMBACKDROP_TYPE 设置为 NONE，再启用 DWMWA_USE_HOSTBACKDROPBRUSH 并实际绘制画笔。合成图使用 UI 线程本地存储，不跨线程共享 COM 对象。

清理必须覆盖隐藏、Esc/提交、主窗口切换、切换背景、错误胶囊和异常路径；resize 后更新 client 尺寸与 DPI 圆角。原 quick 中 NOACTIVATE/Layered/Transparent/NoRedirectionBitmap 的组合经过隐藏探测及用户真实窗口试用，但未来换主窗口/Fluent 容器后仍须重新验证。

## 中断前状态与证据

中断前移除 A 的代码已完成，`.tools/u4-remove-soft-final/results.json` 共22阶段全部通过。缺少的是最后文档记录，本轮已补齐。此目录保留该结果和 `native-probe-summary.json`（从被忽略的原日志提取的关键结论），并记录用户原生模糊成功反馈。

原始背景探索与失败/修订过程见 `../../U4背景包裹与HostBackdrop修订.md`、`../../U4原生模糊验证记录.md`。用户没有逐项确认所有 DPI、性能和系统版本组合，不能将本次成功泛化为全部原生场景均已验收。

## 未来 Fluent 接续

复用材质模块思路和生命周期管理，根据新的宿主窗口调整调用点；不要直接覆盖当前 settings/runtime 等整文件。重新验证系统兼容、真实透明/模糊、焦点/穿透、混合 DPI、尺寸变化、性能及资源清理。原生材质功能与当前卡片阴影方案独立，本归档不自动启用任何功能。
