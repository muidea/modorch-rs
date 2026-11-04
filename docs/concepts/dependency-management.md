# 依赖与启用闭包

适用 0.0.2；规范见 [Dependency Contract](../../contracts/dependency-contract.md)。

requires 表示必须存在且启用；optional 表示已启用时参与顺序，不隐式启用。默认 RequireSelected；需要框架显式补齐 requires 时设置 IncludeRequired。未知组件不能被自动创造。Initiator 的 requires 与 optional 都不能指向 Module。

build 校验并冻结启动计划；同层按 DAG 确定顺序，停机反向。存在资源借用就必须声明依赖，消息路由本身不自动建立生命周期依赖。重复身份以 kind + id 判定，两种角色同名是不同身份。

不要凭配置 enabled 推断编译 Feature 已提供 Factory。非法容量、描述符变化、依赖环和缺失配置都须在创建有副作用资源之前处理。完整测试位于 [registry.rs](../../crates/modorch-registry/tests/registry.rs)。
