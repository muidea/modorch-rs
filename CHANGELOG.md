# Changelog

## 0.0.1 — 2026-11-02

首个 GitHub 源码版本。框架包与内部依赖统一为 0.0.1；先前 0.1.0 仅为未发布的开发标号。文档与 Skill 保持 Candidate，独立 Agent / 跨宿主验收尚未完成；不声明 crates.io / docs.rs 已发布。

- 按当前 Facade 与行为刷新旧设计文档，校正 Lane、Restart、调度和错误 API。
- 建立指南、Contracts、能力矩阵、文档导航、Agent Skills 与可独立验证的模板。
- 增加文档/契约/Skill 漂移检查、Bundle 完整性/安装验证和基于证据的发布门禁。
- StartupError.source 使用 Box，错误链与清理重试能力保留。完整迁移见 docs/migration/0.1-development.md。

当前为开发期 Candidate；是否 Release Ready 由全部强制检查与实际 Agent 评估决定，未发布到 crates.io / docs.rs。
