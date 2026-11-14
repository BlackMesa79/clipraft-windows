# Prettier 接入记录

日期：2026-11-14。已验收的 lint 与结构优化先提交为 `5afd5cf`，本轮在该基线上独立接入前端与脚本格式化。

- 固定 Prettier 3.10.1 和 eslint-config-prettier 10.1.8；新增 format / format:check，具体白名单见 package.json。没有格式化 Rust、PowerShell、文档、生成目录或用户数据。
- 单引号、100 列目标宽度、LF，Vue 模板使用 htmlWhitespaceSensitivity=strict；Vue 内嵌 script/style 正常格式化。ESLint 仍独立检查质量，不把 Prettier 作为 ESLint 插件执行。
- 首次默认模板策略导致菜单文本两端多空格，T5 失败；严格模板策略修复此处后，T5 发现分组名称和数量间的空格在 Vue 换行折叠时消失。使用显式空格插值保持原语义，原测试断言未放宽。首次与中间失败日志保留在 .tools/prettier-check-m1-t5-ui.log 与 .tools/prettier-final-check-m1-t5-ui.log。
- 格式化展开代码后，两个状态工厂及主窗口初始化入口超过 ESLint 50 行限制。仅在三个入口添加说明与 max-lines-per-function 单行例外，保留实例状态闭包及初始化顺序；内部函数继续检查，未调整全局阈值。原控制字符正则豁免随正则移动到正确行，表达式不变。
- 对比 HEAD 经当前 Prettier 两次格式化的输出，34 个发生变化的前端/脚本文件除上述三处行数说明、原正则注释位置和显式空格外均一致；差异审查证据 .tools/prettier-exceptions-review.log。不宣称格式化差异小，也不将离屏测试等同于原生桌面验收。
验证结果：format:check、ESLint --max-warnings=0、vue-tsc/Vite build、git diff --check 均通过。两组状态检查 9/11 项，T1～T8 离屏 8/18/13/18/14/8/20/14 项（共113），U1/U2 20/104 项均通过。日志 .tools/prettier-{check-final,eslint-final,build,diff}.log 及 .tools/prettier-verified-check-*.log。最终版本检查通过后未再修改产品代码。Rust 无差异，本轮未重复 Rust 测试；前一提交常规全量为702通过/3ignored。原生键鼠/焦点/DPI/外部粘贴和大图性能未重跑，B-01/B-02状态不变。未接管键鼠或修改用户偏好；用户随后完成打包测试并反馈无问题，已授权提交；本记录随 Prettier 改动提交。下一阶段进入 U3，本轮未实现 U3。