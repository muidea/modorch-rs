# 开发 Initiator 与 Module

> Applies to: modorch 0.0.1 开发树，按框架指纹匹配
> Capability status: Verified（仅[本次验证](../validation/2026-11-02/release-0.0.1/README.md)列出的命令与 Linux / Rust 1.94 范围）
> Last validated: 2026-11-02；本地日志，远程 CI NotRun

## Goal

开发 Initiator 与 Module，验证构建、运行和资源释放。

## Prerequisites

Rust >= 1.85、Cargo、完整仓库、锁定依赖缓存或首次联网获取依赖。应用拥有 Tokio Runtime；命令在仓库根目录运行。默认 Facade Feature 包含 runtime/eventhub/scheduler，配置和观测按需开启。首次接入先阅读[入门](../getting-started.md)。

## When to use / When not to use

对外共享受控资源用 Initiator；封装业务行为和私有状态用 Module。普通 Rust 类型不必升级为组件。

## Relevant contracts

[COMP-001 / MSG-REQ-004](../../contracts/component-contract.md)。规范性行为以 Contract 为单一维护点；下文为任务步骤。

## Steps

实现描述符和 Factory；描述符稳定且与实例一致，资源连接放 setup。声明 requires / optional，Module::handlers 返回可选的单套注册；选择 DTO、Topic owner、类型和超时语义，测试正常与失败路径后接入 Builder。

## Complete runnable example

完整来源：[examples/request-response/src/main.rs](../../examples/request-response/src/main.rs) 和 [Cargo.toml](../../examples/request-response/Cargo.toml)。依赖仅 Facade、Tokio 及标准库，无数据库、凭据或公网服务。

## Expected observable behavior

示例打印 request-response: completed=42, timeout=Indeterminate，最终排空。

## Failure cases and limitations

Handler 对象和 Lifecycle 对象分离。Serial Handler 可维护私有状态；动态 Lane 回收不保存副本。不要把一个 Module 的完整实例发布为服务。

## Validation commands

```bash
cargo check -p request-response --all-targets --locked
cargo run -p request-response --locked
cargo test --workspace --all-features --locked
```

命令非零或断言失败即未通过。缺工具或离线缺缓存标记 NotRun，不能据静态阅读报告 Passed。

## Related API / Examples / Tests

[API 源码](../../crates/modorch-runtime/src/component.rs)、[行为测试](../../crates/modorch/tests/application.rs)、[限制](../reference/limitations.md)、[兼容性](../reference/compatibility.md)。生成 API 参考：cargo doc --workspace --all-features --no-deps --locked，浏览 target/doc/modorch/index.html；doctest 需另行运行。
