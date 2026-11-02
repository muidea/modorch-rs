# Modorch-RS

Modorch 是单进程 Rust 后端应用的模块化编排库。应用拥有 main、Tokio Runtime、配置、信号与日志；框架提供 Initiator / Module、显式装配、DAG、共享服务租约、进程内消息、受管任务和检查式停机。

当前为 **0.0.1 开发期 Candidate**，Rust >= 1.85、Edition 2024、MIT。尚未声明 crates.io / docs.rs 发布；验证范围与已知限制见[能力矩阵](docs/reference/capability-matrix.md)。

首发版本与已知限制见 [0.0.1 发布说明](docs/releases/0.0.1.md)。先前 0.1.0 是未发布的内部开发标号，现统一为 0.0.1。

从[入门指南](docs/getting-started.md)创建独立应用，或在当前源码运行：

```bash
cargo run -p basic-app --locked
cargo run -p shared-service --locked
cargo run -p request-response --locked
```

独立应用的依赖明确指向匹配版本源码：

```toml
[dependencies]
modorch = { path = "../modorch-rs/crates/modorch" }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

极简应用（完整源码也在 [minimal-app](examples/minimal-app/src/main.rs)；显式启用组件见 basic-app）：

```rust
use modorch::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let handle = ApplicationBuilder::new("my-app").build()?.start().await?;
    handle.shutdown_checked(ShutdownOptions::graceful()).await?;
    Ok(())
}
```

默认选择为空，注册不等于启用。Event/Command 成功只代表准入；运行中的 Request 超时为 Indeterminate。StopIncomplete 保留未完成工作与资源，强制结束不证明正常释放。支持 Serial、Partitioned、Concurrent、Dynamic，及受累计预算约束的 Restart；不提供跨进程、持久化消息、exactly-once 或任意外部副作用安全强杀。

[文档索引](docs/index.md) → [指南](docs/guides/application.md) → [契约](contracts/README.md) → [公共 API 导航](docs/public-api.md)。生成 Rustdoc：cargo doc --workspace --all-features --no-deps --locked。

Agent 从 [AGENTS.md](AGENTS.md) 与[两个开发 Skill](.agents/skills/modorch-app-development/SKILL.md)开始；外部项目需[独立安装匹配的 Bundle](tools/skill-packaging/README.md)，添加 Cargo 依赖不会自动安装 Skill。维护检查见 [CONTRIBUTING](CONTRIBUTING.md)，本次证据见[文档与 Skill 验收](docs/validation/2026-11-02/release-0.0.1/README.md)。[混合负载工具](examples/mixed-load/README.md)与[历史报告](docs/validation.md)保留原始性能测量。

许可证：[MIT](LICENSE)。
