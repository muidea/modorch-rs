# 版本与兼容性

所有框架 crate 使用同一 `0.1.0` 版本系列，Rust Edition 2024、Cargo resolver 3、MIT 许可证。0.x 期间公共 API 仍可按小版本演进，尚无正式 Registry 发布或包名占用声明。稳定 API 仅通过 Facade 引入，业务 DTO 版本由能力 Owner 管理。

Feature 表：

| Feature | 内容 |
|---|---|
| 默认 | runtime、eventhub、scheduler |
| 无默认 Feature | core、依赖/服务注册表、async_trait 宏 |
| runtime | 应用生命周期，同时包含 eventhub / scheduler |
| eventhub | 消息、订阅、执行策略 |
| scheduler | Owner Task 和 Timer |
| config-toml | TOML schema 与 ComponentSelection 转换 |
| observability | 轻量在途快照适配；与 runtime 同时启用时提供完整应用指标采集 |

Cargo Feature 是加法组合，不能隔离同一依赖图中不同消费者的 Feature。组件运行时启用仍必须使用 ComponentSelection。没有库级 signals Feature；信号由宿主自身的 Tokio Feature 提供。

最低 Rust 版本为 1.85.0，已实际通过全工作区、全目标、全 Feature 编译。验证使用锁定依赖；CI 单独运行 1.85.0 编译检查，并在 stable 上运行完整检查、测试、示例和 Feature 矩阵。本次实际执行的工具链与命令结果记录于 [实现验收表](implementation.md)。

当前实现包含三种 FailurePolicy、静态和动态 Lane，以及显式有时限的 Wait 入队 API。进程间 Transport、消息持久化和自动重试有副作用请求属于设计非目标，不随本轮扩展加入。资源强制取消、Exactly-once、跨资源事务和第三方插件沙箱不作承诺。

本次开发收口扩展了 `ExecutionFailed` 的可选操作 ID、`Phase::Handle/Task`、`ComponentState::PartiallyInitialized` 和部分诊断快照字段。Handler/Task 的返回错误现在带执行边界和原始 `source`，调用方识别业务根因应使用 `root_cause()`。这些是尚未发布的 0.1 开发期 API 调整；新增字段的结构体字面量和错误穷举匹配需要同步适配，不声称对旧草图二进制兼容。

## 2026-10-31 性能与内存收口

本轮增加 HubLimits / ExecutionResources / BlockingExecutor / BlockingTaskHandle，以及 Builder 的对应配置和 journal_capacity。MessageOptions 新增 payload_bytes，使用结构体字面量时请补充该字段或 ..Default::default()；未配置字节预算时保持可选。HubMetrics、TaskMetrics、RuntimeMetrics 增加资源诊断字段，FrameworkError 增加可重试的 ResourceLimit 变体，穷举匹配需同步更新。

新增默认边界：等待提交全局 1024／每目标 256，Topic 和 LaneKey 1024 字节，Header 32 项／8192 字节，订阅 65536，每 Owner Task 256；均可显式配置。Concurrent 最大额度改为 4096，并预建该数量的 Tokio Worker 及一个 Dispatcher。生命周期日志改为默认只保留最近 1024 项；需要更长诊断窗口可配置 journal_capacity，淘汰数量通过 RuntimeMetrics 查询。状态观察者不再保留回复，RequestHandle::wait 仍返回 Arc<R>，消息顺序、已接受工作排空和代际关闭合同保持。

执行资源补充 spawn_cpu_wait / spawn_blocking_io_wait、TaskHandle / BlockingTaskHandle 的 wait_finished，以及按类别和 Owner 的资源指标。ExecutionResources 增加 max_waiters（1024）和 max_waiters_per_owner（128），结构体字面量需补字段或使用 ..Default::default()；TaskMetrics 增加容量、类别及 owner_resources。既有立即式提交与 wait 的结果可读边界保持；新增 wait_finished 用于确认任务自身资源归还。动态 Lane 工厂移至路由锁外，max_lanes 同时限制构造中与活跃的 Lane，MailboxMetrics 增加 preparing_lanes；等待式消息准入现在严格复核期限，零等待窗口返回 AdmissionTimeout。
