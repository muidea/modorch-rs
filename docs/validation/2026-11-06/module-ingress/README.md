# Module 入口增强验收（2026-11-06）

本轮设计刷新与代码收口完成，完整本地验证 Passed；发布状态为 Candidate。框架为 **0.0.2 开发树**，Contract **1.2.0**，Skill **1.0.6**。已发布 v0.0.2 工件不包含本增强，不覆盖旧 tag 或 Bundle。

## 范围与实现

先按评审刷新 [设计](../../../modorch_rs_mqtt_broker_prerequisites_design.md)，再完成 Module 默认 activate_ingress、Instance 统一分发及启动/监督恢复接线。重建中的入口激活失败进入同一次尝试的补偿路径，排空并拆卸完成后才按累计预算重试；全部激活成功后才标记受影响组件 Ready。

TCP 屏障采用 listen 前不可连接的语义：setup 可 bind 未 listen 的 TcpSocket，激活才 listen。HTTP 示例改为普通 Module，显式管理监听任务与未激活 socket 的清理。旧任务/服务能力受代际约束，应用级 EventHub/ExternalEventHub 保持有效；不新增 MQTT Broker、QoS、Session 或第二 Hub。

## 实际验证

| 检查 | 状态 | 证据 |
|---|---|---|
| fmt / check / clippy -D warnings | Passed | [fmt](checks/fmt.log)、[check](checks/check.log)、[Clippy](clippy-initial.log) |
| workspace test | Passed，118 个 Rust 测试，包含 10 个新增入口回归 | [test](checks/test.log) |
| doctest / rustdoc -D warnings | Passed，8 个 doctest | [doctest](checks/doctest.log)、[rustdoc](checks/rustdoc.log) |
| MSRV 1.85.0 / Feature 组合 | Passed（编译检查） | [MSRV](checks/msrv.log)、[Features](checks/features.log) |
| 10 个示例、2 个模板、独立消费者 | Passed；HTTP --once 完成真实请求 | [示例](checks/examples.log)、[模板](checks/templates.log)、[外部应用](checks/external.log) |
| Python 工具测试 | Passed，15 个测试；含篡改、穿越、冲突、幂等与漂移检测 | [tooling](checks/tooling-tests.log) |
| 最终 Skill 干净目录安装/构建/运行 | Passed，51 步确定性回放 | [最终回放](final-fixtures/summary.json)、[Bundle 安装](final-fixtures/bundle.json) |
| 文档/Contract 映射/能力矩阵/快照 | Passed，57 个真实测试映射 | [最终检查](static-final.log)、[影响分析](affected-final.json) |
| Skill 预检/托管归档 | Passed；三份 Skill 1.0.6 与本机归档 synced | [preflight](preflight-final.json)、[归档](skill-feedback.json)、[状态](skill-hub-status.json) |
| 独立 Agent / 跨平台 / 高连接数性能 | NotRun | [Agent 计划](agent-plan/summary.json)、[发布门禁](release-readiness.json) |

工具版本见 [versions](checks/versions.log)，机器可读命令、退出码与日志摘要见 [verification.json](verification.json)，Skill 输出契约见 [result.json](result.json)。最终框架指纹：`dd4b8349a1e726db00584d0f6e7cb9c857033d58dc19ba8c8235e337fd810e9f`。

LIFE-INGRESS-001—005 的 Rule → Test 映射在 [mapping.json](../../../../tests/contracts/mapping.json)。真实 TCP 测试覆盖两种角色的 ready 屏障、build 零副作用、激活错误/panic/真实端口占用/任务配额补偿、取消启动等待、慢连接 StopIncomplete、连接任务限额、同 Hub 互通/跨应用隔离、重建激活失败的排空后重试与预算耗尽。既有 supervision 测试继续验证旧服务能力失效及旧任务实际退出。

## 原始失败与复验

首次编译失败和首轮测试夹具错误分别保留于 [initial-compile.log](initial-compile.log) 与 [ingress-first-run.log](ingress-first-run.log)：修复测试辅助方法同名解析、Debug 约束、订阅 ID 重复和背压错误断言。修复后的 8 项及增强后的 9 项检查记录于 ingress-second-run.log / ingress-enhanced.log；最终完整 workspace 日志包含全部 10 个入口测试通过。历史失败不覆盖，不计为最终 Passed。

初次分发回放在 fixtures/，最终文档与快照收口后另在 final-fixtures/ 重验，均保留原始输出。[本地检查脚本](run-local-checks.py)记录执行过程；Clippy 在最终 Rust 代码上已执行通过，后续仅改文档/快照，复用其原始日志。最终验证报告使用最新 static / Bundle / Skill 日志，早期 checks/command-results.json 保留中间状态。

## 边界与发布状态

实测平台 Linux x86_64 / Rust 1.94，MSRV 1.85 为 check，不推定其他平台行为。TCP 屏障依赖组件遵守 Hook 契约，框架不拦截任意用户网络调用；Hook 须及时返回，不新增强制 timeout-drop。连接数、任务配额与 MQTT 交付由组件和宿主明确配置，不作未测量的吞吐或内存承诺。

发布门禁仍有 11 个独立 Agent P0 证据缺口，fixture 回放不替代语义 Agent 验收。本轮没有提交、推送或发布，Skill 仅同步至本机 skill-hub 归档。
