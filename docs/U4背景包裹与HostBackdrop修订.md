# U4 背景包裹与 Host Backdrop 修订

日期：2026-11-15。用户确认 A 裁切问题消失，但要求背景包裹整个快速面板；同时反馈系统 Acrylic 验证项实际显示为实色，未模糊后方窗口。本轮按该反馈修订，未将上一版原生材质记为视觉通过。

## 此次变化

### A：改为包裹整组卡片的柔边圆角衬底

上一版椭圆渐变虽然避免了裁切，但收掉了面板内部的上下及角落区域。本次保留整块圆角暗底，采用水平/垂直遮罩相交，只在外围 2～18 个逻辑像素范围渐隐。卡片区域内部暗度一致，四边仍在窗口裁切前完全透明。

卡片几何、间距、计数及可见条数不变。实际 PNG 检查覆盖四边中点透明、内部四角与中心暗度相同；在浅色代码窗口后方的实际离屏图已目视核对。预览 `.tools/u4-host-final/previews/background-soft-dark.jpg`。

### 原生验证项：由系统 Acrylic 切换为直接绘制 Host Backdrop

用户截图证明上一版在其非激活透明窗口中未实现所需视觉。此次不再用 DWMSBT_TRANSIENTWINDOW 作为模糊图层：

- 显式设置系统 backdrop 为 NONE，避免上一版 Acrylic 实色继续铺底。
- 启用 DWMWA_USE_HOSTBACKDROPBRUSH，并用 Windows.UI.Composition 创建 DesktopWindowTarget、HostBackdropBrush 及低透明度主题染色层。
- target 使用非 topmost 合成层，置于 WebView 子窗口下，卡片/文字仍由 WebView 清晰绘制；整层按圆角几何裁剪。
- quick 开启 no_redirection_bitmap，与透明 WebView/原生合成背景的窗口路径配合；保留 Layered、NOACTIVATE、鼠标穿透等既有标志。
- 原生合成图在 quick 显示、原生尺寸更新及前端 resize_quick 路径同步 client 大小和 DPI 圆角。
- 隐藏/切回 A/B/错误提示时断开 root、关闭 target/compositor，关闭 host backdrop 标志。DispatcherQueue/WinRT 上下文由本 UI 线程持有，不覆盖已有队列。

复用现有 windows 依赖启用 Composition / Desktop / WinRT 等功能特性，无新增第三方依赖，Cargo.lock 不变。没有截图采样或 CPU 模糊用户桌面，也没有接管键鼠或修改系统偏好。

原生项仍是验证版本：**合成图创建成功与旧 Acrylic 底层关闭不等同于真实模糊视觉通过**。本轮未显示原生产品窗口或截取用户桌面。实际 WebView2 合成、模糊、边缘与非激活效果由用户复测。

## 验证

`.tools/u4-host-final/results.json` 22 阶段全部通过：fmt、全目标 clippy、Rust、显式原生探测、前端格式/lint/类型/Vite 及全部既有离屏回归。

- Rust 常规：812 通过 / 0 失败 / 6 ignored；新增的第6项为已显式执行通过的原生隐藏探测，原有5项性能边界未重跑。
- 原生隐藏窗口探测：1 通过。Host Backdrop 合成图创建成功、旧系统 Acrylic 为 NONE、清理后图层释放，窗口隐藏且非激活/穿透/Layered/NoRedirectionBitmap 标志保持；明确 `visual_verified=false`。
- M1：118；U1/U2：20/154；U3/U4：12/31；隐私/详情与录制状态：13/10/11，全部通过。
- A 的 PNG 透明边界及包裹范围检查通过。浅/深模式实际离屏渲染通过，浅色复杂背景下深色卡片已目视核对。

沿用项目配置，未全项目自动修复；本次生产新增模块不足300行，未发现超过两层嵌套或新增多分支 if-else 链。未改变钩子、ID 选择、粘贴、用户数据、manual 或已有偏好；未提交、推送或打包。

## 用户复测

重启开发进程，先用 A 检查背景是否包裹整组卡片、外围渐隐是否自然。随后选择原来的 **原生背景模糊（验证）** 并保存；名称与保存值不变，后台路径已经更换。重点看后方文字是否真正模糊，卡片/文字是否清晰，浅深主题和屏幕边缘是否正常。

继续确认：Esc、提交、重复呼出、切回 A/B 后无合成背景残留，不抢目标焦点/鼠标；若仍出现实色、黑底或无背景，需要按实际截图继续定位，不能记录为已验收。本轮用户测试期间停止产品代码修改。

## 参考与实现依据

- Microsoft Compositor.CreateHostBackdropBrush 文档。
- Microsoft DWMWA_USE_HOSTBACKDROPBRUSH / DWM_SYSTEMBACKDROP_TYPE 文档。
- Microsoft PowerToys MouseHighlighter 的 DesktopWindowTarget 与非激活/穿透/Layered/NoRedirectionBitmap 透明叠层窗口用法。
- 本项目锁定版本 Windows 0.61.3、Tauri/Wry 源码与生成绑定。

这些依据用于确定调用与分层方式，不替代本项目的原生视觉验收。
