# MQTT Topic 对齐验收（2026-11-04）

已完成[实施规范](../../../modorch_rs_mqtt_topic_alignment_design.md)对应的代码、契约、迁移、Skill 和本地验证。本轮未提交开发树，以基线 `ccc77ce9501cd3e942677bba51b6ff3efb1c63f6` 为对照；旧 tag、发布工件与历史 validation 均保留。

## 实现与兼容

- 新增内部 topic.rs：严格 Topic Name 校验、TopicFilter 编译与唯一通配 matcher；公开 Topic/String 字段保持。helper、RouteIndex 和真实 Hub 共用语义，`:id` 使用精确索引。
- `+` 接受空层，末尾 `#` 接受零层，根通配排除 `$`；不 trim、不归一化。Name / Filter 非空、无 NUL、≤65535 UTF-8 字节；Name 禁止 +/#。
- 普通 / external / wait 入站统一校验；初始 Handler 安装和动态 subscribe 均先校验/编译再修改索引，失败保留 registry/generation。
- 协议非法 InvalidConfig；合法 Name/Filter 超过本地 Topic 预算 ResourceLimit（默认 1024 字节）。Destination、Lane、组件去重、背压、WaitGraph 与请求超时合同保持。
- 破坏性迁移：`orders/:id/changed` → `orders/+/changed`，审计空层/零层带来的 Handler 与 ACL 匹配集合变化。详见[迁移记录](../../../migration/0.1-development.md)。
- Contract revision **1.1.0**、Skill revision **1.0.4**；生成 66 项便携资源，三份 Skill 已经 skill-hub validate → feedback dry-run → feedback 本地归档，最终均 synced，见[管理状态](skill-hub-status.json)。

## 实际验证

平台 Linux x86_64，Rust/Cargo 1.94.0、Python 3.12.3；MSRV 1.85.0 通过全工作区全目标全 Feature 编译。原始命令、环境、退出码、日志 SHA-256 见[完整清单](checks/verification.json)。

| 检查 | 实测结果 | 证据 |
|---|---|---|
| fmt / check / clippy | Passed；workspace/all-targets/all-features/locked，Clippy -D warnings | [fmt](checks/fmt.log)、[check](checks/check.log)、[clippy](checks/clippy.log) |
| Rust workspace | **108** 项普通测试、**8** 项 doctest Passed | [test](checks/test.log)、[独立 doctest](checks/doctest.log) |
| MQTT conformance | **39** 组独立向量（包含 MR-T01—T20）三路径一致；6 项集成测试 + 1 项索引单测 Passed | [topic_conformance](../../../../crates/modorch-eventhub/tests/topic_conformance.rs)、[workspace 日志](checks/test.log) |
| 边界/并发 | ASCII/多字节 65535/65536 边界、默认/自定义本地预算、12 类入站、订阅错误无副作用、组件去重及索引增删竞争 Passed | [workspace 日志](checks/test.log) |
| Rustdoc / MSRV | 警告作为错误的 Rustdoc 与 Rust 1.85.0 check Passed | [Rustdoc](checks/rustdoc.log)、[MSRV](checks/msrv.log) |
| Feature / 示例 / 模板 / 外部应用 | 6 个单独 Feature、6 个标准示例、2 个模板、外部应用 Passed | [Feature](checks/features.log)、[示例](checks/examples.log)、[模板](checks/templates.log)、[外部应用](checks/external.log) |
| Python / 文档 | **15** 项 Python 工具测试、47 个 Contract 映射、22 项能力矩阵、链接/Skill/快照漂移 Passed | [Python](checks/tooling-tests.log)、[静态门禁](checks/static.log) |
| Bundle 干净目录回放 | **51** 步 Passed，mode=fixture；含安装幂等、独立应用构建/测试/运行和工具代理 | [fixture 摘要](fixtures/summary.json)、[Bundle](fixtures/bundle.json) |

`cargo test --workspace` 本身也运行这 8 项 doctest；单独 doctest 命令再次验证相同用例，统计不重复相加。Topic 边界、所有入站与三路径匹配全部有真实执行日志。`tooling_tests` 新增独立 Python 映射及 --tooling-results 校验，缺通过行不能报告执行 Passed。

框架指纹：`f2d460b6daf7da5b55254380e1375a28e4119ebfd08d5bf51ce6f6a3821f87e7`。交付指纹及每项检查日志哈希见 verification.json；新增性能来源与测量文件哈希见[测量索引](measurement-index.json)。

## 性能与内存测量

[路由测量脚本](route-measurement.py)直接包含基线/当前 routing.rs，使用最小 DTO/error stub，覆盖 64/4096 订阅、精确/通配、广播/定向。每场景交替执行 5 次，保留[首轮原始结果](route-performance/results.json)。初轮定向样本波动较大，因此另用[固定 CPU 且增加迭代的脚本](route-measurement-pinned.py)补测，原始结果见[补测数据](route-performance-pinned/results.json)。本次补测中位数：

| 订阅数 | Filter | Destination | 基线 µs/lookup | 当前 µs/lookup | 耗时变化 |
|---|---|---|---:|---:|---:|
| 64 | exact | broadcast | 0.303 | 0.334 | +10.4% |
| 64 | exact | directed | 0.275 | 0.316 | +15.0% |
| 64 | wildcard | broadcast | 9.440 | 17.868 | +89.3% |
| 64 | wildcard | directed | 0.368 | 0.444 | +20.6% |
| 4096 | exact | broadcast | 0.331 | 0.386 | +16.5% |
| 4096 | exact | directed | 0.256 | 0.294 | +15.1% |
| 4096 | wildcard | broadcast | 1138.956 | 1050.831 | -7.7% |
| 4096 | wildcard | directed | 0.523 | 0.653 | +24.9% |

性能结论：严格校验增加定向查询成本，补测约 +15%—25%；广播结果受规模和主机负载影响，4096 通配广播本轮约 -7.7%，64 通配广播约 +89%。没有统一提速结论。需要更严的延迟门槛时，应在目标宿主上隔离负载、延长测量并按实际订阅树建立 SLO。索引 RSS 增量与基线接近（4096 通配约 3.6—3.8 MiB）；RSS 受分页/allocator 影响，不能作为精确堆占用。

[混合负载脚本](mixed-measurement.py)运行真实 release mixed-load，覆盖 async、CPU、同步阻塞 I/O 和控制消息；两个 Owner，每 Owner 1024 订阅，Concurrent / Dynamic 各 3×2 秒，每轮恢复一个组件，空闲窗口 200ms。本轮配置的空闲 RSS 增长门槛为 8192 KiB：

| 策略 | 成功操作/活跃秒 | 空闲 RSS 增长 KiB | 恢复次数 | 结果 |
|---|---:|---:|---:|---|
| Concurrent | 45827 | 1920 | 3 | Passed |
| Dynamic | 52951 | 1248 | 3 | Passed |

每轮排空后在途操作、声明 payload 字节、等待者、任务和构造中 Lane 均归零；Dynamic 的活动 Lane/Handler/Worker 控制也归零，资源额度归还，最终均 GracefullyStopped。原始输出及日志哈希见[混合负载摘要](mixed-performance/summary.json)。本地短时测量量化回归成本和资源回收；生产容量、长期泄漏及延迟 SLO 尚需具体部署验证。

## 发布与未覆盖范围

[发布门禁](checks/release-readiness.json)保持 **Candidate**，阻断项仅为缺少 11 项 P0 的独立 Agent 执行/复核证据；[12 个 Agent 用例](agents/summary.json)全部 NotRun。确定性 fixture 已通过，独立 Agent 评估仍单独记录。本轮没有启动额外 Agent，也没有提交、推送或发布。

其他平台、Rust 1.99、远程 CI、远程 Markdown 链接检查为 NotRun。Topic 对齐不提供 MQTT Broker/网络、QoS、Session、Retained 或共享消费组。交付结果见[result.json](result.json)和[报告格式检查](result-validation.log)。

MR-01—06 对应：设计/迁移与基线来源 → topic.rs → routing.rs → limits/lib/workers → Contracts/mapping/指南/Skill → 上述 conformance、并发、性能、workspace 和分发日志。源码/框架未覆盖项均在本节限定；不将 Candidate 标为 Release Ready。
