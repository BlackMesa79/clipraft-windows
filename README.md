<div align="center">
  <img src="src/assets/brand.svg" width="88" height="88" alt="片筏剪贴板图标">
  <h1>片筏剪贴板</h1>
  <p><strong>ClipRaft · 你的内容，随时可用。</strong></p>
  <p>Windows 剪贴板管理器 · 快速粘贴 · 多格式历史 · 本地加密</p>
  <p>
    <a href="https://github.com/BlackMesa79/clipraft-windows/releases/download/v0.7.0/ClipRaft-0.7.0-windows-x64.zip"><strong>下载 0.7.0 · Windows x64</strong></a> ·
    <a href="https://github.com/BlackMesa79/clipraft-windows/releases/tag/v0.7.0">发布说明</a> ·
    <a href="https://github.com/BlackMesa79/clipraft-windows/issues">问题反馈</a>
  </p>
  <p>
    <a href="#界面预览">界面预览</a> ·
    <a href="#主题风格">主题风格</a> ·
    <a href="#核心功能">核心功能</a> ·
    <a href="#快速上手">快速上手</a> ·
    <a href="#常用快捷键">常用快捷键</a>
  </p>
</div>

连续复制几段文字、一张截图、几个文件，之前的内容也能随时找回来。片筏剪贴板（ClipRaft）将它们保存在本机，需要时**按住快捷键、选择记录、松手粘贴**，也可以打开主窗口搜索、预览和整理历史。

## 界面预览

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/images/main-dark.jpg">
  <img src="docs/images/main-light.jpg" alt="片筏剪贴板主面板：左侧分组、中间历史列表、右侧内容预览与粘贴操作" width="960">
</picture>

<p align="center"><sub>主面板 · 分组、历史记录与内容预览</sub></p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/compact-dark.jpg">
    <img src="docs/images/compact-light.jpg" alt="紧凑模式：在较小窗口里搜索、筛选和选择剪贴板历史" width="400">
  </picture>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/quick-dark.jpg">
    <img src="docs/images/quick-light.jpg" alt="3D滚轮快捷面板：中心卡片高亮，上下卡片呈现透视层次，松开修饰键即可粘贴" width="400">
  </picture>
</p>

<p align="center"><sub>紧凑模式 · 小窗口浏览历史　｜　3D 滚轮快捷面板 · 按住选择，松手粘贴</sub></p>

以上预览均使用示例数据，随页面主题展示浅色或深色外观。

## 主题风格

片筏剪贴板提供不同的界面主题。主题展示沿用相同的历史内容与操作结构，后续可在此补充更多风格。

### Fluent

Fluent 主题采用 Windows 11 风格的表面、控件和转场效果，支持浅色、深色、常规与紧凑布局，并可调整背景透出强度。

<p align="center">
  <img src="docs/images/fluent-main.jpg" alt="Fluent 主题主面板，展示分组、历史记录和内容预览" width="960">
</p>
<p align="center"><sub>主面板</sub></p>

<p align="center">
  <img src="docs/images/fluent-compact.jpg" alt="Fluent 主题紧凑布局，展示搜索、分组和历史记录" width="400">
  <img src="docs/images/fluent-settings.jpg" alt="Fluent 主题设置页，展示分类导航与主题设置控件" width="500">
</p>
<p align="center"><sub>紧凑布局　｜　设置</sub></p>

当前默认主题名为「素笺」，Fluent 可在「设置 → 外观 → 皮肤」中选择。

## 核心功能

### ⚡ 按住选择，松手粘贴

在正在编辑的窗口里，按住 **Ctrl + Shift**，点按 **V** 呼出快速面板；用 **↑ / ↓** 或再次点按 **V** 选择历史，松开修饰键即可粘贴。快速面板不获取输入焦点，选择内容时保留原窗口的光标和选区。

有多条历史时，面板默认选中上一条，方便找回刚被新复制内容覆盖的记录。按 **空格**、**鼠标右键** 或 **Esc** 可取消本次操作并关闭面板。

### 📋 文字、图片和文件，都能留在历史里

| 内容类型 | 可以怎样使用 |
| --- | --- |
| **文本与富文本** | 保存纯文本、HTML 和 RTF，在支持的应用里保留排版；也可以只粘贴纯文本。 |
| **图片** | 在列表中查看缩略图，打开大图预览，再将图片粘贴到目标应用。 |
| **文件** | 保存单个或多个文件的路径记录，可粘贴成文件、用默认应用打开，或定位到原位置。 |
| **链接** | 从历史中找回复制的网址，直接用默认浏览器打开。 |

文件记录依赖原文件仍在对应路径；富文本、图片和文件的粘贴效果取决于目标应用支持的格式。

### 🔎 找得快，也整理得好

- **关键词与拼音搜索**：用文本、文件名、全拼或拼音首字母查找历史。
- **类型与分组筛选**：按文本、图片、文件、链接缩小范围，也可以结合分组查找。
- **置顶、收藏与分组**：让常用回复、工作资料和临时片段各有位置。
- **自动清理**：按条数、空间和保留天数管理历史，减少手动整理。

### 🎨 按自己的习惯查看内容

提供**列表、双列和紧凑模式**，支持浅色、深色外观。浏览历史时在预览区确认内容；紧凑模式可配合独立浮动预览，在较小窗口里查看记录。

主窗口和快速面板的快捷键均可自定义，遇到组合冲突时可在设置中检测并更换。

### 🔒 历史留在本机，隐私由你掌握

- **加密保存**：历史数据库使用 SQLCipher 加密，本机密钥由 Windows DPAPI 保护。
- **忽略指定程序**：为不希望记录的应用设置排除规则。
- **敏感内容保护**：对命中规则的内容打码预览，并按设置到期清理。
- **随时暂停**：通过系统托盘暂停或恢复剪贴板监听。

本地加密保护落盘数据；敏感识别按规则工作，不能保证识别所有私密内容。

### 📦 备份、恢复与更换存储位置

在设置中用密码导出加密备份，需要时恢复历史；也可以将数据迁移到自选目录。更换电脑或 Windows 账户时，请使用密码导出包迁移数据。

## 快速上手

> **0.7.0** 已开放下载：[Windows x64 ZIP](https://github.com/BlackMesa79/clipraft-windows/releases/download/v0.7.0/ClipRaft-0.7.0-windows-x64.zip) · [发布说明](https://github.com/BlackMesa79/clipraft-windows/releases/tag/v0.7.0)。

**运行环境：** Windows x64，需要 [Microsoft Edge WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)。当前以 Windows 11 为主要验证环境，Windows 10 兼容性仍待进一步验证。

下载 ZIP 包后：

1. **解压并启动**：将压缩包解压到独立文件夹，双击 `ClipRaft.exe`，程序会驻留系统托盘。
2. **试一次快速粘贴**：依次复制两段不同的文字，在输入框按住 **Ctrl + Shift**、点按 **V**，再松开修饰键，即可粘贴第一段文字。
3. **查看完整历史**：按 **Alt + V** 或点击托盘图标打开主窗口，搜索、预览并整理记录。

关闭主窗口后程序继续在托盘运行；完整退出请使用托盘菜单中的“退出”。

<details>
<summary>使用前了解</summary>

- **ZIP 解压运行不等于数据便携**：历史与设置默认保存在用户目录，不会自动跟随程序文件夹移动；更换位置请使用应用内的数据迁移功能。
- **向管理员窗口粘贴受 Windows 权限限制**：普通权限运行的片筏剪贴板无法向管理员权限的目标应用自动粘贴。
- **日常数据密钥绑定 Windows 账户**：直接复制数据目录不能替代跨账户备份，请通过密码导出包迁移。

</details>

## 常用快捷键

以下为默认设置。快速面板与主窗口的呼出快捷键可在设置中修改。

| 场景 | 操作 | 效果 |
| --- | --- | --- |
| 任意窗口 | 按住 **Ctrl + Shift**，点按 **V** | 呼出快速面板，默认选择上一条记录 |
| 快速面板 | **↑ / ↓**，或再次点按 **V** | 切换记录；松开修饰键后粘贴 |
| 快速面板 | **空格**、**鼠标右键** 或 **Esc** | 取消本次粘贴并关闭面板 |
| 任意窗口 | **Alt + V** | 呼出或隐藏主窗口 |
| 主窗口 | **Ctrl + F** | 返回搜索框 |
| 主窗口 | **↑ / ↓** · **← / →** · **Ctrl + ↑ / ↓** | 选择记录 · 切换类型 · 切换分组 |
| 主窗口 | **Enter** / **Shift + Enter** | 粘贴所选记录 / 仅粘贴其中的纯文本 |

搜索框有文字时，左右键用于移动光标；按上下键进入列表后可切换类型。纯文本粘贴要求记录包含纯文本格式。

## 后续计划

以下功能尚未实现，具体方案和排期可能调整：

- **主题与背景扩展**：自定义背景图及更多界面主题。
- **加密云备份**：在本机加密后上传历史快照，支持按备份版本恢复。
- **多设备同步**：通过端到端加密同步记录，让已授权设备接续使用。

## 开发与反馈

片筏剪贴板基于 **Tauri 2 + Vue 3 + TypeScript + Rust** 开发，通过 Win32 API 处理剪贴板、全局快捷键和粘贴。

构建、测试及实现说明见 [开发与验收记录](docs/开发与验收记录.md)，接续开发请先阅读 [项目交接](docs/handoff.md)。当前构建脚本包含本机工具链路径，从源码运行前需要按本地环境调整。

欢迎反馈使用体验、问题和功能建议。描述问题时，请附上系统与应用版本、复现步骤、预期和实际结果；截图请使用不含隐私的示例内容。

## 开源协议

本项目采用 [MIT 许可证](LICENSE)，允许商用、修改和再分发，使用时需保留版权与许可文本。第三方组件遵循各自的许可证，详见 [第三方组件说明](THIRD-PARTY-NOTICES.md)。
