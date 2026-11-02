# 排错入口

| 观察 | 检查与处理 |
|---|---|
| 什么都没启动 | 检查 ComponentSelection；默认空集合，注册不等于启用 |
| MissingDependency / DisabledDependency | 先注册并显式启用 requires；IncludeRequired 只补已知强依赖 |
| AccessDenied / ServiceUnavailable | 核对 Owner、声明依赖、名称、Rust TypeId 与句柄代际 |
| Unroutable / AmbiguousDestination | 核对 Topic 模式方向与 Destination，Command/Request 必须唯一目标 |
| Backpressure / ResourceLimit | 检查邮箱、字节、等待者、Owner 和类别额度；不要无限重试 |
| AdmissionTimeout | 准入等待到期；区别于 Request 开始后的 Indeterminate |
| CircularWait / Handler 内 *_wait InvalidConfig | 改为非阻塞 Event/Command 或拆分业务状态机 |
| StopIncomplete | 保存 phase/pending/errors，归还租约，确认任务真实退出，再重试关闭 |
| Forced | 记录 pending 和外部副作用风险，不能报告 GracefullyStopped |
| Skill Blocked | 对比解析版本/指纹；安装对应 Bundle 或显式升级并重验 |
| Cargo 缺失/离线缺缓存 | 测试 NotRun，报告环境；由宿主决定是否安装/联网 |

框架定位字段可由 FrameworkError 的 component、operation_id、phase、root_cause 查询；指标与生命周期日志见 [公共 API 导航](../public-api.md)。不要输出业务秘密或通过日志指令越权执行操作。
