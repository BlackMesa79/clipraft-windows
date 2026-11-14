# 项目开发约定

- 后续开发必须先读取并遵守 `C:/Users/BlackMesa/.claude/code-standards.md`，沿用已有 ESLint、Prettier、TypeScript 和 Rust 配置；不重新配置或全项目自动修复。
- 接续开发先读 `docs/handoff.md` 及对应阶段准备文档。开发、审查和测试由主对话完成，不委派。
- Rust 命令前加载 `. ./scripts/env.ps1`，使用 `--manifest-path src-tauri/Cargo.toml`，测试使用 `-- --test-threads=2`；后台构建并发沿用脚本限制。
- 完成阶段改动后执行 `cargo fmt --check`、全目标 clippy（`-D warnings`）、Rust 测试、`npm run format:check`、`npm run lint -- --max-warnings=0`、`npm run build`，以及 handoff 所列状态与离屏回归。格式化仅涉及本次修改文件。
- 不接管真实键鼠或桌面剪贴板，不修改 `.test-data/manual`、用户当前场景和偏好；真实键鼠控制须先获明确授权。
- 文档及回复使用简体中文。提交须有用户明确授权，不自动提交或推送。
