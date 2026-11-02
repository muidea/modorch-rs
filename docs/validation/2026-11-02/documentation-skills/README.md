# 文档与 Skill 收口验收（2026-11-02）

状态：Candidate。先以当前实现完成文档刷新，再建设规范要求的交付物；独立 Agent P0 / 多宿主 E2E 尚未执行，不能标 Documentation & Skills Release Ready。

基线 commit 37bdc93；modorch 0.1.0、MIT、Edition 2024、MSRV 1.85。验证平台 Linux x86_64，本机 Rust 1.94.0。基线源码摘要见 [baseline.json](baseline.json)，实际执行结果见 [baseline-results.json](baseline-results.json)，刷新差异见[实现核对](baseline-audit.md)。历史 2026-10-31 证据不改写。

## 交付与需求追踪

| Req ID | 交付 | 验证与状态 |
|---|---|---|
| DOC-001 | README / Getting Started / 两模板 / 标准示例 | 干净目录 check/test/run 由 fixture replay 记录 |
| DOC-002 | 角色决策指南 / Component Contract | 角色案例、Factory / 私有状态 / 可选端点测试 |
| DOC-003 | Messaging Guide / Contract | Event/Command/Request、超时、FIFO、Lane、路由与预算测试 |
| DOC-004 | Lifecycle / Resource Contract | 启动补偿、关闭超时、租约、Forced、代际恢复测试 |
| DOC-005 | 公共 Rustdoc / API 导航 / Feature 表 | deny(missing_docs)、doctest、warnings-as-errors cargo doc |
| DOC-006 | capability-matrix / limitations / 基线刷新 | 状态、规则、证据与源码指纹检查 |
| SKL-001 | 两个 P0 Skill + 维护 Skill | 静态触发正反边界；实际 Agent 自动定位 NotRun |
| SKL-002 | Cargo metadata preflight / 版本指纹 | 不同版本、同版本不同源码、缺 Feature 拒绝测试 |
| SKL-003 | 自包含 Bundle / 安装器 / 参考快照 | 外部干净目录安装、幂等、构建和校验 |
| SKL-004 | result schema / 便携校验脚本 / harness | NotRun 与 Passed 混报拒绝；实际 Agent 报告 NotRun |
| SKL-005 | 宿主适配说明 / 固定 AE 用例 | 多宿主实际对比 NotRun，不声明自动发现兼容 |
| VAL-001 | CI / 本地文档工具 / 示例模板 | 格式、check/clippy/test/doc/MSRV 与快照检查 |
| VAL-002 | Contract Rule → Test ID → 测试函数映射 | 映射源码检查与实际 Cargo 测试日志检查分开 |
| VAL-003 | release-gate.py | 缺少/失败/NotRun、fixture 冒充 Agent 均阻断 |
| SEC-001 | Skill 不受信输入边界 / 安装器限制 | 穿越、链接、重复项、篡改、冲突与超预算拒绝；真实 AE-011 NotRun |

## 证据解释与发布阻断

baseline-* 日志为修改前的真实执行；本次最终[聚合证据](verification.json)保存命令、退出码、日志摘要、框架与文档/Skill/工具指纹，原始顺序执行记录保存在 final-checks/verification.json。模板和 Bundle 的最终确定性外部验证保存在 final-fixture-replay；实际 Agent 用例另以 agent-evals 记录 NotRun，二者不能互相替代。初次 fixture-replay 的编译超时和 initial-workspace-test.log 的调度敏感测试失败保留原始记录，最终验收另存，不覆盖失败。

本地结果：fmt、全目标全 Feature check、严格 Clippy、workspace test、独立 doctest、warnings-as-errors Rustdoc、MSRV 1.85 check、六种 Feature 组合、六个示例直接运行、两个模板 fmt/check/test/run、外部消费者、三份 Skill 格式/链接、工具测试与静态门禁均 Passed。workspace 共 109 项通过（101 项普通测试、8 项 doctest）；Python 工具测试 14 项通过；37 个 Rust Contract 映射均有实际 Passed 行；21 项能力与 434 个本地链接通过最终检查。详细数量以静态日志为准。

干净目录回放共 51 步 Passed，涵盖最小/组合应用、Event、Request 超时、定时任务、服务释放、启动补偿、错误依赖图、关闭超时及兼容/安全/门禁负面基础设施测试。上述行为验证不能替代 Agent 的语义与触发验收。

最终分发工件为 [modorch-skills-1.0.2-candidate.tar.gz](modorch-skills-1.0.2-candidate.tar.gz)，Contract revision 为 1.0.0。安装、幂等重装、安装后 preflight 与当前负载摘要一致性已实际检查；[分发证据](final-bundle-1.0.2-install.json)确认其 22 个脚本与模板素材同已完成回放的 1.0.0 中间包逐文件相同。旧中间包保留为过程证据。三个 Skill 均已校验、dry-run 后归档至本地 skill-hub 仓库，状态为 synced 1.0.2，未推送远程。

[发布门禁](release-readiness.json)保持 Candidate，仅剩 AE-001..008、AE-010..012 的实际独立 Agent 执行/复核证据不足；AE-009 和至少两种宿主比较也为 NotRun。完整结构化任务报告为 task-result.json，整体 Partial，不能将本地工程通过扩展为最终发布通过。

release-gate 的非零是预期的缺证据阻断，不等同于 Cargo 或静态检查失败。本地工程交付与实际跨 Agent 行为验收分别报告；未执行远程 CI、Rust 1.99、其他 OS 或独立客户端，不据此声称通过。

M0 的真实 API/依赖审计与基线验证、M1 的指南/Contracts/示例/Rustdoc 已完成本地工程检查。M2 的 Skill/Bundle/干净目录基础设施、M3 的维护流程与漂移/发布门禁已实现并验证；实际 Agent E2E 和至少两种宿主的语义对比保持未通过，不以文件齐全关闭。
