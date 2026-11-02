# 负载、并发与稳定性验证

日期：2026-10-31。原始 JSON 与环境、源码及二进制 SHA-256 位于 [2026-10-31](validation/2026-10-31/environment.json)。本报告记录已执行的有限验证，不将合成结果解释为真实业务容量或全天候生产认证。

## 本机实测

下表是观测/错误上下文收口之前版本的历史负载数据，原始文件记录对应源码与二进制摘要。观测/错误上下文收口后的源码另行执行 61 项行为/模型测试及四种模式各 800 次回复的功能负载回归；不将历史吞吐与 300 秒运行结果当作修改后版本的新性能证明。该轮功能回归原始数据见 [收口验证](validation/2026-10-31/closure/environment.json)。

环境为 WSL2 Linux x86_64、8 个逻辑 CPU、Rust 1.94.0，Tokio 使用 4 个 Worker 线程。使用 release 构建，64 个受限生产者，每个生产者同时等待一个 typed Request，邮箱容量 8。Partitioned / Concurrent 各 16 个 shard / 并发额度；Dynamic 最多 128 个 Lane，空闲回收时间 100 ms，本次持续负载使用 64 个 Key。

| 执行策略 | 秒 | 完成并核对的回复数 | 请求/秒 | RSS KiB（启动 → 停机后） | 停机 ms |
|---|---:|---:|---:|---:|---:|
| serial | 20 | 2,006,596 | 100,329 | 3,988 → 4,264 | 0.182 |
| partitioned | 20 | 3,721,787 | 186,088 | 4,168 → 4,556 | 0.262 |
| concurrent | 20 | 1,214,875 | 60,742 | 3,988 → 4,520 | 0.209 |
| dynamic | 20 | 3,615,151 | 180,756 | 4,040 → 5,560 | 0.368 |
| dynamic-soak | 300 | 44,797,881 | 149,326 | 3,936 → 5,520 | 0.607 |

所有场景的完成次数与回复次数一致，无失败结果，GracefullyStopped，最终 pending_operations=0。Serial / Concurrent 因小邮箱产生反压，`admission_rejections` 计数包含 *_wait 内部重试的容量拒绝尝试；该数值不代表最终业务请求失败或丢失。

Dynamic 的 300 秒持续运行完成 44,797,881 次回复核对。29 次运行中 RSS 采样为 5,520–5,628 KiB，停机后 5,520 KiB。样本没有出现持续攀升；该窗口不能证明 24 小时稳定性或任意业务 Handler 的内存边界。不同策略共享宿主资源、运行时段和并发结构不同，此表用于复现及回归观察，不作为性能排名。

吞吐为完成请求数 / 墙钟时间。JSON 中 P95/P99 来自 Handler 执行时间的指数桶估计，不是端到端请求延迟。负载只执行进程内 Echo，不包含数据库、网络、持久化或真实外部副作用。空闲 Lane 的回收/重新创建、Fanout 部分超时、重建故障与在途资源屏障通过确定性回归测试单独验证。

## 复现

```bash
cargo build -p runtime-load --release --locked
for mode in serial partitioned concurrent dynamic; do
  target/release/runtime-load --mode "$mode" --seconds 20 --producers 64 --capacity 8 > "$mode.json"
done
target/release/runtime-load --mode dynamic --seconds 300 --producers 64 --capacity 8 > dynamic-soak.json
```

固定请求数的 CI smoke 使用 `--messages 100 --producers 8`，每个生产者各执行 100 次。持续时长可通过 `--seconds` 扩展，例如真实部署环境执行 `--seconds 86400`；本次没有执行 24 小时运行。程序遇到类型/关联结果错误、任务失败、计数不一致、停机后残留或非正常关闭会返回失败退出码。

`.github/workflows/ci.yml` 的普通验证包含四种模式的固定请求数 smoke。手动触发 workflow_dispatch 时，stability job 执行各模式 30 秒及 Dynamic 900 秒，保留 JSON artifact；远端工作流尚未在本轮执行。

## 状态空间与故障回归

```bash
cargo test -p modorch-core --lib --locked
cargo test -p modorch --test supervision --locked
cargo test -p modorch-eventhub --test concurrency --locked
```

Loom 使用真实 OperationTracker 的状态与算法，将其状态互斥锁在单元测试构建中替换为 Loom Mutex，枚举两个线程之间 Root Admit / Close，以及 Owner Admit / Close / Generation Advance 的交错。模型不覆盖 Tokio Notify、异步执行器、OS 或任意外部资源。

监督回归覆盖故障依赖闭包、无关组件持续运行、旧 Handler/Task/ServiceLease 排空、旧代际句柄失效、重建 Setup 失败补偿、累计预算耗尽、退避期间停机和旧代际故障去重。消息回归覆盖动态 Lane 数量上限、空闲回收后重建、同 Key FIFO、循环等待，以及 Wait 成功、超时、取消、关闭和 Fanout 部分结果。

## 观测与错误上下文回归

新增测试验证按应用/组件/消息类型/Topic 的路由及执行统计、1024 标签上限后的溢出聚合、主动取消和调用超时分离、准入失败不污染取消计数、执行错误的操作身份与原始 source、Setup 部分初始化快照，以及强制中止 Drain 不计作完成。Tracing 集成测试使用宿主线程局部 Subscriber，核对 Handler → Task → 子请求的父子关系、外部消息的新根 Span，以及 graceful/forced 停机日志。

优化前验收共 61 项测试：13 项应用生命周期、6 项监督、1 项 Tracing、24 项消息并发、7 项调度、5 项注册表、2 项操作计数、1 项配置和 2 项 Loom。具体检查命令与结果见 [验收表](implementation.md)。

## 并发与内存优化评估

针对异步 I/O、轻量消息、CPU 任务同时存在的场景，追加了 release 短测、订阅规模扫描、16 MiB 回复保留探针、CPU 执行线程饱和探针及 DHAT 分配分析。结论与优化优先级见 [性能与内存评估](performance-review.md)，原始数据见 [汇总](validation/2026-10-31/performance-review/summary.json)。这些评估数据不替代生产环境长期稳定性验证。

## 性能与内存优化验收

上一轮性能优化新增 13 项回归，当时共 74 项：15 项应用、6 项监督、1 项 Tracing、30 项消息、12 项调度、5 项注册表、2 项操作、1 项配置及 2 项 Loom。覆盖大回复消费/放弃/超时后的弱引用释放、字节/等待者/元数据超限、取消无迟到执行、执行池独立性、Owner 配额、同步任务真实完成、服务资源停机屏障、恐慌监督、Timer 控制项清理及日志淘汰后主因保存。

完整检查脚本及日志、相同基线的 release 交替 A/B、框架执行池混合负载和饱和探针、DHAT 分配分析见 [本轮验收](validation/2026-10-31/performance-closure/README.md)。之前的历史数据及性能评估保留，不用它们替代本轮代码的测量。

## 执行资源与动态构造补充验收

补充收口新增 16 项回归，当前共 90 项：15 项应用、6 项监督、1 项 Tracing、37 项消息（含 2 项准入期限单元测试）、21 项调度、5 项注册表、2 项操作、1 项配置及 2 项 Loom。新增范围为有界资源等待与共享限额、取消/超时/Owner 关闭、真实退出后的额度归还、执行指标、异步任务首次调度前中止、512 次并发资源争抢，以及动态工厂锁外构造、构造槽位限额、取消订阅、失败/恐慌和恢复期间的失效结果清理。

源码摘要、完整检查日志及复现命令见 [当前验收](validation/2026-10-31/execution-closure/README.md)。本次没有重跑此前的性能 A/B、DHAT 或长期 RSS 测量，不将旧版本性能数据作为当前版本的新证明。

## 混合负载及持续验收工具

在上述 90 项框架回归之上新增 4 项 mixed-load 工具集成测试，该次验收共 94 项（历史计数）。工具通过每个 Owner 的四类工作和结果校验记录端到端 P95/P99、吞吐、反压、类别/Owner 资源统计、定时器延迟、Payload 存活字节、RSS 和线程数。动态模式支持突发—空闲、Lane 回收重建及轮流 Owner 故障恢复。配置可设置延迟、拒绝比例、抖动、空闲 RSS 增量和线程阈值；未填写真实目标的模板不产生生产 SLO 通过声明。

`scripts/acceptance.py` 保存源码/配置/二进制摘要和原始输出，顺序执行当前版本与指定历史基线的 Echo A/B、四类混合工作 inline/isolated 对照、订阅规模扫描、可选 DHAT 及恢复长测。完整参数和测量边界见 [工具说明](../examples/mixed-load/README.md)，本轮实际检查和采样另见 [mixed-acceptance](validation/2026-10-31/mixed-acceptance/README.md)，不沿用此前测量数值。

CI 常规验证增加混合、动态恢复和资源饱和 smoke，手动 stability 任务增加 300 秒活动时长的动态混合恢复循环并保留完整 artifact。远端工作流尚未在本轮运行。
