# 0.0.1 干净检出 CI 修复与发布证据

GitHub 预发布 v0.0.1 保留 Candidate。首次远程 Skills CI 发现测试依赖已存在的 target/；已在创建临时证据前显式初始化该构建目录。仅修改工具测试与手动发布门禁的证据入口，框架源码、版本、Skill Bundle 与公共行为均未改变。

- [初次 CI 失败](initial-ci-failure.log)保留原始输出，不覆盖此前记录。
- 从 git archive 创建干净源码副本，应用本次修复，确认 target/ 不存在后执行全部 14 项工具测试，结果 [Passed](clean-checkout-tooling.log)。
- [最新聚合报告](verification.json)复用[先前完整验收](../README.md)的 Rust/模板/分发等日志；框架源码指纹完全相同。替换工具测试证据，并重新执行最终静态门禁，不重复无关 Rust 验证。
- [发布门禁](release-readiness.json)仅缺 11 项实际独立 Agent P0 证据，继续 Candidate；独立跨宿主比较同为 NotRun。GitHub 预发布不声明 Documentation & Skills Release Ready。

最终附件为原始 modorch-skills-1.0.3.tar.gz、此目录的 verification.json / release-readiness.json / SHA256SUMS。源代码与运行条件见 [0.0.1 发布说明](../../../../releases/0.0.1.md)。远程 Actions 结果以对应提交的 GitHub 运行记录为准。
