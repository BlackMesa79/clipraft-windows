# U7 Fluent 主题设计定稿

2026-11-18。用户已确认第二轮设计，作为 U7 实装基线。用浏览器打开 [定稿交互原型](u7-fluent.html)，切换主面板、设置、3D 滚轮、只读浮窗。主面板可切单列、双列与紧凑，四个视图共用浅色、深色和高对比示意。在「设置」展开「明暗」或「常规布局」，可体验下拉浮层和键盘选择。顶部「动效」可切换跟随系统和减少动效。原型使用合成数据和本地文件，粘贴、打开文件及删除只显示演示反馈。

## 视觉方向

用中性的窗口基底和连续内容面组织主窗口。标题栏与分组栏在基底上，搜索、历史列表和详情预览在内容面上。青绿色集中用于当前选择、键盘焦点和「粘贴此条记录」。工作与素材分组各保留独立身份色，选中时同时提供短竖条和底色。此稿的青绿是片筏品牌取色，主窗分层参考微软的连续内容面模式。 [Windows 设计原则](https://learn.microsoft.com/en-us/windows/apps/design/design-principles) · [Mica 分层](https://learn.microsoft.com/en-us/windows/apps/design/style/mica)

| 视觉角色 | 浅色稿 | 深色稿 |
| --- | --- | --- |
| 窗口基底 / 内容面 | `#F3F5F4` / `#FBFCFB` | `#202927` / `#252F2D` |
| 卡片与弹出层实色 | `#FFFFFF` | `#2C3735` |
| 主文字 / 次要文字 | `#202927` / `#52635E` | `#EFF5F3` / `#B5C5BF` |
| 主动作背景 / 文字 | `#176B64` / `#FFFFFF` | `#83D5C3` / `#122A24` |
| 选中填充 / 键盘焦点 | `#E3F0EC` / `#176B64` | `#304843` / `#8BDCCB` |

字体优先 Segoe UI Variable Text，中文使用 Microsoft YaHei UI 等本机字体回退。正文 14 像素，辅助文字 12 像素，标题和主要操作用 600 字重。控件圆角 4 像素，内容面与弹出层 8 像素，主窗外壳 12 像素。开关和短提示用胶囊形。具体数值为这版原型选择，实施时由角色令牌归并。 [Windows 排版指南](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/typography) · [Fluent 形状](https://fluent2.microsoft.design/shapes/) · [Fluent 设计令牌](https://fluent2.microsoft.design/design-tokens/)

## 窗口与状态

| 场景 | 评审内容 |
| --- | --- |
| 主面板常规 | 单列和双列共用导航、搜索栏和详情区。搜索底线在获得焦点时加粗，选中记录有左侧短条，焦点环独立于选中状态。文件或链接的打开入口显示为次要按钮。 |
| 紧凑布局 | 维持约 390 像素宽和 54 像素行高，分组下拉、类型筛选和单列记录保持顺序，内嵌详情收起。设计稿的三布局切换置于评审画板。 |
| 设置 | 以左导航和右侧设置卡片组织。「外观」分主题与布局，卡片使用左侧线图标、中部标题与说明、右侧控件。卡片采用低对比边线、4 像素圆角和 66 像素最小高度。固定保存区展示未修改、未保存、取消和保存。正式接入后皮肤下拉同时提供片筏默认与 Fluent。 |
| 菜单与确认 | 用独立表面、边线和阴影区分浮层。删除在菜单中以文字和位置区分，确认稿首先聚焦「取消」。居中确认用于评审表面、焦点和按键样式，实际确认入口沿用各功能现有位置。 |
| 3D 滚轮 | 维持 420 像素宽、64 像素卡高、76 像素步距及五条示例。左上显示「空格 / 右键取消」，右上显示计数。背景延续透明，卡片实色，各自投影，中心项用选中面和描边标识。 |
| 只读浮窗 | 窄边线、箭头与独立标题区表现锚定关系，内容可滚动。文本、图片、文件失效和敏感遮罩均可试看，浮窗内只有阅读信息。 |

### 下拉框

收起时显示当前值、细边线和右侧箭头。展开时是独立的浅深色浮层，具有圆角和阴影。当前值用左侧青绿短条标识，悬停和键盘焦点各有反馈。长列表在浮层内滚动。方向键浏览，Enter 确认，Esc 撤销并回到下拉框。原型沿用 HTML select 的选择语义，由支持可定制 select 的 Edge 渲染浮层样式。后续实现需核对目标 WebView2 的呈现和兼容路径。 [Windows ComboBox](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/combo-box) · [Windows 设置页和 SettingsCard](https://learn.microsoft.com/en-us/windows/apps/design/app-settings/guidelines-for-app-settings)

### 动效

现有产品已实现 3D 滚轮卡片的 140 毫秒位移过渡。主面板与设置的页面切换、菜单和确认层尚无应用层过渡。第二稿用约 100 毫秒的控件状态和开关反馈、130 毫秒的页面内容淡入、160 毫秒的浮层淡入及短位移演示新增效果。采用短促缓出。键盘操作和应用响应直接生效。减少动效时立即切换。键盘焦点采用 2 像素轮廓，高对比度下保留选中标记与焦点。 [Windows 动效](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/motion) · [Fluent 动效](https://fluent2.microsoft.design/motion/) · [对比度主题](https://learn.microsoft.com/en-us/windows/apps/design/accessibility/high-contrast-themes)

## 材质示意

| 位置 | 第一阶段 | 原生专项候选 |
| --- | --- | --- |
| 主窗标题栏与基底 | 实色基底及实色内容面 | Mica 作用于基底，可从标题栏和导航区看见，阅读内容面继续提供稳定对比。 |
| 记录菜单与短暂只读浮窗 | 实色加阴影 | 评估单层 Acrylic。打开「材质示意」可比较层级与边界。 |
| 快速面板 | 透明窗口与实色卡片 | 沿用独立阴影。本稿的材质切换不改变滚轮舞台。 |

原型用固定渐变模拟 Mica 的柔和色差，用半透明底和 CSS 模糊示意短暂浮层的 Acrylic。微软将 Mica 定义为融合主题和桌面壁纸的不透明基底，并建议把 Acrylic 用于菜单等短暂表面。实现后需在高对比度、系统透明关闭、非活动及设备不支持等场景逐项确认实色回退和可读性。 [Microsoft Mica](https://learn.microsoft.com/en-us/windows/apps/design/style/mica) · [Microsoft Acrylic](https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic)

按 [U7 开发准备](<../U7 Fluent皮肤开发准备.md>) 实现实色皮肤及三窗口集成。原生材质和系统环境下的兼容与回退需在实装后单独验收。

独立 Edge 离屏验证 11 类场景，包括九种明暗与布局组合、真实打开下拉框、键盘选择、长列表限高、模拟强制色及减少动效。抽查的六组实色文字配对最低为 5.8:1。结果和压缩预览位于 `.tools/u7-design-review-v2/`。
