# 扩展应用能力

先选[组件角色](concepts/component-model.md)，再定义业务 Topic/DTO owner 与版本。跨模块共享资源使用窄服务与租约；私有业务状态通过消息处理。应用只依赖 Facade，不引用内部实现 crate。

实现 Factory、Lifecycle 与可选 Handler，声明依赖、顺序策略、队列、Owner 任务和失败预算。在 setup 创建资源；ready 后激活入口；teardown 可重试且不得生成新业务工作。CPU / 阻塞 I/O 单独设资源预算。

[模块指南](guides/module-development.md)和[模块 Skill](../.agents/skills/modorch-module-development/SKILL.md)提供任务步骤；[modular-service 模板](../templates/modular-service)组合服务、请求、周期任务和 TOML 装配。需要修改 Runtime 行为时先走维护型 Skill 与 Contract 变更流程，不能在业务模块旁另造准入/关闭协议。
