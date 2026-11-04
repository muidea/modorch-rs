# 文档索引

当前 0.0.2 开发树；[能力矩阵](reference/capability-matrix.md)限定验证范围，[验收追踪](validation/2026-11-04/release-0.0.2/README.md)记录交付与未覆盖项。

| 读者 | 阅读路径 |
|---|---|
| 新用户 | [入门](getting-started.md) → [应用](guides/application.md) → [组件模型](concepts/component-model.md) |
| 组件开发者 | [模块](guides/module-development.md)、[服务](guides/service-registry.md)、[消息](guides/messaging.md)、[调度](guides/scheduler.md)、[错误](guides/error-handling.md) |
| 理解运行边界 | [依赖](concepts/dependency-management.md)、[生命周期](concepts/lifecycle.md)、[限制](reference/limitations.md)、[兼容](reference/compatibility.md)、[排错](reference/troubleshooting.md) |
| 维护者 | [架构](architecture/overview.md)、[运行时](architecture/runtime.md)、[消息](architecture/eventhub.md)、[调度](architecture/scheduler.md)、[ADR](architecture/adr/001-documentation-source-of-truth.md)、[测试](testing-guide.md)、[扩展](extension-guide.md)、[迁移](migration/0.1-development.md) |
| Agent | [仓库规则](../AGENTS.md)、[应用 Skill](../.agents/skills/modorch-app-development/SKILL.md)、[模块 Skill](../.agents/skills/modorch-module-development/SKILL.md)、[维护 Skill](../.agents/skills/modorch-framework-maintenance/SKILL.md)、[Bundle 安装](../tools/skill-packaging/README.md) |

[Contracts](../contracts/README.md)是规范性语义入口；[公共 API 导航](public-api.md)链接源码，签名以生成的 target/doc/modorch/index.html 为准。当前未声称 docs.rs 已发布。

[设计说明](Modorch-RS-Framework-Design.md)、[实现映射](implementation.md)、[消息细节](message-semantics.md)、[资源屏障](lifecycle.md)、[独立接入](integration-guide.md)保留工程背景。[历史性能](performance-review.md)与[负载报告](validation.md)不替代本次验证。

[文档/Skill 规范](modorch-rs-documentation-agent-skills-spec.md)按实现刷新，所有文件目标的完成状态由验收追踪表给出。
