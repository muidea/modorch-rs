# 错误、补偿与故障策略

> Applies to: modorch 0.0.3 开发树，按框架指纹匹配
> Capability status: Verified（仅[本次验证](../validation/2026-11-06/release-0.0.3/README.md)列出的命令与 Linux / Rust 1.94 范围）
> Last validated: 2026-11-06；本地日志，远程 CI NotRun

## Goal

处理启动错误、执行补偿与故障恢复，并验证主因和清理结果。

## Prerequisites

Rust >= 1.85、Cargo、完整仓库、锁定依赖缓存或首次联网获取依赖。应用拥有 Tokio Runtime；命令在仓库根目录运行。默认 Facade Feature 包含 runtime/eventhub/scheduler，配置和观测按需开启。首次接入先阅读[入门](../getting-started.md)。

## When to use / When not to use

为启动、消息、任务和退出错误设计恢复；不得把框架技术可重试性当业务幂等承诺。

## Relevant contracts

[LIFE-002 / LIFE-003 / LIFE-004 / LIFE-005](../../contracts/lifecycle-contract.md)。规范性行为以 Contract 为单一维护点；下文为任务步骤。

## Steps

build 错误在创建前处理；start 错误保留 source/cleanup/handle。ExecutionFailed 用 component/phase/operation_id/root_cause 定位业务根因。选择 FailFast、StopDependents 或 Restart，只有旧闭包排空和拆卸后才可重建。关闭超时保留资源并重试，强制结束报告 Forced。

## Complete runnable example

完整来源：[examples/lifecycle/src/main.rs](../../examples/lifecycle/src/main.rs) 和 [Cargo.toml](../../examples/lifecycle/Cargo.toml)。依赖仅 Facade、Tokio 及标准库，无数据库、凭据或公网服务。

## Expected observable behavior

示例注入 setup 失败，断言先 setup 后 teardown；主因保留，清理完成。

## Failure cases and limitations

普通 Handler 返回业务错误经 Request 返回，不等同于进程 Panic；Task 错误与 Handler Panic 进入监督。重启不重放请求，预算累计不重置；强制关闭可能保留资源。

## Validation commands

```bash
cargo check -p lifecycle --all-targets --locked
cargo run -p lifecycle --locked
cargo test --workspace --all-features --locked
```

命令非零或断言失败即未通过。缺工具或离线缺缓存标记 NotRun，不能据静态阅读报告 Passed。

## Related API / Examples / Tests

[API 源码](../../crates/modorch-core/src/lib.rs)、[行为测试](../../crates/modorch/tests/supervision.rs)、[限制](../reference/limitations.md)、[兼容性](../reference/compatibility.md)。生成 API 参考：cargo doc --workspace --all-features --no-deps --locked，浏览 target/doc/modorch/index.html；doctest 需另行运行。
