# 并发性能与内存评估及收口（2026-10-31）

结论：已按本次核对的 P0/P1/P2 优先级表落实并发与内存优化，并新增对应回归。包含独立有界执行资源、回复及时释放、准入预算、路由与指标热路径、Worker 复用及诊断保留上限。原始评估保留在下文；性能复测和边界见 [性能收口报告](validation/2026-10-31/performance-closure/README.md)，其后执行资源等待、真实退出回执及动态工厂的补充收口见 [当前代码验收](validation/2026-10-31/execution-closure/README.md)。这些改动与本机短测不代表生产 P99 或长时内存已经达到最优。

## 证据与测量边界

本节为优化前的评估基线，代码版本与上一轮 61 项测试验收的源码摘要相同。环境为 WSL2、8 个逻辑 CPU、4 个 Tokio Worker、Rust 1.94.0、release 构建，运行时未安装 Tracing Subscriber。各吞吐场景顺序执行 3 次，取中位数；这是共享宿主上的短时合成测试，没有测得锁等待时间的占比，不将代码中的潜在竞争当作已量化的瓶颈。

所有原始 JSON、DHAT 文件、探针源码和环境摘要位于 [performance-review](validation/2026-10-31/performance-review/environment.json)，汇总见 [summary.json](validation/2026-10-31/performance-review/summary.json)。

### 路由随订阅数增长

32 个生产者向同一目标执行总计 64000 次 u64 Echo Request。全部订阅都安装在该目标；除一个精确匹配 echo 外，其余为不匹配的精确 Topic。三组只有订阅数不同。

| 订阅数 | 吞吐中位数（请求/秒） |
|---:|---:|
| 1 | 124,872 |
| 128 | 48,092 |
| 1024 | 14,709 |

从 1 到 1024 个订阅，吞吐降至约 11.8%。当前 `targets()` 在应用级路由 Mutex 内扫描全部订阅，构造 BTreeSet/Vec，再由 unique_target 判断单目标。即便 Destination 已指定组件，也会先执行 Topic 匹配。这个结果支持优先建立精确 Topic 和目标索引、预编译通配模式，并缩短数据面锁的持有时间；不表示所有业务都将出现同样比例的下降。源码：[路由](../crates/modorch-eventhub/src/lib.rs)。

### 轻量请求与分配周转

每组运行 2 秒、3 次，邮箱容量 8。Partitioned 为 16 shard，Concurrent 为 16 并发，Dynamic 最多 128 Lane。每个生产者同时只等待一条回复。RSS 为 /proc 每约 25 ms 外部采样的最大值，单位 KiB；只能观察本次短窗口。

| 策略 | 生产者 | 吞吐中位数（请求/秒） | RSS 采样峰值 KiB |
|---|---:|---:|---:|
| serial | 1 | 89,282 | 4188 |
| serial | 64 | 75,170 | 4328 |
| partitioned | 1 | 74,875 | 4328 |
| partitioned | 64 | 122,740 | 4632 |
| concurrent | 1 | 51,221 | 4236 |
| concurrent | 64 | 47,293 | 4696 |
| dynamic | 1 | 86,873 | 4216 |
| dynamic | 64 | 113,335 | 5812 |

在这组轻量 Echo 上，Concurrent 的逐消息 JoinSet::spawn 路径没有取得更高吞吐。这不否定异步 I/O 的并发收益，也不构成将所有策略统一替换为 Serial 的依据。

另用 Valgrind DHAT 分别运行 Serial/Concurrent 各 8000 次成功请求，8 个生产者。所有分配量均包含示例和 Tokio 的开销，不能全部归因于框架；Valgrind 下的耗时不用于吞吐比较。

| 策略 | 累计分配字节 | 分配次数 | 平均分配次数/请求（含启动） |
|---|---:|---:|---:|
| serial | 9,062,764 | 200,356 | 25.04 |
| concurrent | 23,845,082 | 216,356 | 27.04 |

Concurrent 多出 16000 次分配，平均每请求增加 2 次；额外累计分配约 14.8 MB。结合源码中逐消息创建 Tokio Task 和 JoinSet 条目的路径，这支持评估 Worker 复用或面向异步 I/O 的批处理/调度方案。累计分配不是同时驻留量；两组 DHAT 在进程退出时均只剩 544 字节，不能把周转量称为内存泄漏。

每个成功 Request/RequestWait 还会分别在 RequestShared、Dispatch 和路由计数中查询同一指标标签，共 3 次；每次查询均在指标 Mutex 内克隆 ComponentRef/String 和 Topic/String。热点命中也会发生这些短期分配。建议一次查询取得可复用的指标句柄，通过 Dispatch 与 RequestShared 传递。源码：[指标注册表](../crates/modorch-eventhub/src/metrics.rs)、[执行器](../crates/modorch-eventhub/src/workers.rs)。

### 观察句柄保留回复

探针发送一条 16 MiB 回复，消费 RequestHandle::wait 的结果后释放返回值，同时保留只用于查询状态的 RequestObservation。弱引用确认回复仍存活；丢弃 Observation 后回复释放。RSS 从约 3.5 MiB 升至 19.5 MiB，丢弃观察句柄后回到约 3.5 MiB。结果见 [observation.json](validation/2026-10-31/performance-review/observation.json)。

原因是 wait() 克隆 RequestRecord.result，Observation 继续持有包含原回复的同一个 RequestShared。这是随观察句柄寿命延长的数据保留；若应用保存大量状态观察句柄，大回复会被一起保留。应将轻量状态观察与回复所有权分离，在消费或放弃结果后释放回复数据；保留超时后的完成状态，核对完成/超时竞态。源码：[Request 状态](../crates/modorch-eventhub/src/request.rs)。

### 三类业务混合运行

一般混合场景：每类各 4 个生产者、三类分别为短 Echo、Tokio sleep(1ms) 的异步 I/O、20ms 墙钟 busy loop 的 CPU 工作，持续 2 秒。比较直接在 Handler 中执行 CPU 工作，以及由宿主参考适配调用 spawn_blocking 并用 Semaphore 限为 2 并发。每客户端最多记录前 100000 次端到端延迟。

| CPU 执行方式 | CPU 完成数中位数 | I/O 完成数中位数 | I/O P99 中位数 ms | 短请求完成数中位数 | 短请求 P99 中位数 μs |
|---|---:|---:|---:|---:|---:|
| 直接执行 | 297 | 3,691 | 4.048 | 325,808 | 66 |
| 阻塞线程 + 2 并发参考适配 | 192 | 2,996 | 8.039 | 212,050 | 216 |

这一组参考适配的 I/O P99 反而变差，说明把 CPU 工作移到 spawn_blocking 加一个固定并发参数，并不能普遍提高性能。两种方式的 CPU 并发预算也不同；busy loop 以墙钟计时，不能把 CPU 完成数解释为同样计算量的吞吐。应以真实业务的 CPU 工作量、配额和 I/O 尾延迟来选择预算。

另作执行线程饱和探针：4 个 CPU Handler 用屏障同步开始，各忙等 100ms；随后由独立宿主线程提交一个 sleep(1ms) 的 I/O Request。对照组保持相同的 4 个 CPU 任务，通过阻塞线程执行，使 Tokio Worker 可以继续调度 I/O。各执行 3 次。

| CPU 执行方式 | I/O 请求耗时中位数 ms |
|---|---:|
| 直接占用 4 个 Tokio Worker | 101.277 |
| 阻塞线程参考适配，4 个 CPU 任务 | 3.480 |

饱和场景说明未让出执行权的 CPU Handler 可以拖延其他模块的 I/O 和超时处理。当前 Concurrent 只限制活动 Handler 数，未提供 CPU 线程隔离或跨模块的执行时间公平性。该探针验证隔离的必要性，不确定生产环境应配置多少 CPU 线程；参考适配也不等于已交付框架级 CPU 执行器。

## 建议调整顺序

| 优先级 | 调整 | 验收重点 |
|---|---|---|
| P0 | 分离状态观察和回复数据，完成/放弃后及时释放结果 | 弱引用和大回复回归；超时/完成竞态；观察者仍能查询完成状态 |
| P0 | 按工作类型提供宿主可配置的执行资源：异步 I/O、CPU、同步阻塞 I/O | CPU 饱和时其他模块 P99 和计时器抖动；任务有 Owner/限额/真实完成回执 |
| P0 | 增加准入等待者数量上限，以及可选的消息成本/字节预算、字段长度上限 | 大 Payload、突发等待者、动态 Topic/Lane；超额明确拒绝且不迟到执行 |
| P1 | 路由按精确 Topic/目标索引，通配模式预解析，减少单目标路径分配 | 1/128/1024 订阅；订阅回执生效、取消、代际重建、幽灵路由回归 |
| P1 | 每条消息复用一次取得的指标句柄，采集快照时缩短注册表锁范围 | 指标总量/溢出语义保持，分配次数降低，指标采样下 P99 不恶化 |
| P1 | 入队前预留容量、按端点/shard 唤醒等待者，减少无效重试和全局惊群 | 高反压时 CPU/分配量；公平性、取消安全、停机和路由代际复核 |
| P1 | 评估 Concurrent Worker 复用及有界批处理 | 分别比较短任务、异步 I/O、CPU 提交；保留活动上限及独立取消/监督语义 |
| P2 | Tracker 使用轻量计数判断排空，按 Owner 维护计数；保留必要因果状态 | 避免 drain 每次唤醒扫描并构造完整 PendingWork；Loom/停机回归通过 |
| P2 | 对恢复历史设置诊断保留预算，按需及时回收 Timer/Worker 的完成控制句柄 | 恢复/空闲/突发循环中存活对象与 RSS 回落；启动失败主因仍可查 |

### 执行资源应与顺序策略分开

ExecutionPolicy::Serial/Partitioned/Concurrent 决定同一组件的顺序和并发度；工作类型决定在哪种执行资源上运行。建议保留两个维度：轻量消息和异步网络/数据库驱动复用宿主 Tokio；CPU 计算进入受限 CPU 执行资源；同步数据库、文件或旧 SDK 的阻塞调用使用单独受限的阻塞 I/O 资源。CPU 和阻塞 I/O 共用一个槽位池容易互相挤占。具体预算由宿主的可用 CPU、业务耗时和目标 P99 确定，避免固定写死线程数。

新增 CPU/阻塞任务接口应接受拥有参数的同步闭包，绑定组件 Owner，并把 OperationLease/ServiceLease 移进实际执行闭包。Future 被取消或 JoinHandle 被丢弃不能代表闭包已结束；运行中的同步工作无法被安全强杀。检查式停机必须等真实工作退出，强制关闭继续保留独立且诚实的结果。宿主可以注入线程池；库不安装隐式的全局 Runtime 或全局计算池。

### 内存上限需要多层预算

现有队列限制条数，Partitioned 的驻留消息约为 shards × (capacity + 1)，Dynamic 约为 max_lanes × (capacity + 1)，Concurrent 为 capacity + max_in_flight。这些上限还不包括外部等待入队 Future、已经完成但仍被观察句柄保留的结果，以及业务自身状态。Event Fanout 使用 Arc 共享 Payload，不会为每个接收方深拷贝全部数据。

因此需要同时考虑队列条数、在途任务数、等待者数和消息成本预算。对 Arc<dyn Any> 使用 size_of_val 不能计算嵌套 Vec/Arc/连接池的真实堆大小；消息成本需由受信契约显式提供，或在传输/业务入口限制。框架也无法限制调用方自己长期持有的数据。指标当前虽然最多 1024 个精确标签集，但 Topic/String 长度未限制，条目数上限不能单独形成字节上限。应同时限制 Topic/LaneKey/Header 长度和数量，并给指标标签设置独立预算。

生命周期 journal 随恢复步骤追加；增长受到配置的累计重启预算限制，但该预算可以非常大，当前没有独立的诊断保留上限。Timer/动态 Worker 的完成控制句柄主要在后续创建时清理；这更接近峰值后的保留，不能未经测量就称为无限泄漏。

## 优化验收方式

每项优化先保留当前基线，逐项做 A/B；覆盖小/大 Payload、短/长 Topic、1/128/1024 订阅、不同模块及 Lane 数、过载、突发后空闲、恢复循环和三类混合业务。记录吞吐、端到端 P95/P99、定时器延迟、CPU、RSS/存活堆、每请求分配次数、等待者与队列深度。CPU 占用应计入真实配额，不能只按逻辑核数估算。

本轮短测不能证明长期 RSS 稳定或生产 SLO 达标。后续变更仍须通过原 61 项行为/模型测试、MSRV、Feature 组合和独立消费者验证；涉及 Tracker、路由线性化或资源生命周期的变更应补针对性并发回归，不以吞吐收益替代安全关闭语义。

## 复现

根目录运行 cargo build -p runtime-load --release --locked；使用 --mode、--seconds 2、--producers 1/64、--capacity 8 复现轻量消息场景。DHAT 命令示例：

```bash
valgrind --tool=dhat --dhat-out-file=/tmp/modorch-dhat.json target/release/runtime-load --mode concurrent --messages 1000 --producers 8 --capacity 8
```

独立探针源码和 Manifest 已保存。可在临时目录创建 src/，将 probe.rs 放到 src/main.rs，将 probe-Cargo.toml 和 probe-Cargo.lock 分别复制为 Cargo.toml/Cargo.lock；Manifest 中 path 需对应当前框架路径。cargo build --release --locked 后，可运行：

```text
modorch-perf-review observation
modorch-perf-review route 1024 2000
modorch-perf-review mixed inline
modorch-perf-review mixed isolated
modorch-perf-review saturation inline
modorch-perf-review saturation isolated
```

## 本轮代码收口与复测

上述优先级表的 P0/P1/P2 项均已对应实现，公共配置和行为变化见 [API](public-api.md) 与 [兼容性](compatibility.md)。性能对照时的 74 项行为/模型测试、Clippy、MSRV、Feature 组合、示例和独立消费者全部通过，该版本检查日志及源码摘要见 [verification.json](validation/2026-10-31/performance-closure/verification.json)。补充收口后的 90 项测试和检查另见 [当前验收](validation/2026-10-31/execution-closure/README.md)，下列性能数值仍对应此前对照版本。

最终 release 交替对照中，1024 个精确订阅的吞吐中位数从 16,289 提升到 145,233 请求/秒，约 8.92 倍。DHAT 8000 次请求的 Serial 分配次数从 25.04 降至 16.04 次/请求，Concurrent 从 27.04 降至 15.49 次/请求；Concurrent 累计分配字节从约 23.84 MB 降至 4.86 MB。16 MiB 回复在状态观察者仍存活时已可释放。相同 4 个 CPU 任务的线程饱和探针中，I/O 请求耗时中位数从 101.194ms 降至框架 CPU 执行资源下的 2.296ms。

取舍也记录在验收报告中：固定 Worker、通道及资源记账增加了少量常驻开销，本轮各组 RSS 采样峰值有所升高；2 线程 CPU 隔离的普通混合场景中，短请求 P99 从 45μs 升至 164μs，CPU 完成数也下降。因此这里交付的是可配置的资源隔离和已验证的特定热路径收益，没有承诺全业务吞吐、尾延迟和 RSS 同时下降。动态 Lane 工厂现已移至路由锁外，但仍需保持轻量，真实业务的资源配额和长时表现尚需目标环境验证。所有原始样本、脚本及可还原的基线源码见 [性能收口验收](validation/2026-10-31/performance-closure/README.md)。

## 混合负载与资源压力的持续验收

最新复测入口为 [mixed-load](../examples/mixed-load/README.md) 和 `scripts/acceptance.py`：四类工作同时存在于每个 Owner，固定迭代 CPU 计算与同步阻塞 I/O 均纳入测量；采用真实退出回执，避免结果可读后的收尾额度影响连续提交。配置和报告覆盖吞吐、端到端延迟、反压、资源占用、突发后空闲与恢复循环，源码检查和实测单独记录在 [mixed-acceptance](validation/2026-10-31/mixed-acceptance/README.md)。这套工具用于持续验收，不改变上文历史对照的含义，也不为生产环境预设 SLO。
