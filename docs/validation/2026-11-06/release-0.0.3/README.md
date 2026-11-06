# Modorch-RS 0.0.3 Candidate 发布验收（2026-11-06）

本次本地发布验收全部 Passed；发布状态保持 Candidate。八个框架 crate、示例、两个独立模板与外部消费者统一 **0.0.3**，Contract **1.2.0**、Skill **1.0.7**。发布范围为 GitHub 源码预发布与 Candidate Skill Bundle，不执行 cargo publish，不声明 crates.io / docs.rs 发布。旧 tag / 工件和历史 validation 记录不覆盖。

本轮包含 [Module 入口增强](../../../modorch_rs_mqtt_broker_prerequisites_design.md)：默认 activate_ingress、统一分发、全体相关 ready 后激活、重建激活失败的排空补偿和累计预算重试。TCP setup 只预绑定未 listen 的 socket；任务/服务能力受代际约束，应用级 Hub 保持有效。迁移见[说明](../../../migration/0.1-development.md)，公开变化见[发布说明](../../../releases/0.0.3.md)。此前 0.0.2 开发树的[入口验收](../module-ingress/README.md)保留原状态。

## 实际执行

| 检查 | 结果 | 证据 |
|---|---|---|
| fmt / check / clippy -D warnings | Passed | [fmt](checks/fmt.log)、[check](checks/check.log)、[clippy](checks/clippy.log) |
| workspace test / doctest | Passed：118 个 Rust 测试、8 个 doctest | [test](checks/test.log)、[doctest](checks/doctest.log) |
| Rustdoc -D warnings / MSRV 1.85.0 / Feature 组合 | Passed，MSRV 为编译检查 | [rustdoc](checks/rustdoc.log)、[MSRV](checks/msrv.log)、[Features](checks/features.log) |
| 10 个示例、2 个模板、外部消费者 | Passed；HTTP --once 完成真实 TCP 请求 | [示例](checks/examples.log)、[模板](checks/templates.log)、[外部](checks/external.log) |
| Python 工具测试 | Passed，15 个；含安全、篡改、幂等、漂移与门禁负例 | [tooling](checks/tooling-tests.log) |
| 最终 Bundle 干净目录安装/构建/运行 | Passed，51 步确定性回放 | [回放](final-fixtures/summary.json)、[工件](final-fixtures/bundle.json) |
| 文档/Contract/能力/Skill 快照 | Passed，57 个真实测试映射 | [static](checks/static.log)、[影响](affected-final.json) |
| skill-hub 托管归档 | 三份 Skill 1.0.7 synced | [归档](skill-feedback.json)、[状态](skill-hub-status.json) |
| 独立 Agent / 跨平台 / 生产性能 | NotRun | [计划](agent-plan/summary.json)、[发布门禁](release-readiness.json) |

原始命令、版本与退出码见 [verification.json](verification.json)、[versions](checks/versions.log)和[执行脚本](run-local-checks.py)。最终源码指纹：`3c3ddca614e910cef42cd34800364e2600609821f259c22f3a73f464f49e1971`。本地平台 Linux x86_64 / Rust 1.94；不由 MSRV check 推定其他平台行为。

## 工件与状态边界

发布附件为 modorch-skills-1.0.7.tar.gz、verification.json、release-readiness.json 与 SHA256SUMS。Bundle 绑定 0.0.3 的精确源码指纹，旧 Bundle 不因同名 API 自动兼容。最初 fixtures/ 保留版本准备期间的分发验证；最后修正 Contract 的旧 tag / Skill 版本说明后，另在 final-fixtures/ 完整复验，发布使用后者的归档，不覆盖早期证据。

setup-notes.md 记录已修正的版本准备链接/空格问题，未当作行为测试 Passed。完整本地门禁通过与 Release Ready 是不同结论：11 个独立 Agent P0 验收证据仍 NotRun，release-gate 按设计返回非零并保持 Candidate，不使用 fixture 替代 Agent。

本地报告生成时远程 CI 尚未启动；发布流程另核对待发布提交的 GitHub Actions 结论，并在 GitHub Release 中链接实际运行。Skill 仅 feedback 至本机 skill-hub 归档，不自动发布独立 Skill 仓库。
