# 项目开发约定

- 后续开发必须先读取并遵守 `C:/Users/BlackMesa/.claude/code-standards.md`，沿用已有 ESLint、Prettier、TypeScript 和 Rust 配置；不重新配置或全项目自动修复。
- 接续开发先读 `docs/handoff.md` 及对应阶段准备文档。开发、审查和测试由主对话完成，不委派。
- Rust 命令前加载 `. ./scripts/env.ps1`，使用 `--manifest-path src-tauri/Cargo.toml`，测试使用 `-- --test-threads=2`；后台构建并发沿用脚本限制。
- 完成阶段改动后执行 `cargo fmt --check`、全目标 clippy（`-D warnings`）、Rust 测试、`npm run format:check`、`npm run lint -- --max-warnings=0`、`npm run build`，以及 handoff 所列状态与离屏回归。格式化仅涉及本次修改文件。
- 不接管真实键鼠或桌面剪贴板，不修改 `.test-data/manual`、用户当前场景和偏好；真实键鼠控制须先获明确授权。
- 文档及回复使用简体中文。提交须有用户明确授权，不自动提交或推送。

## GitHub Release 发布说明

- 每份发布说明使用 Markdown。首段用一句话注明产品、版本和适用平台，接着按顺序使用下列两个二级标题。标题必须逐字一致，各出现一次，并在标题前后留空行。

  ```markdown
  片筏剪贴板（ClipRaft）X.Y.Z，适用于 Windows x64。

  ## 更新内容

  - 每条列出一项用户能感知的变更。

  ## 下载与运行

  下载附件、运行环境、升级和反馈说明放在本节。
  ```

- 客户端从 `## 更新内容` 之后开始读取，到下一处同级或更高级标题为止。版本变更用 Markdown 无序列表，安装步骤和迁移提示放在「下载与运行」。需要为更新条目分组时使用三级标题。
- 每份 Release 记录其标签版本实际交付的内容。跨版本检查时，客户端负责按正式版本归并和标记各节。
- 发布前确认候选 Release 正文在客户端只显示「更新内容」中的条目，并核对正文与 `CHANGELOG.md` 对应版本一致。
