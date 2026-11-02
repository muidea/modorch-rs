# 运行时与关闭事务

适用 0.0.1 当前实现；架构描述不扩大公共保证。

Builder 冻结验证后的描述符和工厂，start 拥有创建与补偿事务。正常关闭与依赖闭包恢复共享事务锁；超时只限制调用者，checkpoint 防止重复 teardown，代际防止旧能力复用。源码 crates/modorch-runtime/src/lib.rs 与 supervision.rs。

设计原因见 [ADR-001](adr/001-documentation-source-of-truth.md)，可观察约束见 [Contracts](../../contracts/README.md)，实际验证见[报告](../validation/2026-11-02/release-0.0.1/README.md)。

未来跨进程 Transport / 持久化 / 插件 ABI 属于 Unsupported，不通过架构图暗示已实现。
