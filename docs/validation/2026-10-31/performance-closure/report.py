#!/usr/bin/env python3
"""Materialize the report from raw samples; does not run benchmarks."""
from pathlib import Path
import hashlib
import json
import re
import statistics

out = Path(__file__).resolve().parent
root = out.parents[3]
summary = json.loads((out/'summary.json').read_text())
samples = summary['samples']
median = statistics.median
rows = []
for n in [1,128,1024]:
    before = summary[f'route-{n}-baseline']
    after = summary[f'route-{n}-optimized']
    rows.append(f'| {n} | {before:,.0f} | {after:,.0f} | {after/before:.2f}× |')
route_table = '\n'.join(rows)
rows = []
for mode in ['serial','partitioned','concurrent','dynamic']:
    for producers in [1,64]:
        groups = {label:[samples[f'{mode}-p{producers}-{label}-{i}'] for i in [1,2,3]] for label in ['baseline','optimized']}
        rates = [median(s['requests_per_second'] for s in groups[label]) for label in ['baseline','optimized']]
        peaks = [max(s['external_sample']['rss_peak_kib'] for s in groups[label]) for label in ['baseline','optimized']]
        rows.append(f'| {mode} | {producers} | {rates[0]:,.0f} | {rates[1]:,.0f} | {peaks[0]:,} → {peaks[1]:,} |')
load_table = '\n'.join(rows)
dhat = {}
rows = []
for mode in ['serial','concurrent']:
    for label in ['baseline','optimized']:
        data = json.loads((out/f'dhat-{mode}-{label}.json').read_text())
        stats = {key:sum(point[key] for point in data['pps']) for key in ['tb','tbk','eb','ebk']}
        dhat[f'{mode}-{label}'] = stats
        rows.append(f"| {mode} | {label} | {stats['tb']:,} | {stats['tbk']:,} | {stats['tbk']/8000:.2f} | {stats['eb']:,} |")
dhat_table = '\n'.join(rows)
rows = []
for mode in ['inline','isolated']:
    data = [samples[f'framework-mixed-{mode}-{i}'] for i in [1,2,3]]
    cpu = median(s['stats']['cpu']['completed'] for s in data)
    io = median(s['stats']['io']['completed'] for s in data)
    io_p99 = median(s['stats']['io']['p99_micros'] for s in data)/1000
    short = median(s['stats']['short']['completed'] for s in data)
    short_p99 = median(s['stats']['short']['p99_micros'] for s in data)
    rows.append(f'| {mode} | {cpu:,} | {io:,} | {io_p99:.3f} | {short:,} | {short_p99:,} |')
mixed_table = '\n'.join(rows)
saturation = {mode:median(samples[f'framework-saturation-{mode}-{i}']['io_latency_micros'] for i in [1,2,3])/1000 for mode in ['inline','isolated']}
observation = {label:samples[f'observation-{label}'] for label in ['baseline','optimized']}
assert observation['baseline']['retained_after_reply_consumed']
assert not observation['optimized']['retained_after_reply_consumed']
assert observation['optimized']['released_after_observer_drop']
for name, sample in samples.items():
    if 'pending_after_shutdown' in sample:
        assert sample['pending_after_shutdown'] == 0, name
        assert sample['completed'] == sample['replies'], name
        assert sample['shutdown'] == 'GracefullyStopped', name
log = (out/'tests.log').read_text()
assert 'FAILED' not in log
count = sum(map(int,re.findall(r'test result: ok\. (\d+) passed',log)))
assert count == 74, count
verification = dict(passed_tests=count, breakdown={'application':15,'supervision':6,'tracing':1,'eventhub':30,'scheduler':12,'registry':5,'operations':2,'config':1,'loom':2},
    commands='verify.sh', checks=['fmt','check','tests','clippy','features','rustdoc','examples','load-smoke','external','msrv'],
    source_sha256=hashlib.sha256(b''.join(p.read_bytes() for p in sorted((root/'crates').rglob('*.rs')))).hexdigest(),
    baseline_archive_sha256=hashlib.sha256((out/'baseline-source.tar.gz').read_bytes()).hexdigest())
assert verification['source_sha256'] == json.loads((out/'environment.json').read_text())['source_sha256']
(out/'verification.json').write_text(json.dumps(verification,indent=2)+'\n')
(out/'allocation-summary.json').write_text(json.dumps(dhat,indent=2)+'\n')
report = f'''# 并发与内存收口验收（2026-10-31）

核对提出的 P0/P1/P2 项已实现并通过回归；以下为最终代码的独立复测。原始评估保留在 [performance-review](../performance-review/summary.json)，本次使用优化前保留的同一组 release 基线二进制，按试次交替运行，未在测量期间编译或并行运行检查任务。环境仍为 WSL2、8 个逻辑 CPU、4 个 Tokio Worker、Rust 1.94.0，无 Tracing Subscriber。短时合成结果不能代替真实业务长期验证。

## 实现与行为验收

| 核对项 | 收口结果 |
|---|---|
| 观察与回复所有权 | Observation 只保留原子状态；结果消费、完成后丢弃及运行中放弃均释放回复，保留超时后完成状态 |
| 工作类型隔离 | spawn_cpu / spawn_blocking_io 使用独立惰性有界池，可注入宿主执行器；全局、类别及 Owner 额度共同约束；同步工作直到实际退出仍计入 Tracker |
| 准入内存 | 等待者总数/每目标上限，可选的声明字节总额/每目标/等待/单条预算，Topic/Lane/Header/订阅限制 |
| 路由 | 精确 Topic/目标索引，通配模式安装时解析，单目标查询免建 BTreeSet/Vec，保留路由变更及入队线性化锁 |
| 指标 | 每 Dispatch 复用一个指标句柄；命中借用 key；快照锁外读取计数/直方图；1024 精确标签及 1 MiB 字符串预算 |
| 容量和唤醒 | 先预留队列再构造 Dispatch/请求状态，端点容量通知，控制变更/共享字节释放通知，取消与重试复核路由 |
| Concurrent | 一个独立 Dispatcher 和固定 Worker，空闲反馈及交接均有界；驻留上限仍为邮箱容量 + max_in_flight，无逐消息 Tokio Task |
| Tracker | 按活跃 Owner 计数，哈希表复用容量；Drain 直接检查在途状态，不构造 PendingWork |
| 保留与清理 | journal 默认最近 1024 条及淘汰计数，StartupError/last_failure 单独保存；Timer 退出即删，完成 Worker 控制项按创建/快照清理 |

[verify.sh](verify.sh) 全部通过：74 项测试、fmt、all-targets/all-features check、Clippy -D warnings、Rust 1.85、6 组 Facade Feature、rustdoc -D warnings、示例、四种模式各 800 次请求和独立消费者。细分与源码摘要见 [verification.json](verification.json)，原始输出见对应 .log。

新增 13 项回归，包括 16 MiB 回复四种退出路径、准入预算及取消、64 个生产者/32768 次容量重试、同步 Handler 并行进入、执行池隔离及 Owner 共享配额、同步取消真实记账、服务持有期间阻止拆卸、恐慌监督、Timer 清理及日志淘汰。另有 [40 次饱和重复探针](handoff-saturation-stress.json) 全部完成。notification-regression.log、worker-handoff-regression.log、worker-handoff-yield-regression.log 保留中间实现触发问题的记录；最终实现通过对应新增回归。probe-budget-regression.log 记录了探针将容量与客户端数设成相同值时的真实反压，最终探针明确为收尾工作留出容量，未改动框架释放记账的边界。

## 精确订阅规模

32 个生产者，总计 64000 次 u64 Echo Request；三次中位数，单位请求/秒。每组仅订阅数变化。

| 订阅数 | 基线 | 优化后 | 比值 |
|---:|---:|---:|---:|
{route_table}

这是精确 Topic 索引对应场景的收益，不代表大量通配匹配或昂贵工厂有相同比例。

## 轻量消息

每组 2 秒、3 次；表内为吞吐中位数及外部 /proc 每 25ms 采样的三次最大 RSS（KiB）。邮箱容量 8，Partitioned/Concurrent 为 16 shard/并发，Dynamic 最多 128 Lane。

| 策略 | 生产者 | 基线请求/秒 | 优化后请求/秒 | RSS 峰值 KiB（前 → 后） |
|---|---:|---:|---:|---:|
{load_table}

各组完成数与回复数一致、无最终失败、GracefullyStopped、最终 pending=0。CPU 用户/系统时间与逐次数据保存在 JSON 的 external_sample；RSS 含宿主、示例和分配器。固定 Worker 增加空闲 Task/通道，相比按消息创建 Task 减少分配周转，不能据此承诺每个 RSS 样本都降低。

## 分配与回复释放

DHAT 每组 8000 次请求、8 个生产者；包括框架、示例和 Tokio 启动。Valgrind 耗时不用于吞吐比较。

| 策略 | 版本 | 累计分配字节 | 分配次数 | 次数/请求（含启动） | 退出时剩余字节 |
|---|---|---:|---:|---:|---:|
{dhat_table}

累计分配是周转量，退出剩余字节不单独证明泄漏；RSS 也不等于存活堆。汇总见 [allocation-summary.json](allocation-summary.json)。16 MiB 回复探针弱引用确认：基线消费后仍保留，优化后消费后即释放，即便 Observation 仍存活。原始证明和 RSS 见 [observation-optimized.json](observation-optimized.json)，回归还覆盖放弃/超时及完成后未消费的释放。

## 三类业务同时运行

每类 4 个客户端，CPU 为 20ms 墙钟 busy loop，异步 I/O 为 sleep(1ms)，短消息为 Echo，持续目标 2 秒。inline 直接在 Handler 执行 CPU；isolated 使用框架 spawn_cpu，2 个 CPU 线程、8 个总容量，每 Owner 8 个任务，模块 Concurrent 上限仍为 4。两种配置的 CPU 执行预算不同；busy loop 不是固定指令工作量。表内为三次中位数，延迟为请求入队到回复的端到端采样。

| CPU 方式 | CPU 完成 | I/O 完成 | I/O P99 ms | 短请求完成 | 短请求 P99 μs |
|---|---:|---:|---:|---:|---:|
{mixed_table}

本组混合场景中，隔离后的短请求 P99 从 45μs 升至 164μs，CPU 完成数也降低；本轮没有把执行隔离描述为所有负载的性能提升。

相同 4 个 CPU 任务、屏障同步后各忙等 100ms 的饱和探针中，inline 的 I/O 请求耗时中位数为 **{saturation['inline']:.3f}ms**，框架 4 线程/4 容量的 CPU 资源为 **{saturation['isolated']:.3f}ms**。这是执行隔离的直接证据；仍需按部署 CPU 配额和真实业务调优，不能从单个饱和场景外推所有业务 P99。

## 边界与复现

等待提交不承诺 FIFO；字段和声明字节预算不测量 Any 的深层堆，也不限制调用方自行保留的结果/状态。同步闭包必须主动使用隔离接口、拥有输入并持有使用期间的 ServiceLease；运行中的同步工作无法安全强杀，丢弃等待句柄不释放真实工作记账。CPU/I/O 默认线程池在首次使用时启动配置的线程，池及其空闲线程随 Scheduler 对象生命周期保留；高 Worker/线程额度会增加空闲资源，应按真实并发设置。

动态 Lane 工厂仍在创建临界区内同步调用，以保留创建/代际/路由的一致性；工厂应保持轻量，本轮没有量化其锁等待占比。没有新增 24 小时或生产 SLO 验证，也没有将旧版本 300 秒运行结果算作本版证据。

可用 [baseline-source.tar.gz](baseline-source.tar.gz) 在独立临时目录还原基线 workspace，按其 Cargo.lock 构建 runtime-load；基线探针源码和 Manifest 见 ../performance-review/probe.rs 及 probe-Cargo.toml。当前探针见 [probe.rs](probe.rs)、[probe-Cargo.toml](probe-Cargo.toml) 和锁文件；独立创建 src/main.rs 后构建，Manifest 的路径需指向对应源码目录，两个版本使用各自 target 目录。使用 `python3 compare.py 基线二进制目录 优化后探针路径` 运行交替对照，需要 Valgrind；可加 --skip-dhat 跳过分配阶段。运行 [report.py](report.py) 从原始文件生成本报告。执行命令、二进制和源码摘要见 [environment.json](environment.json)。
'''
(out/'README.md').write_text(report)
print(json.dumps(dict(tests=count, saturation=saturation, allocation=dhat),indent=2))
