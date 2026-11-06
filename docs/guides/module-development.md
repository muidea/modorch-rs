# 开发 Initiator 与 Module

> Applies to: modorch 0.0.3 开发树，按框架指纹匹配
> Capability status: Verified（仅[本次验证](../validation/2026-11-06/release-0.0.3/README.md)列出的命令与 Linux / Rust 1.94 范围）
> Last validated: 2026-11-06；本地日志，远程 CI NotRun

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

## 网络入口 Module

Module 可用 `#[async_trait] impl Module` 覆盖默认无操作的 `activate_ingress(&mut self, &RuntimeContext)`；既有不覆盖 Hook 的 impl 无需修改。所有组件 ready 通过后才激活，全部激活成功后 Application 才报告 Ready。setup 可 `TcpSocket::bind`，但 `listen` / `TcpListener::bind` 必须放到激活阶段；只推迟 accept 仍会允许 TCP 握手。

长期 Listener、Connection 使用 `ctx.tasks.spawn`，明确最大连接数和宿主 task_limits；监听循环优先处理取消，再接受连接。外部请求使用 `ctx.event_hub.external()`，避免沿用 Listener 的因果准入。begin_shutdown 停止接单，quiesce 等待生产者退出，工作与租约排空后 teardown；Hook 失败和 panic 均须支持部分激活补偿。

可运行 TCP 示例为 [graceful-http](../../examples/graceful-http/src/main.rs)：HTTP 与 health 均为普通 Module，共享应用 Hub；执行 `cargo run -p graceful-http --locked -- --once` 自检。示例串行处理连接且有 I/O 等待上限，不是 MQTT Broker 或高并发服务器。

重建先完成受影响闭包的 ready，再激活该闭包；激活失败按累计预算补偿后重试。旧 TaskScheduler / 服务能力失效，应用级 EventHub/ExternalEventHub 仍有效；退出的监听任务与 socket 所有权保障旧入口不接单。契约见 LIFE-INGRESS-001—005，真实 TCP 回归见 [ingress.rs](../../crates/modorch/tests/ingress.rs)。

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
