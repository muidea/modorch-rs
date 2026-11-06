# 运行时与关闭事务

适用 0.0.3 当前实现；架构描述不扩大公共保证。

Builder 冻结验证后的描述符和工厂，start 拥有创建与补偿事务。正常关闭与依赖闭包恢复共享事务锁；超时只限制调用者，checkpoint 防止重复 teardown，代际防止旧能力复用。源码 crates/modorch-runtime/src/lib.rs 与 supervision.rs。

设计原因见 [ADR-001](adr/001-documentation-source-of-truth.md)，可观察约束见 [Contracts](../../contracts/README.md)，实际验证见[报告](../validation/2026-11-06/release-0.0.3/README.md)。

未来跨进程 Transport / 持久化 / 插件 ABI 属于 Unsupported，不通过架构图暗示已实现。

入口激活通过 Instance 统一转发到 Initiator / Module 的可选 activate_ingress。初次启动为全体 ready → 开根准入 → 全体激活 → Ready；监督重建对受影响依赖闭包执行相同时序。重建、开准入与激活构成单次尝试，激活错误或 panic 也进入补偿；任务/租约排空与 teardown 成功后方可推进下一代，所有激活成功后统一标记组件 Ready。

TCP 入口在 setup 最多 bind 未 listen 的 TcpSocket，激活时才 listen；框架无法拦截任意用户网络调用。长期任务必须受管，外部消息使用 external() 新根边界。代际校验适用于任务/服务能力，应用级 Hub 句柄不绑定生产者代际。Hook 必须及时返回，沿用现有生命周期预算限制，不通过丢弃 Hook Future 强行超时。
