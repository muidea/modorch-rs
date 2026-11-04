# 文档与 Skill 验证

Python 3.11+ 标准库即可执行静态检查；Cargo 及锁定依赖用于行为验证。

```bash
python3 tools/doc-validation/run.py
python3 tools/doc-validation/run.py --results <真实-workspace-test.log> --tooling-results <真实-unittest-v-log>
python3 tools/doc-validation/affected.py
```

run.py 分别检查本地链接、真实测试函数映射、Skill 格式与引用、能力证据和生成快照；没有 --results 时，Contract 检查只证明映射存在，不声称测试执行通过。tooling_tests 单独映射 Python 门禁，只有传入 --tooling-results 且含真实通过行才报告工具执行 Passed。远程链接不主动联网检查。

完整本地验收保留每条命令、退出码和原始日志：

```bash
python3 tests/agent-evals/fixture-replay.py --output validation/fixture-evidence
python3 tests/agent-evals/run.py --output validation/agent-plan
python3 tools/doc-validation/verify.py --output validation/local-checks --fixtures validation/fixture-evidence/summary.json --agent-plan validation/agent-plan/summary.json
python3 tools/doc-validation/release-gate.py validation/local-checks/verification.json
```

输出目录必须不存在；使用新目录保留每次失败和重验记录。verify.py 离线检查 fmt/check/clippy/test/doctest/rustdoc/MSRV、Feature 组合、六个示例、两个模板、外部消费者、工具测试与 Skill 格式，核对已有干净目录回放日志的摘要；需要 Rust 1.85.0、当前工具链与 skill-hub。离线依赖不齐时不能据此声称通过。

默认 Agent plan 是十二例 NotRun；fixture 是确定性基础设施验证。只有实际 Agent 执行、原始工件和语义复核均通过时才满足对应发布条件。release-gate.py 在本地检查通过但独立 Agent 证据不足时仍返回非零并标记 Candidate。完整本地验收通过与 Release Ready 是两个独立结论。
