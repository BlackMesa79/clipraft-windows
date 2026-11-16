# 品牌与图标定稿记录

2026-11-17：用户确定继续使用 **片筏剪贴板 / ClipRaft**，最终图标采用双层精修 C「柔和青玉」。正式矢量源为 `src/assets/brand.svg`；本目录 `two-layer-refine-c.svg` 保存选定设计原稿，`clipraft-original.svg` 保存此前软件图标。

**快贴 / PasteDash** 名称与图标保留备用，不作为当前产品名称。用户选定的是 B「闪电角标」，保存为 `pastedash-selected.svg`，与 `pastedash-icon-v1-b.svg` 相同，其余候选也一并留档。较早的箭头设计稿保存在相邻的 `../pastedash/`。

运行时图标包括主窗口品牌图、`src-tauri/icons/tray.png`（32×32）与 `src-tauri/icons/icon.ico`（多尺寸）。PNG 由项目已有 Tauri icon 工具从正式 SVG 导出，ICO 从同源 PNG 制作。名称沿用现有 brand.ts、窗口标题和托盘文案，备份格式与数据目录标识继续兼容。
