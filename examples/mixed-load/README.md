# 混合负载与资源验收

`mixed-load` 是独立消费者示例和可复现验收工具。每个 Owner 同时运行短消息、异步 sleep、固定迭代数 CPU 计算、同步阻塞 sleep 四类 Request。CPU 和阻塞 I/O 在 `isolated` 下走框架的独立资源池、有界等待和 `wait_finished`；`inline` 在 Handler 内直接执行，供隔离对照。所有生产者各自最多持有一个请求；这是有界客户端饱和测试，没有模拟无限外部到达流量或真实数据库吞吐。

```bash
cargo build -p mixed-load -p runtime-load --release --locked
target/release/mixed-load --profile examples/mixed-load/profiles/local.toml --seconds 5
target/release/mixed-load --profile examples/mixed-load/profiles/overload.toml --seconds 5
target/release/mixed-load --policy dynamic --recover true --seconds 25 --cycles 12 --payload-bytes 65536
```

每周期完成负载后等待真实在途工作清零，再按需向一个 Owner 提交故障 Command，等待代际推进和全应用就绪，随后进入空闲窗口。Dynamic 同时验证 Lane/Handler 回收与下周期重建；不同 Owner 轮流恢复。故障 Command 返回错误触发监督，正常 Request 内的容量错误作为业务结果计数，不触发意外恢复。

每条请求核对序列和计算结果；每个 Owner、每类工作都必须取得成功进展。空闲时检查消息字节/等待者、Task、CPU/I/O 槽位及 Payload 归还，Dynamic 另查活跃/构造中 Lane、Handler 和 Worker 控制项清零。停机必须 GracefullyStopped、pending=0、Payload/Handler=0，Handler 完成数必须等于收到的回复数。调用响应超时等不确定结果使验收失败，不能把它计作成功或自动重放。

stdout 输出单个 JSON，stderr 输出周期进度。配置、总量与按 Owner/类别的成功和拒绝计数、成功及全部尝试的端到端 P95/P99、资源等待/执行指标、定时器延迟、RSS/线程数、空闲窗口、停机结果均保留。延迟包含 Payload 分配、准入、Handler、同步任务退出及回复，以累计微秒二次幂桶上界估计；资源内部指标沿用框架合同。采样间隔默认 100ms，定时器延迟来自 Tokio 定时器实际被调度的延迟，不是独立硬件计时器。

统计内存有界：每个 Owner/类别只有固定直方图，活动和空闲样本各保留最近 256 条，周期数不会增加统计条目数；配置同时限制生产者数和它们可能持有的 Payload 总字节。`live_payload_bytes` 精确核对本工具创建的 Vec 长度，不能替代全进程存活堆。RSS 含框架、Tokio、采样 JSON 和分配器缓存，采样缓冲填满之前自身也会增长。初次空闲 RSS 已包含池预热；最后与首次空闲 RSS 差值仅用于配置的有限窗口验收。

配置文件使用 TOML，完整字段及默认值见 [config.rs](src/config.rs)。所有字段都可通过 `--字段名 值` 覆盖，下划线改用连字符；数组值使用 JSON，例如：

```bash
target/release/mixed-load --profile examples/mixed-load/profiles/deployment.template.toml --cycles 12 --seconds 25 --max-p99-us '[5000,20000,50000,50000]' --max-rejection-percent 0 --max-idle-rss-growth-kib 4096
```

上述阈值只是语法示例。部署模板的延迟、抖动、RSS 和线程阈值默认 0 表示禁用，拒绝比例默认允许 100%；必须按真实 CPU/内存配额、业务成本与 SLO 修改后才能据此验收。P99 和拒绝比例逐 Owner/类别检查；缺少成功进展仍失败。启用抖动检查时没有样本也失败；RSS/线程检查在无法采集 `/proc` 时失败；RSS 增量检查要求至少两个周期。线程数包括宿主 Tokio、两类资源池及进程自身，不能直接用 CPU 线程预算替代。

批量工具面向 Linux，不在测量期间构建或并行执行其他检查：

```bash
python3 scripts/acceptance.py --suite smoke --output /tmp/modorch-smoke
python3 scripts/acceptance.py --suite performance --baseline-binary /path/to/old/runtime-load --baseline-sha256 EXPECTED_SHA256 --dhat --output /tmp/modorch-performance
python3 scripts/acceptance.py --suite stability --soak-seconds 300 --cycles 12 --output /tmp/modorch-stability
python3 scripts/acceptance.py --suite stability --soak-seconds 86400 --cycles 96 --output /tmp/modorch-day
```

输出目录必须不存在，防止覆盖历史证据。保存源码/配置/二进制摘要、实际命令、宿主与 cgroup 配额、原始 JSON、stderr、外部每 25ms RSS/线程采样及 CPU 用户/系统时间；失败保留 stdout/stderr 与 failure.json。性能组按试次交替运行旧/新 Echo 二进制和当前版本 inline/isolated 混合场景，另扫描 1/128/1024 精确订阅。未提供基线时只报告当前结果；DHAT 需要 Valgrind，累计分配和退出存活量单独报告，其运行时间不用于吞吐比较。CPU/I/O 隔离组的实际并行预算不同，不能把它当成等 CPU 配额的性能排名；订阅扫描包含全部四类工作，不能从它单独量化纯路由收益。

长测总活动时长向上取整到完整周期，另加空闲/恢复/排空时间。无限期等待、公平 FIFO、同步工作强杀和全进程内存硬限制均不是此工具提供的承诺。远端 CI 和实际生产 SLO 需在对应环境执行后记录结果。
