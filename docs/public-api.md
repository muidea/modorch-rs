# 公共 API

统一入口是 `modorch` 和 `modorch::prelude::*`。源码示例均参与 Workspace 编译，具体方法签名以生成的 Rustdoc 为准：`cargo doc --workspace --all-features --no-deps`。

| 能力 | API |
|---|---|
| 装配 | ApplicationBuilder::register_initiator / register_module / selection / build |
| 依赖 | ComponentDescriptor::requires / optional，DependencyResolution::IncludeRequired 显式补齐强依赖 |
| 组件契约 | Lifecycle、Initiator、Module、InitiatorFactory、ModuleFactory |
| 生命周期上下文 | SetupContext、RuntimeContext、ModuleContext、ShutdownContext |
| 服务 | ServicePublisher::publish、ServiceReader::get、ServiceHandle::acquire、ServiceLease、ResourceOwnership |
| 消息 | EventHub::publish / command / request 及 *_wait，ExternalEventHub |
| 订阅 | subscribe / unsubscribe 返回含 generation 的生效回执 |
| 执行 | HandlerRegistration::Serial / Partitioned / Concurrent / Dynamic，HandlerFactory |
| 任务 | TaskScheduler::spawn / spawn_cpu / spawn_blocking_io / spawn_cpu_wait / spawn_blocking_io_wait / delay / interval，TaskHandle、BlockingTaskHandle、TimerHandle |
| 应用句柄 | health / wait / begin_shutdown / shutdown_checked / force_shutdown / event_hub |
| 诊断 | lifecycle_journal，ApplicationHandle::metrics，EventHub::metrics / message_metrics / mailbox_metrics，FrameworkError::retryable / operation_id / component / phase / root_cause |

相对设计草图的具体落点：

1. 具备消息能力的 Initiator 可从 `event_endpoint()` 返回 HandlerRegistration；Module 从 `handlers()` 返回 `Option<HandlerRegistration>`。同一组件通过 registration 中的多个 SubscriptionSpec 订阅多个主题，始终只有一套组件执行域。
2. `Lifecycle::ready` 是可异步等待的就绪屏障；`Initiator::activate_ingress` 在全组件 Ready 后调用。`start` 本身不得承担无限运行循环。
3. TaskScheduler 使用闭包接收框架创建的 CancellationToken，周期任务使用可重复构造 Future 的 TaskFactory。运行时上下文绑定 Owner，spawn 不接受调用方提供 Owner ID。
4. 消息返回值采用 `Arc<R>`，进程内 payload 使用 `Arc<dyn Any + Send + Sync>`。外部消息 source 为 None，内部 Handler source、causation_id、operation_id 和 Span 由框架补齐。
5. StartupError 保留主因、补偿结果和可再次清理的 ApplicationHandle。错误链保留原始 FrameworkError；停机补偿错误存于 StopIncomplete.errors。
6. ShutdownOptions.timeout 约束调用者等待时间。运行中的关闭事务继续持有资源及 checkpoint，不因调用者超时或取消而中断钩子。

FailurePolicy 支持 FailFast、StopDependents 和 Restart。StopDependents 停止故障组件及其传递依赖方；Restart 在旧代际完成拆卸后重建整个受影响集合，达到重试预算后执行 FailFast。健康快照暴露 ComponentHealth 的状态、代际、累计重启次数与最近故障。请求循环检测对 Concurrent 组件采用保守的组件级等待域，可能拒绝本可利用空闲并发额度完成的同步重入；调用方可改用非阻塞 Command / Event。

`ExecutionPolicy::Dynamic { max_lanes, idle_timeout }` 与 `HandlerRegistration::Dynamic` 配对使用。HandlerFactory 以单调递增的 Lane 实例 ID 创建独立 SerialHandler；逻辑 LaneKey 从消息读取。最大活跃 Lane 数与每 Lane 邮箱容量构成内存边界，空闲 Handler 被回收，不承诺跨回收保留私有 Handler 状态。

动态创建先在路由锁内预留 Key/实例 ID，再在锁外同步调用工厂，最后重新核对订阅、端点身份、准入权限及期限后安装。`max_lanes` 同时限制已活跃和正在构造的 Lane，邮箱指标的 `preparing_lanes` 暴露后者。同 Key 构造期间立即式提交返回 Backpressure；等待式提交可等待构造完成。取消订阅、关闭或恢复期间无法安装失效构造结果，失败/恐慌会释放预留。工厂应保持轻量；锁外调用仍是同步调用，期限只能阻止超时后入队，不能中断正在执行的工厂。

`command_wait` / `request_wait` / `publish_wait` 的最后一个参数为准入等待上限。Request 成功准入后返回 RequestHandle，其 `wait` 仍单独约束业务响应等待。ExternalEventHub 提供相同异步方法，强制建立新根身份。Handler 内调用 *_wait 会立即返回 InvalidConfig，防止受限执行域等待邮箱形成死锁。

`ServicePublisher::publish_borrowed` 显式标记宿主借入资源；`ServiceHandle::ownership()` 可查询 Owned / Borrowed。框架回收借入服务的应用内可用性，宿主负责底层资源最终关闭。资源 Owner 与读取组件的代际均绑定到句柄，旧代际句柄不能访问新资源。

## 错误与观测收口

Handler 和后台 Task 的执行错误由 `ExecutionFailed` 记录当前 `ComponentRef`、`operation_id`、`Phase::Handle` / `Phase::Task`，`source` 保留原始错误及跨组件调用的完整链。`root_cause()` 可识别原始 `CircularWait` / `TypeMismatch` 等错误；`retryable()` 沿错误链查询技术重试可能性，不保证业务幂等。生命周期钩子保留实际阶段，在有效计数作用域内记录操作 ID；未建立 Operation 的工厂/启动钩子使用 `None`，不伪造操作身份。准入前的路由/容量错误和专用取消回执继续使用原有错误变体。

`ApplicationHandle::metrics()` 汇总应用 ID、在途操作及 Owner、Hub 总量、消息标签、邮箱、任务和 Drain 快照。`EventHub::message_metrics()` 按组件、消息类型、实际 Topic 统计 Accepted、Unroutable、AmbiguousDestination、Backpressure、Closed、NotReady、AdmissionTimeout、CircularWait 及其他失败，同时提供执行中/完成/失败、调用超时、主动取消、执行前取消和 P95/P99。路由尚未确定目标时组件字段为 `None`；准入等待到期计入 AdmissionTimeout，容量重试仍分别计入 Backpressure。这些是框架尝试和等待结果，不是业务失败次数。

每个 Hub 最多保留 `MESSAGE_METRIC_LABEL_LIMIT`（1024）个精确标签集，新增标签聚合到 `overflow=true`、标签均为 `None` 的单独行；已有标签继续精确累计，溢出行仍保留路由总量和执行统计。计数自 Application 创建起累计；无数据分位数为 0，有数据时为 Handler 执行耗时的指数桶上界估计，不是端到端延迟。各子系统独立采样，整个快照不承诺同一时刻的一致性。

邮箱快照含应用 ID、组件、活跃 Lane 数和各 shard 深度；任务快照含应用 ID、Task/Timer 数和 Owner 分布。`drain` 统计全应用及依赖闭包恢复的实际 Drain 调用：开始/完成/中断次数、当前活动耗时、完成和中断累计耗时及最近完成耗时，单位为 `Duration`。调用者超时不结束后台 Drain；强制中止只计入 interrupted。耗时采用单调墙钟，不受 Tokio 测试虚拟时间影响。

Facade 同时启用 `runtime` 和 `observability` 时，可调用 `modorch::observe::ApplicationMetrics::capture(&handle)` 获取完整导出输入；只启用 `observability` 仍保留轻量的 `OperationMetrics` 转换，不引入 Runtime。Exporter 和全局 Subscriber 由宿主安装。框架在消息、Task、Timer 的执行边界传播受信因果 Span，外部入口创建独立根 Span；停机日志使用 `outcome=graceful/forced/incomplete` 区分完成、中止与等待超时。Setup 失败时组件标记为 `PartiallyInitialized`，生命周期日志中保留窗口内的状态快照在回滚后仍可查询。

## 混合负载执行与内存配置

ApplicationBuilder 新增 message_limits(HubLimits)、execution_resources(ExecutionResources)、journal_capacity(usize)；独立使用的 EventHub::with_limits 和 SchedulerRuntime::with_resources 接受相同配置，构建时验证。异步任务继续使用宿主 Tokio；CPU 与同步阻塞 I/O 使用各自的惰性有界线程池，工作类型与 Serial/Partitioned/Concurrent 顺序策略独立。模块可保存 Context 中绑定 Owner 的 TaskScheduler，供 Handler 调用：

```rust,ignore
let task = tasks.spawn_cpu(move |cancel| {
    // 输入和 ServiceLease 应拥有所有权并移入闭包；长计算应检查 cancel。
    calculate(input, service_lease, cancel)
})?;
let reply = task.wait().await?;
```

spawn_cpu / spawn_blocking_io 接受 FnOnce(CancellationToken) -> FrameworkResult<T>，返回 BlockingTaskHandle<T>；wait 消费结果句柄，task() 可取得只观察任务状态/错误的 TaskHandle。丢弃结果句柄或取消等待不代表同步工作退出。force_cancel 只对同步工作发出协作取消，Tracker 和资源槽位一直计数到实际退出；未开始的闭包在出队时检查取消，跳过业务执行。运行中闭包的错误/恐慌进入既有 Owner/代际监督。CPU 闭包内不隐式安装 Tokio Runtime。

默认 CPU 线程数为可用并行度减一（最少 1、最多 64），CPU 总容量为线程数的两倍；同步阻塞 I/O 为 8 个线程、32 个总容量。capacity 包括执行和排队。立即式准入超额返回 Backpressure；两类任务与异步 spawn 共享 task_limits 的全局任务额度，并共享每 Owner 默认 256 个任务的额度；不同类另有独立容量。宿主应按真实 CPU 配额和 P99 调整参数；可注入 BlockingExecutor 替换某一资源，其适配器负责及时入队、实际线程并发和最终执行/丢弃，框架仍执行总容量与 Owner 准入检查。默认池不会创建全局 Runtime 或全局池，Scheduler 销毁并且工作退出后线程随通道关闭退出。

`spawn_cpu_wait(task, timeout)` / `spawn_blocking_io_wait(task, timeout)` 在期限内等待全局、Owner 和类别额度，再注册并提交一次。`ExecutionResources.max_waiters` 默认 1024、`max_waiters_per_owner` 默认 128，两类资源与同一 Owner 的多个 Scheduler 句柄共享等待者额度；超额返回 ResourceLimit，等待到期返回 AdmissionTimeout。等待期间不占 Task 槽位或新增 OperationLease，但仍持有调用方移入的闭包及输入；取消等待会释放这些捕获和等待者额度，未准入任务不会迟到执行。关闭、Owner 封锁和代际变化会唤醒等待者并重新核对准入权限；已接受工作派生子操作仍遵循既有因果准入合同。不承诺 FIFO。此处 `*_wait` 是执行资源准入，与消息邮箱的等待入队接口不同；任务持有额度等待子任务时必须预留足够容量，避免一直等到超时。

`TaskHandle::wait` / `BlockingTaskHandle::wait` 表示结果可读；`wait_finished` 则等到任务自身捕获、OperationLease、额度及控制记录已释放后返回同一结果。容量为 1 的连续提交可使用 `wait_finished`，无需额外为收尾阶段增配额度。回执不代表调用方另行保留的结果、克隆租约或外部副作用已释放。取消同步任务只发出协作信号，实际运行结束前始终保留记账。

HubMetrics 增加 admission（等待者、等待/在途声明字节）和 worker_tasks（采样时回收已完成的动态 Worker 控制句柄）。TaskMetrics 增加 cpu_tasks、blocking_io_tasks，计入已接受的执行及排队工作。Timer.wait 在完成控制项删除后确认驱动退出。RuntimeMetrics 增加 journal_capacity 与 journal_dropped；lifecycle_journal 只返回最近的记录，默认保留 1024 项，StartupError.source 与组件 last_failure 独立于日志淘汰保存。

`TaskMetrics.cpu` / `blocking_io` 提供容量、可用槽位、排队、执行、等待者及累计完成/失败/启动前取消计数，并区分全局额度、Owner 额度、类别容量、等待者上限、等待超时与宿主执行器拒绝。反压计数按尝试累计，包含等待内部重试，不等于最终业务失败数；`admitted` 在注册任务时累计，执行器拒绝或丢弃会另计 executor_failures / canceled_before_start。排队和执行 P95/P99、准入等待 P99 为累计微秒指数桶估计，取消/超时也计入实际等待耗时，不是业务端到端延迟。`owner_resources` 按 Owner 提供共享额度、等待者及 Async/CPU/BlockingIo 排队与执行分布，收尾阶段已离开执行分布但仍可能短暂占用额度。快照各字段独立采样，不承诺瞬时等式一致。

`completed` 包含已开始后正常、失败或协作取消退出的工作，`failed` 是其中错误结果的子集。Owner 指标只保留当前有 Scheduler/任务/等待者持有的额度条目；没有存活持有者时清理条目，累计类别指标仍保留。
