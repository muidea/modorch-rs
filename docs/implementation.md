# 设计逐项实现与验收

本表对照 [完整目标设计](Modorch-RS-Framework-Design.md)，记录 0.1 实现的源码落点、自动化行为证据及边界。原设计文档保留为目标规范，不把可选高级能力或未执行的生产验证标成完成。

## 逐章对应

| 设计章节 | 对应实现 | 交付与边界 |
|---|---|---|
| 1 定位、目标、不变量 | core、runtime、Facade、MIT Manifest | 库形态；只有 Initiator / Module；所有业务资源实例隔离 |
| 2 总体架构 | [架构说明](architecture.md)、八个 crate | 显式 Builder、DAG、生命周期、服务、消息、任务和统一停机 |
| 3 工作区、独立应用、Feature | 根 Cargo.toml、crates/*、examples/*、tests/fixtures/external-app | resolver 3 / Edition 2024；六种单独 Feature 组合；外部 Workspace 实际接入 |
| 4 组件、描述符、Factory | runtime/component.rs、core/lib.rs | 对象安全 async-trait、Factory 配置验证、可选 Initiator EventEndpoint；一个组件一套执行域、多订阅 |
| 5 Builder 与激活 | runtime/lib.rs、registry/lib.rs | 默认空选择；先验证再创建；显式 IncludeRequired；Ready 后激活入口 |
| 6 服务与资源 | registry/services.rs | 依赖白名单、类型+Owner+名称定位、Owner 可用性、被 Tracker 计数的 ServiceLease |
| 7.1–7.3 信封、回执、路由 | eventhub/types.rs、lib.rs、request.rs | Event 部分投递、Command 仅准入、typed Request、显式限时等待入队、同步订阅回执、无缓存过期路径 |
| 7.4 Lane 与所有权 | eventhub/workers.rs | Serial、固定 Partitioned、受限 Concurrent、带上限及空闲回收的 Dynamic；有界 mailbox；同 Key FIFO |
| 7.5 循环等待 | eventhub/waitgraph.rs | 自等待、多级与独立 Root 等待环；取消后删除边；Concurrent 使用保守等待域 |
| 7.6 取消状态 | eventhub/request.rs | 排队取消不执行；Running 超时为 Indeterminate；观察完成状态；中止执行器仍有准确回执 |
| 8 任务和定时器 | scheduler/lib.rs | Owner 绑定、任务状态/完成回执、协作取消、全局任务/Timer 限额、三种 Tick 策略 |
| 9 生命周期、启动事务 | runtime/lib.rs | 按组和 DAG Setup/Start/Ready；记录钩子结果；部分初始化回滚；调用者取消不丢失实例所有权 |
| 10 检查式停机 | core/operation.rs、runtime/lib.rs | 原子关闭 Root、生命周期收尾因果链、全局计数固定点、超时保留、重试 checkpoint、独立 Forced 回执 |
| 11 监督、错误 | core/FailureReporter、runtime supervisor | 三种 FailurePolicy、按依赖闭包停用/重建、代际隔离、重试预算、Panic 捕获、结构化 source/phase/component/operation_id、可查询根因与技术重试性 |
| 12 健康、观测、边界 | Health、EventHub::metrics/mailbox_metrics、task_metrics、observe | App ID、组件/消息类型/Topic/路由结果、队列/Active Lane/任务/延迟快照、Drain 耗时、跨任务因果 Span；宿主负责 Exporter 和全局日志配置 |
| 13 集成示例 | examples 四个业务应用、runtime-load、external-app | 共享服务、私有 Module 状态、子请求、Ready 后 HTTP 入口、宿主信号 |
| 14 API 与兼容 | modorch/lib.rs、[API](public-api.md)、[兼容性](compatibility.md) | 0.1 公共 Facade、版本一致、无 unsafe 注册/业务全局单例；未发布 Registry 稳定版本 |
| 15 行为验收 | 下表与 CI | 核心协议通过集成测试；生产容量与长期稳定性不以单次测试推断 |
| 16 M0–M5 顺序 | 公共契约 → 实例装配 → 消息/Lane → Tracker/停机 → 配置/观测/示例/CI | 基础实现按此顺序完成；高级监督、有限负载/持续运行及 Loom 检查已补齐，实测边界见验证报告 |
| 17 强制边界与术语 | [消息语义](message-semantics.md)、[生命周期](lifecycle.md)、[接入指南](integration-guide.md) | 明确 Accepted / Completed、Timeout / NotExecuted、ShutdownTimeout / Stopped、Arc / ResourceReady 的区别 |

## 行为证据

| 原验收组 | 对应自动化测试/样例 |
|---|---|
| 独立 Cargo 接入 | tests/fixtures/external-app，包含外部定义 Factory / Module 与 TOML/观测 Feature |
| 多 Application 隔离 | application::applications_are_isolated_in_one_runtime；registry 服务隔离测试 |
| 重复注册、默认空选择、未启用不创建 | application::default_selection_is_empty_duplicates_and_unavailable_are_errors / only_enabled_factories_create_and_dag_orders_lifecycle |
| 缺失/禁用/自依赖/循环/跨层依赖与非法容量 | registry/tests/registry.rs |
| DAG 启动与逆序拆卸 | application::only_enabled_factories_create_and_dag_orders_lifecycle |
| 部分初始化安全补偿 | application::partial_setup_rolls_back_and_preserves_primary_error |
| Initiator 可选消息端点 | application::initiator_can_register_an_optional_event_endpoint |
| Module 私有状态、跨模块契约 | multi-module 应用，Storage Handler 私有持有订单列表 |
| 服务依赖、线程安全与失效 | registry::services_enforce_declared_dependencies_isolation_and_leases |
| 借入宿主资源、代际失效 | registry::borrowed_resources_remain_host_owned_but_handles_are_generation_scoped |
| 高级监督 | supervision 的 6 项测试：依赖闭包、旧代际屏障、重建失败、累计预算、关闭退避、旧故障去重 |
| Dynamic 与 Wait | concurrency 中动态回收/重建、循环等待与等待入队成功/取消/超时/停机/Fanout 测试 |
| 状态空间与负载验证 | core 的两个 Loom 模型，runtime-load 四种执行策略及 300 秒持续运行，见 [验证报告](validation.md) |
| 租约阻止 Owner 提前释放 | application::externally_held_service_leases_block_owner_teardown |
| Topic/Destination 双维度、Lane 不改变目标 | concurrency::named_destinations_and_topics_route_independently_of_lane_keys |
| 订阅回执及并发控制面 | concurrency::serial_fifo_dynamic_subscriptions_and_typed_requests / control_plane_changes_never_publish_stale_routes |
| Fanout 部分投递与精确单目标 | concurrency::bounded_fanout_reports_partial_delivery_and_ambiguous_requests |
| Serial FIFO、Partitioned 同 Key FIFO、多 shard 并行 | concurrency 中 Serial / Partitioned 测试 |
| Concurrent 严格上限 | concurrency::concurrent_workers_obey_strict_limit |
| 有界队列、竞争生产者与反压恢复 | concurrency::bounded_mailbox_recovers_under_competing_producers_without_losing_fifo，16 个生产者 / 1600 次投递 |
| 取消与超时区别 | concurrency::queued_timeout_cancels_but_running_timeout_is_indeterminate |
| 自等待、多级等待、独立 Root 环 | concurrency::rejects_self_and_cross_component_wait_cycles / detects_wait_cycles_between_independent_roots |
| Handler Panic、队列/执行器关闭回执 | concurrency::handler_panic_is_supervised_and_has_no_success_receipt / aborting_a_worker_completes_waiters_with_an_honest_result |
| Owner 任务、取消回执、Panic | scheduler/tests/tasks.rs，application FailFast 测试 |
| Timer 不占任务槽、Skip / QueueBounded / Overlap | scheduler/tests/tasks.rs，使用 Tokio 暂停时间 |
| 关闭 Root、保留合法子请求 | operations::shutdown_allows_children_but_zero_is_a_fixed_point，application::shutdown_timeout_preserves_resources_and_accepts_only_causal_children |
| 外部生产者不继承内部停机权限 | concurrency::external_ingress_cannot_inherit_a_producer_tasks_shutdown_privileges |
| 准入与关闭竞态 | operations::close_races_with_admission_without_lost_operations，50 轮多线程回归 |
| 停机超时保留资源、重试不重复释放 | application 的 Drain / Quiesce / checkpoint 测试 |
| 调用者取消启动后资源仍有归属 | application::cancellation_of_startup_does_not_abandon_created_resources |
| Forced 与 Graceful 分离 | application 的强制停机/中断 Drain 测试；tracing 测试验证两类结果日志 |
| 按标签观测与有界内存 | concurrency 的路由结果/应用隔离、标签溢出、超时与取消计数回归；保留 1024 个精确标签集和一个溢出集 |
| 执行错误上下文 | concurrency::handler_errors_keep_execution_identity_and_the_original_source；scheduler::task_errors_retain_operation_owner_phase_and_retryability |
| 部分初始化与 Drain 耗时 | application::partial_setup_rolls_back_and_preserves_primary_error、shutdown_timeout_preserves_resources_and_accepts_only_causal_children、forced_drain_is_interrupted_and_never_counted_as_completed |
| 因果 Trace 与外部根隔离 | tracing::traces_preserve_task_ancestry_separate_external_roots_and_log_shutdown_outcomes |
| 宿主 Runtime/信号/Subscriber | 所有组件共享宿主 Tokio Runtime；HTTP 示例在 main 中监听信号；框架源码无全局 Subscriber 或 process::exit |
| 配置先校验、未知字段/无效容量拒绝 | config/tests/composition.rs；Builder 中 Factory::validate_config 先于任何 create |

## 本地验收记录

验证日期：2026-10-31，stable Rust 1.94.0 与最低工具链 Rust 1.85.0。CI 配置见 `.github/workflows/ci.yml`，本地执行不代表远端 CI 已运行。

| 本地检查 | 结果 |
|---|---|
| cargo fmt --all -- --check；独立消费者 fmt | 通过 |
| cargo check --workspace --all-targets --all-features --locked | 通过 |
| cargo test --workspace --all-features --locked | 通过，94 项行为、模型与验收工具测试 |
| cargo clippy --workspace --all-targets --all-features --locked -- -D warnings | 通过 |
| 消息并发与准入期限（随全工作区测试运行） | 通过，35 项集成测试及 2 项准入期限单元测试；注入 Panic 由框架捕获 |
| Facade 无默认 Feature；单独 eventhub / scheduler / runtime / config-toml / observability | 全部通过 |
| cargo doc --workspace --all-features --no-deps，RUSTDOCFLAGS=-D warnings | 通过 |
| minimal-app / stateful-initiator / multi-module / graceful-http --once | 四个实际运行通过，HTTP 完成一次请求后排空 |
| 独立 external-app Workspace 编译及运行 | 通过；自定义 Factory / Module 只消费 Facade |
| git diff --check、实现文档相对链接检查 | 通过 |
| Rust 1.85.0 全工作区、全目标、全 Feature 检查 | 通过；cargo +1.85.0 check --workspace --all-targets --all-features --locked |

依赖使用根工作区与外部消费者各自提交的 Cargo.lock。初次依赖获取因代理耗时，后续本地 Cargo 检查使用 `--offline`；CI 使用 `--locked`。

## 边界与后续部署验证

- 此前已实现 Restart、StopDependents、动态 Lane、Wait 入队及显式 Owned / Borrowed 服务发布。本次复核补齐第 11–12 节的执行错误操作上下文、带容量上限的消息标签与路由结果指标、Drain 完成/中断耗时、跨 Task/Timer 的因果 Span，以及第 10/15 节要求的部分初始化状态和不同停机日志。具体接口见 [公共 API](public-api.md)。
- 当前代码验证包含 94 项测试、实际代码状态机的 Loom 检查和四种模式功能负载回归；新增混合负载、资源压力及动态恢复循环工具。原始数据及环境保存在 [验证报告](validation.md)。真实业务、多机、全天候稳定性与生产 SLO 需要在目标部署中验证，不用本机合成数据代替。
- Concurrent 同步重入采用保守拒绝。Handler 内使用等待式入队会显式拒绝，以避免邮箱容量死锁。
- 进程间 Transport、消息持久化、Exactly-once、跨资源事务和插件沙箱属于原设计非目标，不作为本轮未完成开发项。宿主继续管理信号、Subscriber、资源最终关闭和 Exporter。
- Restart 不自动重放有副作用消息；累计重建预算、失败补偿和代际句柄行为见 [生命周期](lifecycle.md)。Teardown 出错必须保留资源并支持重试，强制取消不保证外部副作用结束。

## 并发与内存优化收口

性能核对提出的 P0/P1/P2 项已落实：观察状态与回复分离；独立有界 CPU/阻塞 I/O 池与 Owner 额度；准入等待者、声明字节及元数据预算；精确 Topic/目标索引与通配预解析；Dispatch 复用指标句柄与锁外快照统计；容量预留后分配和端点唤醒；Concurrent 固定 Worker；Tracker Owner 计数和无快照排空；有界生命周期日志、Timer 退出即删及 Worker 采样清理。对应配置和取舍见 [公共 API](public-api.md)、[消息合同](message-semantics.md)。

上一轮新增 13 项针对性回归（消息 6、调度 5、应用 2），当时总计 74 项；验证记录与对照数据见 [performance-closure](validation/2026-10-31/performance-closure/README.md)。随后补齐执行资源有界等待、类别和 Owner 资源指标、真实退出回执、动态工厂锁外构造与超时复核，新增 16 项回归，总计 90 项。该轮源码的检查日志和复现方式见 [execution-closure](validation/2026-10-31/execution-closure/README.md)。生产 CPU 配额、真实混合业务与长时驻留仍需在目标环境验收。

## 混合负载与部署验收工具

新增 `examples/mixed-load` 和 `scripts/acceptance.py`，覆盖多 Owner 四类业务、有界饱和、端到端直方图、Payload 存活字节、资源排空、动态 Lane 创建回收及反复恢复。验收阈值从 TOML 和命令行读取，超标返回失败并保存报告；添加 4 项工具集成测试，验证正常资源归还、动态恢复循环、SLO 失败退出及非法配置拒绝，该次总计 94 项（历史计数）。CI 普通任务新增混合/恢复/资源压力 smoke，手动稳定性任务新增 300 秒活动负载及恢复循环，均上传 artifact。

工具说明见 [mixed-load](../examples/mixed-load/README.md)，本轮源码检查及测量见 [mixed-acceptance](validation/2026-10-31/mixed-acceptance/README.md)。远端工作流与生产阈值仍需在对应环境执行和确认。
