# 实现总览

适用 0.0.3 当前实现；架构描述不扩大公共保证。

八个单向依赖 crate 通过 Facade 组合。应用持有 main/Runtime，实例内创建 Registry、Hub、Scheduler、Tracker 和监督状态；没有隐式全局单例。完整图见 ../architecture.md。

设计原因见 [ADR-001](adr/001-documentation-source-of-truth.md)，可观察约束见 [Contracts](../../contracts/README.md)，实际验证见[报告](../validation/2026-11-06/release-0.0.3/README.md)。

未来跨进程 Transport / 持久化 / 插件 ABI 属于 Unsupported，不通过架构图暗示已实现。
