# 实现基线核对与文档刷新

基线 commit：37bdc93，Cargo 0.1.0、Edition 2024、MIT、MSRV 1.85。真实执行记录见 baseline-results.json，源码摘要见 baseline.json；本机 Rust 1.94，Linux x86_64。先完成以下文档刷新，再建设新交付物。

| 旧描述 | 当前实现与修订 |
|---|---|
| v0.3 / MIT OR Apache-2.0 / signals Feature | 改为 0.1.0 / MIT / 宿主信号；manifest 为唯一依赖定义 |
| 六个 crate / demo-app | 八个 crate，六个真实示例；规范中新增目录仍标为交付要求 |
| Module::handlers 返回 Vec | Option<HandlerRegistration>，一个组件一套执行域可含多个订阅 |
| 没有 ready / activate_ingress | 全体 ready 屏障后开放准入并激活入口 |
| source 为必填 ComponentRef、MessageId 等草案别名 | source 为 Option，外部 None，ID 用 OperationId；重复签名改为源码链接 |
| PartialDelivery 错误、路由匹配缓存 | PublishReport.deliveries / is_partial，无匹配缓存 |
| 只有三种执行策略、Lane 未实现 | 四种策略，Partitioned / Dynamic 均已有测试；Concurrent 无顺序承诺 |
| TaskScheduler Trait + TaskSpec、once/every、独立 TaskGroup 管理器 | TaskScheduler 结构体句柄、闭包 / TaskFactory、spawn/delay/interval；TaskGroup 是 TaskScheduler 别名，OperationTracker 统一计数 |
| 自动重启非目标 / 只有新 Application 可推进代际 | Restart 已实现受预算及屏障约束的组件重建；Stopped 不可复活 |
| ResourceUnavailable、StopIncomplete 旧字段 | ServiceUnavailable；StopIncomplete { phase, pending, errors } |
| StartupError 内联主错误 | Box<FrameworkError>，仍可 downcast；cleanup / handle 保留 |
| 94 项为当前测试总数 | 该数字明确标为历史验收计数，最终数量以新日志为准 |
| 性能评估中“当前扫描、逐消息 spawn、无日志预算” | 明确为优化前快照，保留历史测量哈希；当前状态由公共 API / 能力矩阵说明 |

设计中的 AdmissionGate、LifecycleCoordinator 等是职责称谓，不承诺同名公共类型。源码中只有公开导出才是可调用 API。非目标仍包括持久化、跨进程、exactly-once、跨资源事务、安全强杀外部副作用和动态插件 ABI。
