# 0.0.1 发布验证

本地强制检查全部 Passed。发布目标为 GitHub **v0.0.1 预发布（Candidate）**，由用户明确选择；不执行 cargo publish。框架 0.0.1、Skill revision 1.0.3、Contract revision 1.0.0。

[完整命令与日志摘要](verification.json)保存框架和交付指纹，各项原始输出位于 checks/；[发布门禁](release-readiness.json)继续保持 Candidate，仅缺 11 项 P0 独立 Agent 执行/复核证据。十二例实际 Agent 和跨宿主验收均 NotRun，确定性 fixture 不能替代。GitHub 预发布不声明 Documentation & Skills Release Ready。

- fmt、全目标全 Feature check、严格 Clippy、workspace test、独立 doctest、warnings-as-errors Rustdoc、MSRV 1.85、六种 Feature 组合均 Passed。
- workspace 共 109 项 Rust 测试通过（101 项普通测试、8 项 doctest）；另执行独立 doctest，8 项通过；Python 工具测试 14 项通过。
- 六个标准示例直接运行、两个独立模板 fmt/check/test/run、外部消费者均 Passed。
- [干净目录回放](fixtures/summary.json)共 51 步 Passed，覆盖安装/重装、模板与行为运行，以及兼容/安全/门禁基础设施负面检查。
- 37 个 Rust Contract 映射均有实际 Passed 行，21 项能力、三份 Skill 与生成快照通过一致性检查。
- [Skill Bundle](fixtures/modorch-skills-1.0.3.tar.gz)与当前源码负载完全一致，摘要见 [bundle-integrity.json](bundle-integrity.json)。校验、dry-run、feedback 后，[skill-hub 状态](skill-hub-status.json)为 synced 1.0.3。

工具链为本地 Linux x86_64 / Rust 1.94.0；MSRV 1.85 仅执行 check。独立 Agent、其他 OS 和 Rust 1.99 未由本次本地结果推定通过；远程 CI 在推送后另核对 GitHub Actions。此前 0.1.0 开发树的[验收记录](../documentation-skills/README.md)与原始日志保留，不改写为 0.0.1 的执行结果。第三方依赖沿用原锁定版本。

GitHub 预发布附带 Bundle、SHA256SUMS、本页对应的 verification.json 与 release-readiness.json；下载校验和采用文件名，不需要开发机绝对路径。仅用户授权的项目 master 与 v0.0.1 标签推送至 GitHub，不推送本地 skill-hub 仓库。
