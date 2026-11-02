# 路由与执行域

适用 0.0.1 当前实现；架构描述不扩大公共保证。

Topic/Destination 选择目标，Lane 只影响局部执行。路由准入与订阅变更线性化；精确 Topic/目标索引和预解析通配模式减少扫描。Dynamic 先预留再锁外构造，安装前复核代际、期限和订阅。源码 crates/modorch-eventhub/src/。

设计原因见 [ADR-001](adr/001-documentation-source-of-truth.md)，可观察约束见 [Contracts](../../contracts/README.md)，实际验证见[报告](../validation/2026-11-02/release-0.0.1/README.md)。

未来跨进程 Transport / 持久化 / 插件 ABI 属于 Unsupported，不通过架构图暗示已实现。
