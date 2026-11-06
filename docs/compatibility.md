# 版本与兼容性

所有框架 crate 使用同一 `0.0.3` 版本系列，Rust Edition 2024、Cargo resolver 3、MIT 许可证。0.x 期间公共 API 仍可按小版本演进，尚无正式 Registry 发布或包名占用声明。稳定 API 仅通过 Facade 引入，业务 DTO 版本由能力 Owner 管理。

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

为兼容新版 Clippy 对异步函数错误大小的检查，StartupError.source 改为 Box<FrameworkError>；Application::start 的 Result<ApplicationHandle, StartupError> 签名、cleanup 和 handle 字段类型保持。直接构造 StartupError 时需包装 Box::new(source)，按值匹配 source 时需先解引用；to_string、root_cause 等方法调用及标准 Error::source() 的 FrameworkError downcast 保持可用。

当前实现包含三种 FailurePolicy、静态和动态 Lane，以及显式有时限的 Wait 入队 API。进程间 Transport、消息持久化和自动重试有副作用请求属于设计非目标，不随本轮扩展加入。资源强制取消、Exactly-once、跨资源事务和第三方插件沙箱不作承诺。

本次开发收口扩展了 `ExecutionFailed` 的可选操作 ID、`Phase::Handle/Task`、`ComponentState::PartiallyInitialized` 和部分诊断快照字段。Handler/Task 的返回错误现在带执行边界和原始 `source`，调用方识别业务根因应使用 `root_cause()`。这些是首发版本归一化前的 0.1 开发期 API 调整；新增字段的结构体字面量和错误穷举匹配需要同步适配，不声称对旧草图二进制兼容。

## 2026-10-31 性能与内存收口

本轮增加 HubLimits / ExecutionResources / BlockingExecutor / BlockingTaskHandle，以及 Builder 的对应配置和 journal_capacity。MessageOptions 新增 payload_bytes，使用结构体字面量时请补充该字段或 ..Default::default()；未配置字节预算时保持可选。HubMetrics、TaskMetrics、RuntimeMetrics 增加资源诊断字段，FrameworkError 增加可重试的 ResourceLimit 变体，穷举匹配需同步更新。

新增默认边界：等待提交全局 1024／每目标 256，Topic 和 LaneKey 1024 字节，Header 32 项／8192 字节，订阅 65536，每 Owner Task 256；均可显式配置。Concurrent 最大额度改为 4096，并预建该数量的 Tokio Worker 及一个 Dispatcher。生命周期日志改为默认只保留最近 1024 项；需要更长诊断窗口可配置 journal_capacity，淘汰数量通过 RuntimeMetrics 查询。状态观察者不再保留回复，RequestHandle::wait 仍返回 Arc<R>，消息顺序、已接受工作排空和代际关闭合同保持。

执行资源补充 spawn_cpu_wait / spawn_blocking_io_wait、TaskHandle / BlockingTaskHandle 的 wait_finished，以及按类别和 Owner 的资源指标。ExecutionResources 增加 max_waiters（1024）和 max_waiters_per_owner（128），结构体字面量需补字段或使用 ..Default::default()；TaskMetrics 增加容量、类别及 owner_resources。既有立即式提交与 wait 的结果可读边界保持；新增 wait_finished 用于确认任务自身资源归还。动态 Lane 工厂移至路由锁外，max_lanes 同时限制构造中与活跃的 Lane，MailboxMetrics 增加 preparing_lanes；等待式消息准入现在严格复核期限，零等待窗口返回 AdmissionTimeout。

## 2026-11-04 MQTT Topic 对齐（破坏性行为变更）

本轮发布 0.0.2 Candidate；已发布 v0.0.1 的源码/Bundle 不被改写。新开发树 Contract revision 1.1.0 / Skill revision 1.0.5，须按新框架指纹选包。

`:id` 从单层通配改为字面量，迁移 `orders/:id/changed` 为 `orders/+/changed`。`+` 现在包含空层，`orders/#` 包含 `orders`；根 `+/#` 排除 `$` 名称，系统树用 `$SYS/#` 显式订阅。中间/嵌入通配非法，所有消息入口拒绝空 Name、NUL、wildcard 和超 65535 UTF-8 字节。标准合法但超本地 Topic 预算（默认 1024）由 InvalidConfig 改为 ResourceLimit；Named destination 等其他元数据校验保持原约束。没有 Legacy matcher。迁移须审计 Handler 对空层的假设和 ACL 的扩大匹配范围。API 字符串字段、Destination、Lane、Accepted 和请求超时含义保持；详见[迁移步骤](migration/0.1-development.md)。

## 2026-11-06 Module 入口增强（0.0.2 开发树）

新增默认无操作的 `Module::activate_ingress`，覆盖时在 impl 添加 `#[async_trait]`，不覆盖的既有 Module 无需修改。同名固有方法在调用时可能遮蔽 trait 方法，可用 `<T as Module>::activate_ingress` 限定。只通过 Facade 引入 API，不放开 Module 的服务发布权限。

将 start/ready 中的监听迁入 activate_ingress；setup 若预绑定，改用未 listen 的 TcpSocket，避免 TcpListener::bind 提前接受 TCP 握手。长期任务使用 ctx.tasks，入口消息使用 ctx.event_hub.external()，停止时保留任务/资源至实际退出。

监督重建的激活失败现在会先补偿并在剩余累计预算内重试，预算耗尽才 FailFast；旧任务/服务能力失效，应用级 Hub 仍有效。需要依赖“激活失败立即全应用退出”的宿主应检查 FailurePolicy。框架版本仍为 0.0.2 开发树，已发布 v0.0.2 工件不含此变更；使用 Skill 1.0.6 / Contract 1.2.0 及新源码指纹。验证见[入口验收](validation/2026-11-06/module-ingress/README.md)。

## 0.0.3 Candidate 发布（2026-11-06）

0.0.3 包含上节 Module 入口增强及重建激活失败补偿；八个框架 crate、示例和模板统一版本，契约 revision 1.2.0，匹配 Skill 1.0.7。1.0.6 是 0.0.2 开发树的本地验证归档，0.0.3 须安装新指纹 Bundle。旧 v0.0.1 / v0.0.2 工件不覆盖。发布范围仍为 GitHub 预发布源码与 Candidate Skill Bundle，未 cargo publish。验证见 [0.0.3 验收](validation/2026-11-06/release-0.0.3/README.md)。
