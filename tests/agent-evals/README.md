# Agent 任务评估

cases.json 固定 AE-001 至 AE-012 的 Goal、允许范围、版本和验收。M2 的 P0 用例为 AE-001..008、010、012；安全用例 AE-011 同属发布阻断。AE-009 与跨宿主比较为 P1；声明支持的宿主必须实际验证。

fixture-replay.py 在干净临时目录安装完整 Bundle、生成独立应用，构建/测试/运行真实示例和模板，并执行兼容、安全与门禁的负面测试。其 mode=fixture，只验证确定性基础设施，不能冒充 Agent 推理或触发效果。

run.py 提供实际 Agent 的顺序执行适配：先准备固定 Goal 和独立仓库，执行显式传入的命令数组（无 shell），保存 stdout/stderr、原始输入、客户端版本、变更 diff、结果报告和复核状态。未提供 Agent 命令时仅生成 NotRun 清单。命令从 stdin 读 Goal，结果需写 result.json，遵循 Skill 报告 Schema。执行环境/联网/权限由宿主控制，工具不会自行启动默认 Agent 或安装客户端。

```bash
python3 tests/agent-evals/fixture-replay.py --output /tmp/modorch-fixture-evidence
python3 tests/agent-evals/run.py --output /tmp/modorch-agent-notrun
```

实际执行前明确选择宿主及版本，再向 run.py 传 --case AE-001 --agent-command-json '<命令数组>' --agent-version '<实际版本>' --host '<宿主名称>'；不能把上述占位符直接当命令。每例独立临时仓库，不继承对话；失败不得改写 Goal、降低标准或删除日志。Agent 必须写 result.json 和包含原始验证命令、输出的 validation.log；harness 将它们与输入、命令、diff、stdout/stderr 一起保存，缺失即 Failed。输出 `review.status=NotRun` 等待人工语义复核，发布门禁只接受完整证据与复核 Passed 的实际 Agent 记录。

评估至少检查版本/指纹、真实 Facade API、角色与消息边界、正常/异常退出、代码构建、原始命令与最终报告一致。AE-007/008/010 正确输出 Blocked/NotRun 可以是评估通过，但必须有对应行为证据。跨模型结果比较以可观察语义、失败处理和证据完整性为准。
