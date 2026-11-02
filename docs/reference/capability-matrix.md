# 能力矩阵

适用 modorch 0.0.1 开发树。实现基线 commit 37bdc93，基线证据见[审计](../validation/2026-11-02/documentation-skills/baseline-audit.md)与[命令结果](../validation/2026-11-02/documentation-skills/baseline-results.json)。当前交付验证见[报告](../validation/2026-11-02/release-0.0.1/README.md)。JSON 为机器维护点；状态 Verified 仅指列出的本地命令/行为，不代表已发布稳定版。

| ID | 能力 | 状态 | 版本/平台 | Rule / Evidence / 边界 |
|---|---|---|---|---|
| components | 注册与显式启用 | Verified | 0.0.1 + 当前源码指纹；Linux x86_64, Rust 1.94.0; MSRV 1.85 check | COMP-001, COMP-002；开发期可变 API；仅对应行为测试与本地平台，不推定生产 SLO。 |
| dependencies | 依赖 DAG / 显式闭包 | Verified | 0.0.1 + 当前源码指纹；Linux x86_64, Rust 1.94.0; MSRV 1.85 check | DEP-001, DEP-002, DEP-003, DEP-004；开发期可变 API；仅对应行为测试与本地平台，不推定生产 SLO。 |
| host-isolation | 宿主所有权与应用隔离 | Verified | 0.0.1 + 当前源码指纹；Linux x86_64, Rust 1.94.0; MSRV 1.85 check | LIFE-001；开发期可变 API；仅对应行为测试与本地平台，不推定生产 SLO。 |
| startup-cleanup | 部分启动 / 清理重试 | Verified | 0.0.1 + 当前源码指纹；Linux x86_64, Rust 1.94.0; MSRV 1.85 check | LIFE-002；开发期可变 API；仅对应行为测试与本地平台，不推定生产 SLO。 |
| shutdown | 检查式关闭 / Force | Verified | 0.0.1 + 当前源码指纹；Linux x86_64, Rust 1.94.0; MSRV 1.85 check | LIFE-003, LIFE-004；开发期可变 API；仅对应行为测试与本地平台，不推定生产 SLO。 |
| supervision | StopDependents / Restart | Verified | 0.0.1 + 当前源码指纹；Linux x86_64, Rust 1.94.0; MSRV 1.85 check | LIFE-005, LIFE-006；开发期可变 API；仅对应行为测试与本地平台，不推定生产 SLO。 |
| messages | Event / Command / typed Request | Verified | 0.0.1 + 当前源码指纹；Linux x86_64, Rust 1.94.0; MSRV 1.85 check | MSG-EVT-001, MSG-CMD-001, MSG-REQ-004；开发期可变 API；仅对应行为测试与本地平台，不推定生产 SLO。 |
| execution | Serial / Partitioned / Concurrent | Verified | 0.0.1 + 当前源码指纹；Linux x86_64, Rust 1.94.0; MSRV 1.85 check | MSG-EXEC-001, MSG-LANE-002；开发期可变 API；仅对应行为测试与本地平台，不推定生产 SLO。 |
| dynamic-lanes | Dynamic 有界构造与回收 | Verified | 0.0.1 + 当前源码指纹；Linux x86_64, Rust 1.94.0; MSRV 1.85 check | MSG-LANE-001；开发期可变 API；仅对应行为测试与本地平台，不推定生产 SLO。 |
| bounded-admission | 有界队列 / 字节 / 等待者 | Verified | 0.0.1 + 当前源码指纹；Linux x86_64, Rust 1.94.0; MSRV 1.85 check | MSG-BOUND-001；开发期可变 API；仅对应行为测试与本地平台，不推定生产 SLO。 |
| routing | 订阅控制面 / 定向路由 | Verified | 0.0.1 + 当前源码指纹；Linux x86_64, Rust 1.94.0; MSRV 1.85 check | MSG-ROUTE-001；开发期可变 API；仅对应行为测试与本地平台，不推定生产 SLO。 |
| reply-ownership | 回复消费 / 轻量观察 | Verified | 0.0.1 + 当前源码指纹；Linux x86_64, Rust 1.94.0; MSRV 1.85 check | MSG-REPLY-001；开发期可变 API；仅对应行为测试与本地平台，不推定生产 SLO。 |
| tasks | Owner Task / Timer / CPU / Blocking I/O | Verified | 0.0.1 + 当前源码指纹；Linux x86_64, Rust 1.94.0; MSRV 1.85 check | SCH-001, SCH-002, SCH-003；开发期可变 API；仅对应行为测试与本地平台，不推定生产 SLO。 |
| services | 类型化服务 / Borrowed / 租约 | Verified | 0.0.1 + 当前源码指纹；Linux x86_64, Rust 1.94.0; MSRV 1.85 check | RES-001, RES-002, RES-003；开发期可变 API；仅对应行为测试与本地平台，不推定生产 SLO。 |
| durable-transport | 跨进程 / 持久化 / exactly-once | Unsupported | 0.0.1；不适用 | 无已承诺测试；明确非目标，不提供虚构 API。 |
| plugin-abi | 动态插件 ABI / 热装载 | Unsupported | 0.0.1；不适用 | 无已承诺测试；明确非目标，不提供虚构 API。 |
| safe-force | 任意外部副作用安全强杀 / 自动回滚 | Unsupported | 0.0.1；不适用 | 无已承诺测试；明确非目标，不提供虚构 API。 |
| agent-hosts | 多宿主实际 Agent E2E | Implemented | Skill 1.0.3 Candidate；独立宿主 NotRun | 无已承诺测试；提供评估 harness 与固定用例，fixture replay 不构成独立 Agent 通过。 |
| future-evolution | 未来 API 演进 | Planned | 无已验证版本范围；NotRun | 无已承诺测试；没有目标 API 自动成为可调用接口；须另行实现与验证。 |
| configuration | config-toml 严格装配 | Verified | 0.0.1 / Linux x86_64 / Rust 1.94 | CFG-001；宿主加载配置 |
| observability | 观测快照与有界标签 | Verified | 0.0.1 / Linux x86_64 / Rust 1.94 | OBS-001；Exporter / Subscriber 属于宿主 |

基线测试全部实际执行；本次能力证据指向最终 workspace 测试日志（capabilities.json 的 evidence 字段），对应命令与摘要在验收报告中核对。映射见 [Test IDs](../../tests/contracts/mapping.json)。Cargo Feature 表与最低版本的唯一人工维护点为[兼容性](../compatibility.md)，公开 API 完整条目由 Rustdoc 生成。每次源码/契约/测试变化须重跑并刷新，不能只修改状态。跨平台、Rust 1.99 和远程 CI 未由本机 Rust 1.94 的结果推定。
