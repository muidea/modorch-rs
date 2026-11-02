# ADR-001：按实现刷新文档并保持契约可验证

- Date: 2026-11-02
- Status: Accepted for current development tree
- Context: 旧设计草案中的 TaskSpec、Vec Handler、signals、v0.3 和缺失 Lane/Restart 的描述与 0.1 实现冲突，分散维护签名容易继续漂移。
- Decision: 先刷新文档，以源码/Rustdoc 作为签名单一入口，Contracts 作为规范性语义维护点。每条行为规则映射真实测试，公开能力只在对应日志执行通过的范围内标 Verified。Skill 参考由受控文件生成并记录逐文件摘要，不单独编辑。
- Consequences: 修改公开 API 必须连同 Contract、教程、示例、Skill revision 和兼容表复核；生成快照过期或缺证据时门禁失败。开发期同版本源码以指纹区分，不拿 0.0.1 字符串代替兼容性验证。
- Alternatives: 全部复制签名无法保证同步；完全以代码取代契约无法判断实现缺陷，因此不采用。
- Verification: tools/doc-validation/run.py、tests/contracts/mapping.json、Skill Bundle 完整性与 preflight 测试。
