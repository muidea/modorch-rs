# 路由与执行域

适用 0.0.2 当前实现；架构描述不扩大公共保证。

Topic/Destination 选择目标，Lane 只影响局部执行。路由准入与订阅变更线性化；精确 Topic/目标索引和预解析通配模式减少扫描。Dynamic 先预留再锁外构造，安装前复核代际、期限和订阅。源码 crates/modorch-eventhub/src/。

内部 topic.rs 统一 MQTT Topic Name 校验和 TopicFilter 编译/匹配；公开字符串字段保持。索引只将 `+` / 尾部 `#` 放入通配分支，`:id` 是精确字面量。消息入站、订阅和初始 Handler 安装均检查协议合法性及独立本地预算；失败订阅不改索引或 generation。空层、`$` 与长度规则见[消息语义](../message-semantics.md)；同组件多 Filter 仍只入队一次。Topic 对齐不增加网络 MQTT 能力。

设计原因见 [ADR-001](adr/001-documentation-source-of-truth.md)，可观察约束见 [Contracts](../../contracts/README.md)，实际验证见[报告](../validation/2026-11-04/release-0.0.2/README.md)。

未来跨进程 Transport / 持久化 / 插件 ABI 属于 Unsupported，不通过架构图暗示已实现。
