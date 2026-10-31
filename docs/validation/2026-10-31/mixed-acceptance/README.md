# 混合负载与持续验收收口（2026-10-31）

已补齐混合负载工具、资源饱和与恢复循环、可配置 SLO、当前版本性能/DHAT 复测和 CI 接线。94 项测试及完整检查通过；3 组 smoke、68 组性能/分配样本和一轮约 301 秒活动负载均通过行为核对。各组源码清单一致并已复核摘要，详见 [verification.json](verification.json)。这不是生产 SLO 或 24 小时稳定性认证。

## 实现与自动化

- [mixed-load](../../../../examples/mixed-load/README.md) 在每个 Owner 同时运行短消息、异步 I/O、固定迭代 CPU 和同步阻塞 I/O；按 Owner/类别核对结果与进展，记录成功和全部尝试的端到端直方图、反压来源、资源指标、定时器延迟、Payload 存活字节、RSS/线程及停机。
- 支持 Concurrent/Dynamic、突发—空闲和 Owner 轮流恢复；检查真实排空、Lane/Handler/Worker 控制项回收、Payload 和额度归还。活动/空闲样本各最多 256 条，Owner/类别直方图数量固定。
- TOML/命令行配置线程、总/Owner 容量、等待者、消息成本与成本预算；可检查逐 Owner/类别 P99、拒绝比例、定时器抖动、空闲 RSS 增量和线程数。超标返回失败退出码；[部署模板](../../../../examples/mixed-load/profiles/deployment.template.toml) 的阈值需由目标业务填写。
- [acceptance.py](../../../../scripts/acceptance.py) 顺序运行、外部每 25ms 采样 RSS/线程和 CPU 时间，保存原始结果、配置、源码/二进制 SHA-256、命令、宿主及 cgroup 配额。输出目录禁止覆盖；失败保留 stdout/stderr/failure.json。
- CI 常规任务增加混合、Dynamic 恢复和资源饱和 smoke；手动稳定性任务增加 300 秒混合恢复循环并上传 artifact。远端 CI 尚未执行。

新增 4 项工具集成测试，覆盖四类工作与资源归还、两周期动态恢复、P99 超标返回失败及非法配置拒绝。完整 fmt、全目标/全 Feature check/test/Clippy、MSRV 1.85、Feature 组合、Rustdoc、示例、Echo smoke 和独立消费者日志见 [checks](checks/)。

## 当前版本与历史基线

环境为本机 WSL2、8 个可用逻辑 CPU，release 构建；先完成编译及检查，再顺序测量。各吞吐组 2 秒、3 次，按试次交替运行，表中吞吐取中位数，RSS 为各次外部采样峰值的最大值。基线为此前性能优化之前保留的 Echo 二进制，SHA-256 `0e0eb0fb0d46547cc62ab680942fd76e6b9f5129d86addff253b73193ed2cd3d`；它不是上一轮 90 项测试的版本。可还原源码见 [历史基线归档](../performance-closure/baseline-source.tar.gz)。当前代码本轮主要新增验收工具，没有声称以下差异由本轮工具开发带来。

| 策略 | 生产者 | 历史基线请求/秒 | 当前请求/秒 | 比值 | RSS 峰值 KiB（前 → 后） |
|---|---:|---:|---:|---:|---:|
| serial | 1 | 80,951 | 68,970 | 0.85× | 4,216 → 4,488 |
| serial | 64 | 59,428 | 77,240 | 1.30× | 4,436 → 4,696 |
| partitioned | 1 | 75,691 | 58,854 | 0.78× | 4,276 → 4,520 |
| partitioned | 64 | 93,148 | 122,932 | 1.32× | 4,656 → 5,000 |
| concurrent | 1 | 48,627 | 55,780 | 1.15× | 4,236 → 4,856 |
| concurrent | 64 | 41,994 | 60,699 | 1.45× | 4,644 → 5,012 |
| dynamic | 1 | 63,733 | 62,694 | 0.98× | 4,204 → 4,452 |
| dynamic | 64 | 93,045 | 63,649 | 0.68× | 5,724 → 6,188 |

本轮同时观测到提升和回退：Serial/Partitioned 单生产者以及 Dynamic 64 生产者的中位数低于历史基线；Dynamic 64 的当前三次样本约为 63,649 / 59,871 / 92,419 请求/秒，波动明显。这里只报告观测，不从短测推断锁争用原因，也不承诺所有策略吞吐改善；对这些场景有容量要求时应在固定配额目标环境复测和剖析。所有原始 JSON、CPU 时间和摘要见 [performance](performance/)。

## 每 Owner 四类工作并存

每类总计 4 个客户端（2 Owner，各 2 个），CPU 为固定 100000 次整数迭代，异步 I/O 为 sleep(2ms)，同步阻塞 I/O 为 sleep(5ms)。inline 直接占用 Tokio；isolated 使用 2 个 CPU、4 个阻塞 I/O 线程及各自有界容量，两组实际并行预算不同。三次中位数，延迟为微秒二次幂桶上界估计，包含分配、准入、执行、真实同步工作退出及回复。

| 类型 | inline 成功数 | isolated 成功数 | P95 μs（前 → 后） | P99 μs（前 → 后） |
|---|---:|---:|---:|---:|
| short | 4,310 | 186,541 | 16,384 → 256 | 16,384 → 512 |
| async_io | 491 | 2,531 | 65,536 → 8,192 | 131,072 → 8,192 |
| cpu | 4,208 | 7,197 | 16,384 → 2,048 | 16,384 → 4,096 |
| blocking_io | 1,370 | 1,430 | 8,192 → 8,192 | 32,768 → 16,384 |

定时器调度延迟 P99 的三次中位数为 65,536 → 4,096 μs；混合组 RSS 峰值 6,484 → 6,712 KiB。这个合成饱和场景支持隔离的必要性，不能代表真实数据库、网络或等 CPU 配额的性能排名。1/128/1024 精确订阅的四类混合扫描亦完成；它包含工作成本，不能单独量化纯路由收益。全部样本中每 Owner/类型都有成功进展，最终回复与 Handler 完成数相等。

## 分配

DHAT 的 Echo 每组 8000 次回复，包含示例和 Tokio 启动；Valgrind 耗时不用于吞吐对比。累计分配是周转量，不是同时驻留量。

| 策略 | 版本 | 累计字节 | 分配次数 | 次数/回复 | 退出存活字节 |
|---|---|---:|---:|---:|---:|
| serial | baseline | 9,062,726 | 200,355 | 25.04 | 544 |
| serial | current | 4,482,833 | 128,351 | 16.04 | 544 |
| concurrent | baseline | 23,844,060 | 216,354 | 27.04 | 544 |
| concurrent | current | 4,858,187 | 123,155 | 15.39 | 544 |

当前四类混合负载另运行 2 周期、1024 次回复的 DHAT，退出存活 544 字节。该数值单独不能证明所有业务没有泄漏。原始 heap JSON 和 stderr 保存在 performance/，汇总见 [summary.json](performance/summary.json)。

## 约五分钟资源饱和、空闲与恢复

使用 overload 配置：4 Owner、每 Owner 每类 4 客户端（总计 64），CPU/阻塞 I/O 各 1 线程/1 容量，全局 Task 2、每 Owner 1，消息 Payload 64 KiB。12 周期，每周期目标活动时长 25 秒，随后恢复一个 Owner 并空闲 250ms；实际活动 301.076 秒、总墙钟 304.148 秒。

- 完成并核对 25,393,920 次回复、12 次恢复，无最终失败，GracefullyStopped，最终 pending=0。
- 12 个空闲窗口中 Payload、Task、消息字节/等待者、Lane、Handler 和 Worker 控制项均清零；两类槽位全部归还。每 Owner/类别均有成功进展。
- 空闲 RSS 为 15,996–16,724 KiB，首末 15,996 → 16,724 KiB，差值 +728 KiB；进程内部 RSS 采样峰值 19,472 KiB，外部峰值 20,528 KiB。观察到有限增长，不能称作严格平台或全天候稳定证明；分配器、统计缓冲及业务工作集均影响 RSS。
- 线程采样保持 7，Payload 存活峰值 3,932,160 字节；生命周期日志达到 64 条后保持上限，最后累计淘汰 20 条。活动样本只保留最后 256 条。
- CPU 与阻塞 I/O 在真实饱和下累计产生全局/Owner/类别额度拒绝尝试，等待后最终成功；这些重试计数不等于业务失败数。原始指标和空闲序列见 [动态恢复长测](stability/dynamic-recovery-soak.json)。

本机配置没有填写生产 RSS/延迟阈值，因此这里的 passed 表示行为不变量和已启用的配置检查通过，不表示已满足未知的生产目标。

## 复现与剩余环境验收

先按 [工具说明](../../../../examples/mixed-load/README.md) 完成 release 构建，测量期间不要同时编译或运行检查。以下输出目录须不存在：

```bash
python3 scripts/acceptance.py --suite smoke --output /tmp/modorch-mixed-smoke
python3 scripts/acceptance.py --suite performance --baseline-binary /path/to/pinned/runtime-load --dhat --output /tmp/modorch-mixed-performance
python3 scripts/acceptance.py --suite stability --profile examples/mixed-load/profiles/overload.toml --soak-seconds 300 --cycles 12 --output /tmp/modorch-mixed-stability
```

剩余的是环境验收：执行远端 CI；用真实目标 CPU/内存配额、外部 I/O 和业务成本填写部署模板，确认容量、延迟、拒绝比例与长时 RSS 指标；按实际需要执行 24 小时及更长窗口。当前没有新增生产框架功能缺项结论，也没有用本机五分钟合成结果替代这些验收。
