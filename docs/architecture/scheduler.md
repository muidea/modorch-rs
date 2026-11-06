# 执行资源与真实退出

适用 0.0.3 当前实现；架构描述不扩大公共保证。

异步工作复用宿主 Tokio，CPU / 同步阻塞 I/O 使用各自惰性有界池。全局与 Owner 限额跨类别共享；定时器独立计数。结果可读与捕获/额度释放是两个边界，wait_finished 提供后者确认。源码 crates/modorch-scheduler/src/。

设计原因见 [ADR-001](adr/001-documentation-source-of-truth.md)，可观察约束见 [Contracts](../../contracts/README.md)，实际验证见[报告](../validation/2026-11-06/release-0.0.3/README.md)。

未来跨进程 Transport / 持久化 / 插件 ABI 属于 Unsupported，不通过架构图暗示已实现。
