# 执行资源与动态构造收口验收（2026-10-31）

本轮对上一轮验收的剩余接口和并发边界完成代码收口。当前源码通过 90 项测试及完整检查，源码摘要与检查清单见 [verification.json](verification.json)。此前的性能对照保留在 [performance-closure](../performance-closure/README.md)，其中的吞吐、P99、DHAT 和 RSS 数值仍对应当时版本。

| 收口项 | 当前行为 |
|---|---|
| 执行资源有界等待 | spawn_cpu_wait / spawn_blocking_io_wait 共享全局和 Owner 等待者限额；期限内获得三层额度后仅注册、提交一次，取消、超时和权限失效不会迟到执行 |
| 真实退出回执 | TaskHandle / BlockingTaskHandle 的 wait_finished 在任务自身捕获、额度、租约及控制记录归还后返回；容量 1 连续提交无需为收尾增配额度，同步任务直到实际退出仍保留记账 |
| 资源诊断 | 类别容量、排队、执行、等待者、反压来源、退出结果及耗时分位数；按 Owner 区分 Async/CPU/BlockingIo 分布和共享额度，空闲且无持有者的条目清理 |
| 动态 Lane 构造 | 锁内预留、锁外调用工厂、锁内复核安装；构造中 Lane 同样受 max_lanes 限制，失败、恐慌、取消订阅和恢复均清理失效预留，不阻塞无关路由 |
| 消息准入期限 | 重试前、获得路由锁后及实际入队前复核期限；容量通知与超时同时就绪时优先超时，零等待窗口不提交 |

相对此前的 74 项新增 16 项：7 项消息回归和 9 项调度回归。当前细分为应用 15、监督 6、Tracing 1、消息 37（含准入期限单元测试 2）、调度 21、注册表 5、操作 2、配置 1、Loom 2。

回归覆盖等待者上限、取消/超时、Owner 关闭与旧代际、共享 Owner/全局额度、真实释放后的唤醒、512 次并发资源争抢、容量 1 连续提交 256 次、执行器拒绝/丢弃、首次调度前中止的捕获释放及类别/Owner 指标；动态构造覆盖无关路由进展、构造槽位限制、取消订阅、超时、恐慌后重试与恢复期间不安装旧 Lane。

以下检查全部通过，原始输出保存在本目录对应 .log：fmt、all-targets/all-features check、全工作区测试、Clippy -D warnings、6 组 Facade Feature、rustdoc -D warnings、四个示例、四种策略各 800 次回复核对、独立 external-app 编译/运行/格式及 Rust 1.85 全目标全 Feature 编译。独立消费者仅通过 Facade 调用两类等待接口和退出回执，核对资源统计及闲置 Owner 条目的清理。

根目录复现：

```bash
bash docs/validation/2026-10-31/performance-closure/verify.sh docs/validation/2026-10-31/execution-closure
```

等待不承诺 FIFO；等待者数量限制不测量闭包捕获的深层堆。wait_finished 只确认任务自身持有的资源释放，调用方另行保留的结果、租约克隆和外部副作用仍由调用方负责。动态工厂在锁外仍同步执行，超时复核阻止后续入队，无法强行中断工厂；工厂仍应保持轻量。

本轮完成的是代码与行为验收，未重跑性能 A/B、DHAT、长期 RSS 或生产 SLO 测量。真实轻量消息、异步 I/O、CPU 与同步阻塞 I/O 同时运行时的配额、吞吐、尾延迟和突发后驻留，需要在目标部署中验证。
