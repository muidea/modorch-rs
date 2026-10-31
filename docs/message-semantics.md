# 消息与并发语义

Topic 与 Destination 决定接收者；LaneKey 只用于目标组件的固定 shard 分配，采用稳定 FNV-1a。SubscriptionSpec 的 topic_pattern 支持 `+`、尾部 `#` 和 `:parameter`，匹配方向为订阅模式到实际主题。

| 调用 | 成功含义 | 满队列 | 调用者超时/取消 |
|---|---|---|---|
| publish | PublishReport 逐目标记录入队结果 | 明确返回每接收者拒绝状态；可判断部分投递 | 不等待 Handler 完成 |
| command | AdmissionReceipt，已被邮箱接受 | Backpressure | 不宣称业务已完成 |
| request | RequestHandle，wait 返回类型化业务结果 | Backpressure / AdmissionTimeout | 排队中取消后不执行；Running 返回 Indeterminate |

PublishPolicy::Reject 与显式 DropNewest 都保留逐目标状态；DropNewest 不产生排队等待。显式等待入队使用 command_wait / request_wait / publish_wait；默认同步方法仍立即返回。等待有独立超时，超时返回 AdmissionTimeout；future 在等待容量期间取消后不会迟到入队。Handler 内拒绝等待式准入，以避免执行域与邮箱容量之间形成等待环。没有订阅者返回 Unroutable；Request / Command 多目标返回 AmbiguousDestination。

成功准入发生在路由锁保护的容量预留后 Permit::send 边界；容量不足时不构造 Dispatch 或请求状态。Serial 按准入 FIFO 执行；Partitioned 同 shard 按准入 FIFO，不同 shard 可并行；Concurrent 由一个独立 Dispatcher 向 max_in_flight 个复用 Worker 交接消息，每个 Worker 同时执行一条消息，不承诺执行顺序。其空闲 Task 数为该额度加一个 Dispatcher，额度最多 4096。

订阅控制和路由使用同一互斥边界。订阅回执返回时变更已经生效，取消订阅只影响随后路由，已准入消息继续执行。多个匹配订阅指向同一组件时，一条消息只向该组件投递一次。

RequestHandle::wait 消费句柄，等待超时与 Handler 的启动/完成状态使用同一状态锁。丢弃句柄会取消仍在队列中的工作；运行中的工作仍由 Tracker 计数并继续执行。RequestObservation 只持有独立的原子状态，可查看超时后的完成状态；它不持有回复。wait 消费结果，丢弃句柄会释放已完成但未消费的结果，运行中放弃等待后的回复直接释放。框架不自动重试。销毁 Worker 时，Running 等待者收到 Indeterminate，Queued 等待者收到 CanceledBeforeStart。

WaitGraph 在 Request 入队前登记实际组件/shard 之间的边，拒绝 A→A、A→B→A、多级及独立根请求形成的等待环。边在等待结束、取消或句柄丢弃时删除。Command/Event 不增加同步等待边。该机制覆盖框架 Request，业务自行建立的 Mutex 等待或第三方阻塞不能由框架推断。

外部入口如果本身运行在 Owner Task 内，应使用 `hub.external().request(...)` 或对应 command/publish 方法建立新根请求。Handler 使用 ctx.event_hub 提交必要子操作，停机后仍可完成已经接受的因果链。

EventHub::metrics 提供累计准入/拒绝、队列长度、活动处理数、峰值、完成/失败、调用超时、排队取消和有界直方图 P95/P99 估计。延迟为 Handler 执行耗时，非端到端请求时延；直方图按微秒的 2 次幂分桶，最终桶饱和。业务 Topic 标签、采样和指标出口由宿主适配，避免框架自动创建无限基数时间序列。

动态 Lane 模式为每个活跃 LaneKey 创建独立邮箱与 SerialHandler，并为其分配不重复的等待图身份。达到 max_lanes 或单 Lane 邮箱满时返回 Backpressure。空闲超时后，Worker 在路由锁下再次检查邮箱为空，再删除 Lane，因此并发准入不会向已回收 Lane 提交工作。后续相同 Key 可创建新实例；成功准入的消息仍按该 Key FIFO。应用如需保留长期业务状态，应将其放在 Module 的显式共享私有状态中，不依赖空闲 Lane 副本。

publish_wait 的 Fanout 在快照目标之间并行等待，每个目标只准入一次。等待过程中取消订阅或更换路由会重新校验目标匹配，已准入的目标不重试；部分超时仍返回各目标明确状态。动态 Lane 在等待容量时可能回收和重建，提交前始终重新取得当前路由及 Lane，避免对旧 Sender 迟到提交。

## 准入预算与唤醒

EventHub::with_limits 或 ApplicationBuilder::message_limits 配置 HubLimits。默认最多 1024 个正在等待容量的提交、每目标最多 256 个；立即成功的提交不占等待名额。达到等待名额或等待字节上限返回 ResourceLimit；队列或在途字节额度不足返回 Backpressure，可由 *_wait 在期限内重试。超时、取消、路由变化均重新核对并释放等待预算，不会在 Future 销毁后迟到入队。容量通知按端点发送，订阅/代际变化及共享字节释放使用控制通知；另保留 5ms 的关闭状态复核。等待提交之间不承诺 FIFO 或无饥饿。

可分别配置 max_in_flight_bytes、max_in_flight_bytes_per_target、max_waiting_bytes 和 max_message_bytes。配置任一字节预算后，每次提交必须显式填写 MessageOptions::payload_bytes。单条消息大于在途或单条预算会立即返回 InvalidConfig；可容纳但暂时无额度的消息才等待。预算按声明值计入排队和执行的整个生命周期，Fanout 每个目标分别记账。它不深度测量 Any，也不覆盖调用方未轮询的 Future、业务状态或调用方保留的回复；入口和消息契约必须给出可信成本。

默认 Topic（包括 Named destination）与 LaneKey 各最多 1024 字节，Header 最多 32 项且键值合计最多 8192 字节，订阅总数最多 65536；这些值均可配置。消息指标独立限制为 1024 个精确标签及 1 MiB 标签字符串预算，超出后聚合到 overflow。精确 Topic 和 Component 采用索引，通配模式安装时解析；订阅增删、路由选择与入队仍共用同一线性化锁。
