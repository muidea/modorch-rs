# Changelog

## 0.0.3 Candidate — Module 入口增强（2026-11-06）

- Module 增加可选 activate_ingress，初始启动与监督重建统一入口屏障；公开 API 默认无操作，保留服务发布边界。
- 修复重建激活失败绕过补偿与重试的问题；完整激活成功后统一发布相关组件 Ready。
- TCP 预绑定示例改用未 listen 的 TcpSocket，HTTP 示例以普通 Module 实现；增加真实网络、关闭、代际和预算回归。
- Contract 1.2.0 / Skill 1.0.7 绑定新开发树指纹；迁移见 [记录](docs/migration/0.1-development.md)，证据见 [验收](docs/validation/2026-11-06/release-0.0.3/README.md)。

## 0.0.2 Candidate — MQTT Topic 对齐（2026-11-04）

- **Breaking**：Topic Filter 统一 MQTT 3.1.1/5.0；`:id` 是字面量，单层通配改用 `+`。支持空层、`#` 零层与 `$` 根通配排除。
- Topic Name / Filter 严格校验，UTF-8 协议上限 65535 字节；合法但超本地预算返回 ResourceLimit。helper、预编译索引和真实消息共用语义；不增加 MQTT 网络能力。
- Contract revision 1.1.0、Skill revision 1.0.5；迁移见 [记录](docs/migration/0.1-development.md)，验证见 [报告](docs/validation/2026-11-04/release-0.0.2/README.md)。

## 0.0.1 — 2026-11-02

首个 GitHub 源码版本。框架包与内部依赖统一为 0.0.1；先前 0.1.0 仅为未发布的开发标号。文档与 Skill 保持 Candidate，独立 Agent / 跨宿主验收尚未完成；不声明 crates.io / docs.rs 已发布。

- 按当前 Facade 与行为刷新旧设计文档，校正 Lane、Restart、调度和错误 API。
- 建立指南、Contracts、能力矩阵、文档导航、Agent Skills 与可独立验证的模板。
- 增加文档/契约/Skill 漂移检查、Bundle 完整性/安装验证和基于证据的发布门禁。
- StartupError.source 使用 Box，错误链与清理重试能力保留。完整迁移见 docs/migration/0.1-development.md。

当前为开发期 Candidate；是否 Release Ready 由全部强制检查与实际 Agent 评估决定，未发布到 crates.io / docs.rs。
