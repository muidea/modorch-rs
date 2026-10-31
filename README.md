# Modorch-RS

Modorch 是供独立 Rust 应用通过 Cargo 引入的模块化编排框架。应用显式注册和选择 Initiator / Module，框架管理依赖、消息、后台任务和检查式停机。每个 Application 独立，宿主负责 Tokio Runtime、信号和日志 Subscriber。

当前版本为 `0.1.0`，支持按依赖关系停止组件、受预算约束的重启、动态 Lane 回收和显式限时等待入队。目标设计见 [设计说明书](docs/Modorch-RS-Framework-Design.md)，逐项实现及验收对应关系见 [实现验收表](docs/implementation.md)。

```toml
[dependencies]
modorch = { path = "../modorch-rs/crates/modorch" }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

```rust
use modorch::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let handle = ApplicationBuilder::new("my-app").build()?.start().await?;
    // Register factories and select components before build; default selection is empty.
    handle.shutdown_checked(ShutdownOptions::graceful()).await?;
    Ok(())
}
```

可运行示例：

```bash
cargo run -p minimal-app
cargo run -p stateful-initiator
cargo run -p multi-module
cargo run -p graceful-http -- --once
```

- [共享状态](examples/stateful-initiator/src/main.rs)：Initiator 发布窄接口，Module 按声明依赖获取 ServiceHandle，使用期间持有 ServiceLease。
- [模块协作](examples/multi-module/src/main.rs)：私有存储状态、版本化契约、类型化 Request 和内部子请求。
- [HTTP 入口](examples/graceful-http/src/main.rs)：Ready 后激活监听、Owner 任务、宿主信号及排空。省略 `--once` 后等待 Ctrl-C。
- [混合负载验收](examples/mixed-load/README.md)：每 Owner 四类工作、资源压力、突发后空闲、Lane 回收和故障恢复循环；支持配额配置及 SLO 检查。
- [独立工作区](tests/fixtures/external-app)：通过路径依赖接入，可单独运行，无框架到应用的反向依赖。

`command` 回执只代表入队；`request` 等待业务完成。已执行请求超时返回 `Indeterminate`。`StopIncomplete` 保留未完成工作与依赖资源，后续可重试关闭。强制关闭返回独立回执，不承诺外部副作用已停止。

混合负载可用 `TaskScheduler::spawn_cpu / spawn_blocking_io` 分别提交到有界执行池；`ApplicationBuilder::execution_resources / message_limits` 配置执行资源、Owner 额度和消息内存预算，详见 [公共 API](docs/public-api.md)。

诊断入口：`ApplicationHandle::metrics()` 汇总消息路由、按 Topic/组件统计、队列/任务和 Drain 耗时；错误上下文及观测边界见 [公共 API](docs/public-api.md)。

负载与稳定性验证入口：`cargo run --release -p runtime-load -- --mode dynamic --seconds 300 --producers 64 --capacity 8`。实际结果与复现方法见 [验证报告](docs/validation.md)。

混合场景可执行 `cargo run --release -p mixed-load -- --profile examples/mixed-load/profiles/local.toml`。批量 A/B、CPU/RSS 采样、DHAT 和恢复循环由 `scripts/acceptance.py` 驱动；测量前完成 release 构建，输出目录保留源码与二进制摘要。部署模板见 [deployment.template.toml](examples/mixed-load/profiles/deployment.template.toml)，阈值需按目标环境填写。

开发检查：

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features --locked
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test -p modorch-eventhub --test concurrency -- --nocapture
cargo run --manifest-path tests/fixtures/external-app/Cargo.toml --locked
```

Rust Edition 2024，最低版本及验证记录见 [兼容性说明](docs/compatibility.md)。许可证为 [MIT](LICENSE)。
