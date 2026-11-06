# 生命周期与资源屏障

Build 验证全体选中组件、配置、DAG、容量和支持的 FailurePolicy，不执行 Factory::create。Start 顺序为：Initiator Setup/Export、Module Setup/Handler 安装、全体 Start、全体 Ready、激活入口。组内按 DAG；未启用组件没有实例、订阅或任务。

Factory 返回对象后，框架在调用 Setup 前登记实例所有权。因此 Setup 中途报错或 Panic 的对象同样进入 BeginShutdown、Quiesce 和 Teardown。lifecycle_journal 保存每次已执行生命周期阶段与结果。StartupError 保留启动主因、清理回执和句柄；清理异常不会覆盖主因。取消 start 的调用者等待后，拥有实例的后台启动事务继续运行，成功后自动进入检查式关闭。

检查式关闭依次执行：

1. 在同一 AdmissionGate 锁下保留生命周期最后一条因果链并关闭根准入，取消 TimerDriver 与 Owner Task 的主动生产者。
2. Initiator 后 Module 执行 BeginShutdown、Quiesce。钩子可在受控生命周期链下使用依赖服务或提交必要的收尾子请求。
3. 释放生命周期因果链，等待共享 Tracker 的消息、处理器、任务、TimerDriver、子操作及服务租约全部归零。此时没有活父操作再成功提交子操作，形成排空固定点。
4. Module 逆 DAG 后 Initiator 逆 DAG Teardown。已成功拆卸的组件保存 checkpoint，重试不重复调用；失败组件及其依赖保留。Teardown 应只释放资源，不再生成新业务工作。
5. 关闭邮箱执行器和 Tracker，状态进入不可逆的 Stopped。

ServiceHandle 只保存注册表定位和类型；acquire 持注册表锁检查 Owner 可用性，同时登记租约操作，返回不暴露裸 Arc 的 ServiceLease。模块只能取得自己及声明依赖提供的服务。服务逻辑不可用后 acquire 返回 ServiceUnavailable；已存在的租约在实际归还前持续阻止 Owner 拆卸。

ShutdownOptions.timeout 到期返回 StopIncomplete { phase, pending, errors }，不会取消正在运行的钩子或丢弃在途工作。再次 shutdown_checked 等待原事务；如果上一事务因钩子返回错误结束，则只重试未成功的 checkpoint。钩子返回错误意味着自身仍持有未释放资源；组件应自行保证失败重试安全。Runtime 被宿主提前销毁时，框架无法继续履行排空，宿主应在退出 Runtime 前等待关闭。

force_shutdown 中止框架 Future，不调用仍可能被使用资源的常规 Teardown，返回 Forced { pending }。这不证明外部操作已经停止，可能保留组件资源。Forced 后再次关闭继续返回 Forced；已 Stopped 的应用重复关闭幂等。

TaskHandle.cancel 和关闭请求只发送协作取消。TaskFuture 返回后才变为 Completed / Failed / Cancelled；任务无视取消会导致 StopIncomplete。强制中止任务有 Cancelled 回执，Panic 交给 Owner 的 FailurePolicy 处理，默认 FailFast。TimerHandle.wait 确认 TimerDriver 已退出，已派发任务仍有独立 OperationTracker 计数。

Interval 的 SkipIfRunning 跳过重叠 Tick；QueueBounded 最多保留指定数量待执行 Tick，超量跳过；AllowOverlap 受指定并发上限及全局任务限额约束。TimerDriver 不占任务执行槽位；系统任务槽位饱和时 Tick 不会造成无界等待队列。

## 按组件监督与重建

故障事件携带组件与代际，使用最多 1024 项的实例级故障邮箱。溢出升级为全应用故障，避免无限保存异常。只有应用监督器消费故障事件；旧代际事件不会作用于新实例，业务 Request 的正常错误仍通过请求结果返回。

StopDependents 计算故障组件及已启用、声明 requires / optional 依赖的传递闭包，关闭该集合面向外部的新准入，停止其任务与 Timer 生产者。已在闭包内执行的操作可完成必要子请求，其他组件继续运行。完成 BeginShutdown / Quiesce、闭包在途计数归零、旧邮箱及 Handler 退出后，再逆 DAG 拆卸；应用进入 Degraded。组件级服务借用同时保护读取组件与资源 Owner，外部持有的租约也会阻止拆卸。

Restart 使用相同屏障，旧资源拆卸完毕才推进代际，按 DAG 重新 Factory Create / Setup / Export / Handler / Start / Ready，最后开放对应准入并激活入口。重建阶段失败的部分实例先走完整补偿屏障，再尝试下一代。消息不自动重放；已执行失败请求的副作用需要业务幂等与恢复机制。

max_attempts 表示故障 Owner 在本 Application 生命周期内的累计重建预算，每次尝试（含 Setup 失败）消耗一次；不会因短暂 Ready 而重置，防止崩溃循环无限重建。达到上限、旧资源无法成功拆卸或入口激活失败时转为 FailFast。backoff 等待可被正常停机打断。完整停机与按组件恢复共享事务锁；强制停机会中止恢复监督 Future，保留 Forced 语义。

TaskScheduler、ServiceReader 和 ServiceHandle 绑定组件代际。旧任务提交器在新代际中返回 Closed，旧服务句柄返回 ServiceUnavailable，避免调用方在对象重建后继续通过旧能力访问资源。恢复历史见 ComponentHealth 与生命周期事务日志。外部 shutdown_checked 的超时仍只限制等待，不能通过超时绕过资源屏障。

## 普通 Module 的入口激活

Initiator 与 Module 均有默认无操作的 activate_ingress，所有相关 ready 通过后统一调用。TCP 入口仅在该 Hook listen，setup 可用 TcpSocket 预绑定。激活失败及 panic 进入检查式补偿；监督重建中的激活错误消耗同一次累计重启预算，补偿成功才可再试。旧任务和服务能力有代际约束，EventHub/ExternalEventHub 是应用级共享能力。详见 [入口契约](../contracts/lifecycle-contract.md)和 [HTTP 示例](../examples/graceful-http/src/main.rs)。
