# 从零运行 Modorch 应用

> Applies to: modorch 0.0.1 当前开发树；Rust >= 1.85，Edition 2024
> Capability status: Verified；2026-11-02，Linux x86_64 / Rust 1.94；其他平台 NotRun

## Goal

从匹配源码创建独立应用、显式启用一个 Module、检查式关闭。当前没有 Registry 发布声明，不使用未经发布验证的 crates.io 或 docs.rs 安装命令。

## Prerequisites

安装 Rust 1.85 或更高版本，准备 Python 3.11+（模板工具）和当前完整源码。首次构建需依赖缓存或网络；命令不会安装系统工具。先运行 rustc --version / cargo --version。

## When to use / When not to use

应用拥有 main、Tokio、信号、配置及 Subscriber；框架负责组件编排。单次普通函数不必使用框架组件。

## Relevant contracts

[COMP-002](../contracts/component-contract.md)、[LIFE-001](../contracts/lifecycle-contract.md)、[COMPAT-001](../contracts/compatibility-contract.md)。

## Steps

在仓库根目录运行，目标目录必须不存在：

```bash
python3 tools/skill-packaging/init-template.py --template minimal-app --dest /tmp/modorch-hello --framework-path "$PWD/crates/modorch"
cargo check --manifest-path /tmp/modorch-hello/Cargo.toml --locked --offline
cargo test --manifest-path /tmp/modorch-hello/Cargo.toml --locked --offline
cargo run --manifest-path /tmp/modorch-hello/Cargo.toml --locked --offline
```

模板携带 Cargo.lock；没有缓存时按宿主联网策略先执行 cargo fetch --locked --manifest-path /tmp/modorch-hello/Cargo.toml，保留已有锁定版本。初始化工具只创建指定新目录，依赖显式指向经过核对的 Facade 路径；不会修改现有工程或用户全局目录。Git 接入须固定已验证 tag/commit，并重新验证 Skill 指纹。不要只拷贝根 Workspace 内依赖 workspace=true 的示例 manifest 到外部。

## Complete runnable example

[最小模板](../templates/minimal-app/src/main.rs) 实现 Factory、Module、Lifecycle；[basic-app](../examples/basic-app/src/main.rs) 为工作区版本。应用注册后还要显式选择启用组件，默认选择为空。

## Expected observable behavior

运行打印 basic-app: stopped，退出码为 0。组件 setup/start/ready 后正常关闭；模板内 smoke test 验证选中 Module 参与生命周期。

## Failure cases and limitations

找不到 modorch / 未锁定依赖 / 离线缺缓存会非零退出；记录原始错误，不能猜测 API。build 的 MissingDependency / DisabledDependency 在创建资源前处理。start 返回 StartupError 时检查 source、cleanup 和 handle；停机超时不能当 Stopped。模板不读取生产秘密、不隐式监听公网。

## Validation commands

上面的 check/test/run 是干净目录 smoke；框架本身使用 CONTRIBUTING 中完整门禁。完整日志见[本次验证](validation/2026-11-02/release-0.0.1/README.md)。

## Related API / Examples / Tests

[应用指南](guides/application.md)、[组件模型](concepts/component-model.md)、[API 导航](public-api.md)、[独立消费者](../tests/fixtures/external-app)。
