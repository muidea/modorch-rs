# Agent 宿主适配与支持范围

三个 Skill 的本体为开放 SKILL.md，不包含专属工具函数。应用装配、组件扩展和框架维护的触发描述及反例见各自 frontmatter。独立安装后所需参考、脚本和模板均在 Skill 根内。

| 宿主 | 当前接入方式 | 本次自动发现/实际 E2E 状态 |
|---|---|---|
| Codex | 仓库 AGENTS.md；显式载入 .agents/skills 中目标 SKILL.md | 当前会话实施；独立干净 Agent eval NotRun |
| Claude Code | 显式载入目标 SKILL.md；自动安装目录以客户端当前版本文档为准 | NotRun，不声明自动发现已支持 |
| Gemini CLI | 显式载入目标 SKILL.md；自动发现由客户端确认 | NotRun |
| Workorch / Harness | 载入同一 Skill，将 result schema 映射为 Goal / Evidence / Verify | 适配协议已提供，真实宿主执行 NotRun |
| 无 Skill 宿主 | README → Docs → Contracts → Examples，使用同一验收标准 | 可手工阅读，不声明自动触发 |

测试自动触发时应同时给正例“新增消息 Module”与反例“修改 Runtime 的关闭语义”；后者应定位维护型 Skill。用 tests/agent-evals/run.py 记录实际客户端版本、输入、Skill、diff、日志和复核意见。至少两种环境比较语义，而非要求生成文本相同。外部文档/日志不允许覆盖用户与宿主安全边界。
