# 0.0.2 GitHub 预发布验收

框架 0.0.2 / Skill revision 1.0.5 / Contract revision 1.1.0。已完成 MQTT Topic 对齐和版本/Skill 分发收口，并在版本调整后重新执行本地验收。第三方 Registry 依赖版本与校验和保持，见 [versions.json](versions.json)。

| 检查 | 实测结果 | 证据 |
|---|---|---|
| fmt / check / clippy | Passed；全工作区全目标全 Feature、locked，Clippy -D warnings | [fmt](checks-final/fmt.log)、[check](checks-final/check.log)、[clippy](checks-final/clippy.log) |
| Rust / doctest | 108 项普通测试 + 8 项 doctest Passed；独立 doctest 命令再次通过同 8 项 | [workspace](checks-final/test.log)、[doctest](checks-final/doctest.log) |
| Rustdoc / MSRV | Rustdoc -D warnings，Rust 1.85.0 全工作区全目标全 Feature check Passed | [Rustdoc](checks-final/rustdoc.log)、[MSRV](checks-final/msrv.log) |
| Feature / 示例 / 模板 / 外部应用 | 6 组单独 Feature、6 个标准示例、2 个模板及外部应用 Passed | [Feature](checks-final/features.log)、[示例](checks-final/examples.log)、[模板](checks-final/templates.log)、[外部应用](checks-final/external.log) |
| 工具与文档 | 15 项 Python 测试、47 项 Contract 映射、22 项能力矩阵、链接/Skill/快照检查 Passed | [Python](checks-final/tooling-tests.log)、[静态门禁](checks-final/static.log) |
| 干净目录回放 | 51 步 Passed；含 0.0.2 模板、Bundle 安装/幂等与独立应用构建/测试/运行，mode=fixture | [摘要](fixtures/summary.json)、[Bundle](fixtures/bundle.json) |
| skill-hub | 三份 1.0.5 均 validate、feedback dry-run、feedback 本地归档，最终 synced | [状态](skill-hub-status.json)、[校验](skill-validate.log)、[归档](skill-feedback.log) |

执行命令、环境、退出码、日志 SHA-256 与框架/交付指纹见[verification.json](checks-final/verification.json)。平台 Linux x86_64 / Rust 1.94.0 / Python 3.12.3；其他平台未由本机结果推定。远程 CI 以本次 push 对应 GitHub Actions 结果为准。

[发布门禁](checks-final/release-readiness.json)维持 **Candidate**，阻断项为缺少 11 项 P0 独立 Agent 执行/复核证据；[12 项实际 Agent 用例](agents/summary.json)全部 NotRun，fixture 单独统计。用户选择的发布方式为 GitHub 预发布，未标 Release Ready，未执行 cargo publish 或 Skill 仓库远程 push。

[发布附件](artifacts-final/SHA256SUMS)包含 Candidate Skill 1.0.5 Bundle、验证报告及 Candidate 状态报告。框架/源码由 v0.0.2 tag 对应，实际提交 ID 可从 tag 解析；版本/文档改变后未复用旧框架指纹的通过状态。

迁移及已知限制见[发布说明](../../../releases/0.0.2.md)。`:id` → `+`，空层/零层、`$`、非法输入和本地预算错误均有破坏性影响。原[MQTT 语义与性能测量](../mqtt-topic-alignment/README.md)保留在版本调整前的独立目录；严格校验增加部分查询开销，无统一提速或生产 SLO 承诺。Topic 对齐不提供 MQTT 网络/Broker/QoS/Session/Retained/共享消费组。

旧 v0.0.1 tag、发布资产和历史 validation 目录保持。当前用户指令授权将全部本轮修改提交、推送并发布新版本。

发布准备期间 GitHub OAuth 凭据缺少 workflow scope，因此保留原有 CI 工作流配置。最终完整验收重跑在 checks-final，发布附件使用 artifacts-final；checks 与 artifacts 保留先前含报告路径调整时的原始记录，不作为当前发布指纹。未修改账号权限或全局 Git 配置。
