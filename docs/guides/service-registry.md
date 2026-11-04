# 共享服务与租约

> Applies to: modorch 0.0.2 开发树，按框架指纹匹配
> Capability status: Verified（仅[本次验证](../validation/2026-11-04/release-0.0.2/README.md)列出的命令与 Linux / Rust 1.94 范围）
> Last validated: 2026-11-04；本地日志，远程 CI NotRun

## Goal

发布和读取共享服务，通过租约保护实际使用期间的资源。

## Prerequisites

Rust >= 1.85、Cargo、完整仓库、锁定依赖缓存或首次联网获取依赖。应用拥有 Tokio Runtime；命令在仓库根目录运行。默认 Facade Feature 包含 runtime/eventhub/scheduler，配置和观测按需开启。首次接入先阅读[入门](../getting-started.md)。

## When to use / When not to use

多个组件必须借用同一窄资源接口时使用；仅通过消息暴露业务能力无需共享整个实例。

## Relevant contracts

[RES-001 / RES-002 / RES-003](../../contracts/resource-contract.md)。规范性行为以 Contract 为单一维护点；下文为任务步骤。

## Steps

Initiator 在 setup 建资源，export_services 用 ServicePublisher 发布 Arc 窄接口。读取者先声明 Owner 依赖，再由 ctx.services.get 获取类型化 ServiceHandle；每次操作 acquire，并在真实工作结束时归还 ServiceLease。宿主资源显式 publish_borrowed。

## Complete runnable example

完整来源：[examples/shared-service/src/main.rs](../../examples/shared-service/src/main.rs) 和 [Cargo.toml](../../examples/shared-service/Cargo.toml)。依赖仅 Facade、Tokio 及标准库，无数据库、凭据或公网服务。

## Expected observable behavior

Module 读取 greeting，关闭后 Owned 资源释放，示例断言 released=true。

## Failure cases and limitations

键含 Owner、名称和 TypeId；同类型同键重复拒绝，不同类型不同键。直接克隆业务服务内部裸句柄可能绕过租约；业务须保证每次外部操作可用性。

## Validation commands

```bash
cargo check -p shared-service --all-targets --locked
cargo run -p shared-service --locked
cargo test --workspace --all-features --locked
```

命令非零或断言失败即未通过。缺工具或离线缺缓存标记 NotRun，不能据静态阅读报告 Passed。

## Related API / Examples / Tests

[API 源码](../../crates/modorch-registry/src/services.rs)、[行为测试](../../crates/modorch-registry/tests/registry.rs)、[限制](../reference/limitations.md)、[兼容性](../reference/compatibility.md)。生成 API 参考：cargo doc --workspace --all-features --no-deps --locked，浏览 target/doc/modorch/index.html；doctest 需另行运行。
