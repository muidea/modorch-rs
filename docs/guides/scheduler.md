# 后台、周期和阻塞任务

> Applies to: modorch 0.0.2 开发树，按框架指纹匹配
> Capability status: Verified（仅[本次验证](../validation/2026-11-04/release-0.0.2/README.md)列出的命令与 Linux / Rust 1.94 范围）
> Last validated: 2026-11-04；本地日志，远程 CI NotRun

## Goal

创建受管后台、周期和阻塞任务，验证取消、排空和资源释放。

## Prerequisites

Rust >= 1.85、Cargo、完整仓库、锁定依赖缓存或首次联网获取依赖。应用拥有 Tokio Runtime；命令在仓库根目录运行。默认 Facade Feature 包含 runtime/eventhub/scheduler，配置和观测按需开启。首次接入先阅读[入门](../getting-started.md)。

## When to use / When not to use

长期、周期、CPU 或同步 I/O 工作使用受管 Scheduler；短异步消息可在 Handler 内处理。

## Relevant contracts

[SCH-001 / SCH-002 / SCH-003](../../contracts/scheduler-contract.md)。规范性行为以 Contract 为单一维护点；下文为任务步骤。

## Steps

从 Context 取得 Owner 绑定 TaskScheduler。spawn 接受取消闭包；单次延时用 delay、周期用 interval 与 TaskFactory，显式 TickPolicy。CPU 用 spawn_cpu、同步 I/O 用 spawn_blocking_io；闭包拥有输入及 ServiceLease。需要确认额度归还使用 wait_finished。

## Complete runnable example

完整来源：[examples/scheduled-tasks/src/main.rs](../../examples/scheduled-tasks/src/main.rs) 和 [Cargo.toml](../../examples/scheduled-tasks/Cargo.toml)。依赖仅 Facade、Tokio 及标准库，无数据库、凭据或公网服务。

## Expected observable behavior

单次任务执行，周期至少触发两次；timer.cancel + timer.wait 后停止 driver，应用排空后 tasks/timers 为零。

## Failure cases and limitations

没有 once/every/TaskSpec API；TaskGroup 只是 TaskScheduler 的类型别名，并非另一套任务管理器；Timer.wait 不等待所有已派发子任务。同步闭包无法安全强杀；池满与 Owner 限额需有界准入/失败策略。Skip 并非补偿所有漏 Tick。

## Validation commands

```bash
cargo check -p scheduled-tasks --all-targets --locked
cargo run -p scheduled-tasks --locked
cargo test --workspace --all-features --locked
```

命令非零或断言失败即未通过。缺工具或离线缺缓存标记 NotRun，不能据静态阅读报告 Passed。

## Related API / Examples / Tests

[API 源码](../../crates/modorch-scheduler/src/lib.rs)、[行为测试](../../crates/modorch-scheduler/tests/tasks.rs)、[限制](../reference/limitations.md)、[兼容性](../reference/compatibility.md)。生成 API 参考：cargo doc --workspace --all-features --no-deps --locked，浏览 target/doc/modorch/index.html；doctest 需另行运行。
