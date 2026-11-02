# 独立应用接入

框架可通过本地路径或正式发布后的 Registry 版本引入。框架不规定应用目录，也不反向引用业务 Crate。完整可编译消费者见 [external-app](../tests/fixtures/external-app)。

建议使用独立 contract crate 保存 Topic 常量与请求/响应 DTO；Initiator 与 Module 各自提供无业务副作用的 Factory。Factory::validate_config 在所有实例创建前校验业务配置，真正连接端口、初始化数据库等放在 Setup。

应用装配顺序：

1. 解析宿主配置、Cargo Feature 和可用组件，注册所有本次编译可用的 Factory。
2. ComponentSelection 显式选择启用组件；缺失或未编译的组件在 Build 时报告 ComponentUnavailable，禁用强依赖报告 DisabledDependency。
3. 将队列、任务及停机参数显式传给 Builder / ShutdownOptions，再 Build 和 Start。
4. Ready 后接入外部流量；外部入口使用 ExternalEventHub，组件内部继续使用 EventHub。
5. 宿主选择信号或 handle.wait 的运行结果，无论哪种结果都等待 shutdown_checked，再处理退出码。

可选 TOML 适配：

```text
Pseudocode / Non-compilable：以下为装配步骤摘要，完整可运行来源为 external-app。

let config = modorch::config::CompositionConfig::parse(&input)?;
let mut builder = ApplicationBuilder::new(&config.application.name)
    .selection(config.selection()?)
    .queue_capacities(config.messaging.default_queue_capacity,
                      config.messaging.per_shard_capacity);
builder.register_module(MyFactory)?;
let handle = builder.build()?.start().await?;
handle.shutdown_checked(ShutdownOptions { timeout: config.grace_period() }).await?;
```

库内不读取环境变量、不寻找配置路径、不注册进程信号、不创建私有 Runtime、不安装 tracing Subscriber，也不调用 process::exit。需要共享宿主资源时，应用用 Initiator 包装外部 Arc，通过 publish_borrowed 显式声明 Borrowed。宿主负责底层资源的最终关闭；普通 publish 默认为 Owned，由 Initiator 关闭。框架提供实例内逻辑可用性和租约屏障，没有隐式全局 Registry 或自动所有权推断。

## 混合业务的资源配置

同一模块可以同时处理短消息、异步 I/O 和 CPU 工作。保持 Serial/Partitioned/Concurrent 的顺序合同，异步驱动在宿主 Tokio 执行，CPU 密集段使用模块保存的 TaskScheduler::spawn_cpu，同步旧 SDK/文件调用使用 spawn_blocking_io。闭包必须拥有输入；依赖服务在使用期间持有并移入 ServiceLease。两类资源有独立的线程/容量，同时受全局与 Owner 任务额度约束；收到 Backpressure 时由业务按其重试合同处理，也可用 spawn_cpu_wait / spawn_blocking_io_wait 在有界期限和等待者额度内等候准入。连续提交并需要确认额度归还时使用 wait_finished。

通过 ApplicationBuilder::execution_resources 调整池与 Owner 额度，通过 message_limits 配置等待者、元数据和可选的消息声明字节预算；启用任一字节预算后需为 MessageOptions::payload_bytes 提供可信成本。默认值、取消/实际退出边界及接口详见 [公共 API](public-api.md)，性能取舍见 [本轮复测](performance-review.md)。
