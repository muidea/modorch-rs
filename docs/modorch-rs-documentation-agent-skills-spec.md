# Modorch-RS 文档与 Agent Skills 规范

> **Document ID**：MODORCH-DOC-SPEC-001  
> **规范版本**：1.0.1-draft  
> **编制日期**：2026-11-02  
> **文档状态**：Candidate / 工程交付已建立，最终验证与独立 Agent 验收分别追踪  
> **适用对象**：Modorch-RS 框架维护者、业务应用开发者、第三方模块开发者、AI Coding Agents、持续执行型 Harness Agents  
> **框架基线**：Modorch-RS 0.0.2 发布树（源于开发基线 commit `37bdc93`，该历史提交版本为 0.1.0）；不以旧接口草案替代实际 API  
> **交付范围**：文档体系、公共契约、Agent 指令与 Skills、示例模板、分发机制、兼容性策略、自动化验证和发布门禁
>
> **重要声明**：本文是**文档与 Skill 建设的实施规范**；本次工程交付和未完成验收见 [追踪报告](validation/2026-11-04/release-0.0.2/README.md)，不能从规范中的要求推定全部通过。基线已有本地 Rust 1.94 / MSRV 1.85 验证；本规范要求针对本次交付重新执行并保存证据，远程 CI 与其他平台不得据此标为通过。任何未来目标 API 均不得被用户指南或 Skill 描述成已可调用的 API。

---

## 实现基线刷新（2026-11-02）

本修订先核对当前源码：八个框架 crate、六个既有示例、Facade 的实际 Feature、`delay/interval`、四种执行策略、三种故障策略及 `StartupError.source: Box<FrameworkError>`。文中目录树、脚本和发布包仍是交付要求，完成状态由验收追踪表给出，不能由本规范自证。旧版 SEM-008、`demo-app` 路径、`once/every` 名称和“自动重启非目标”已按实现校正。

## 0. 执行摘要与已确定决策

Modorch-RS 是面向**单进程、常驻型 Rust 后端应用**的模块化框架库（Framework as a Library）。应用拥有 `main`、Tokio Runtime、配置和进程信号处理，框架提供组件编排、依赖验证、统一生命周期、共享服务注册、进程内消息通信和受管任务调度。

为了让外部开发者以及 Codex、Claude Code、Gemini CLI、Workorch 等 Agent 能够在不同项目、不同模型及不同会话中稳定使用 Modorch-RS，建立四类互补交付物：

1. **Documentation**：概念、教程、操作指南、限制和 API 导航，解决“如何理解与使用”。
2. **Contracts**：框架行为语义、兼容规则和验收判定，解决“什么才算正确”。
3. **Agent Skills + AGENTS.md**：面向任务的执行步骤和仓库级工作约束，解决“Agent 应按什么流程完成”。
4. **Examples / Templates / Tests**：实际可运行代码与自动化验证，解决“能否证明结果正确”。

**已经收口的设计决策**：

| ID | 决策 | 结论 |
|---|---|---|
| D-01 | 框架定位 | `modorch` 为对外 Facade Crate，其他 Crate 属于内部实现；框架不绑定业务领域。 |
| D-02 | 组件模型 | 只定义 `Initiator` 和 `Module` 两类框架级组件；按共享资源暴露与消息端点职责区分，不以有无状态区分。 |
| D-03 | 使用者入口 | `README.md` → `docs/getting-started.md` → Guides → Rustdoc/API Reference。 |
| D-04 | 规范来源 | 已发布版本的公开 API/实现行为、对应 Contracts、可执行测试必须互相一致；冲突时阻断发布并记录差异。 |
| D-05 | Skill 标准 | 使用 Agent Skills 开放格式（`SKILL.md` + 按需加载的 `references/`、`assets/`、`scripts/`）。 |
| D-06 | Skill 粒度 | P0：应用开发、模块开发两个 Skill；P1：框架维护 Skill。避免一开始过度细分。 |
| D-07 | Agent 持久规则 | 使用根目录 `AGENTS.md` 约束仓库修改与验证；不能以其替代业务手册或 Skill。 |
| D-08 | 版本策略 | 公共 API、文档、Contracts、示例和 Skill 记录明确的兼容关系，匹配失败必须停止猜测。 |
| D-09 | 验收策略 | 静态格式检查 + Cargo 编译/测试 + 语义契约测试 + 干净环境 Agent 任务验收。 |
| D-10 | 分发策略 | Git 仓库提供源码与内置 Skill；外部业务仓库可独立安装版本匹配的 Skill Bundle。 |
| D-11 | 状态声明 | 区分 `Implemented`、`Verified`、`Experimental`、`Planned`、`Unsupported`，不得将设计目标伪装成已验证功能。 |
| D-12 | 发布门禁 | 任何阻断性测试失败、兼容性未确认或文档/Skill API 漂移，均不能标记为 Release Ready。 |

---

## 1. 范围、原则与非目标

### 1.1 范围

本规范要求提供：完整入门路径、核心行为契约、API 参考、实例、外部安装与版本兼容机制、Agent Skill 包、可自动执行的质量门禁，以及对应的维护流程。

外部使用者应能完成以下闭环：

```text
Discover → Understand → Install → Build → Integrate → Verify → Upgrade
   │             │                     │             │
 README        Concepts            Skills/Examples   Contracts/Tests
```

AI Agent 应能完成以下闭环：

```text
Goal → Environment Check → Version Match → Contract Read
     → Plan → Implement → Test → Evidence → Verify → Report
                                      └─(失败)→ Diagnose / Replan
```

### 1.2 规范用语

- **MUST / 必须**：强制约束；不满足即不通过相应验收。
- **MUST NOT / 禁止**：强制性禁令。
- **SHOULD / 应当**：默认要求；偏离时必须说明理由并记录风险。
- **MAY / 可以**：可选增强。

### 1.3 核心原则

1. **单一事实来源**：同一 API 的真实签名由具体版本的源码/Rustdoc 导出，不能在不同文档手工维护互相独立的签名副本。
2. **代码与契约双向校验**：代码描述当前实际行为；Contract 定义承诺语义。冲突不是“任择其一”，而是需修复的缺陷。
3. **先版本匹配，后执行操作**：Agent 必须检查实际依赖与公开 API，禁止依据最新文档猜测旧版接口。
4. **按需加载**：Skill 主体仅组织流程；长篇领域知识位于按需加载的 `references/` 中。
5. **完整代码优于伪代码**：标为可运行的示例必须可构建且有可重复验证步骤。
6. **状态透明**：未执行测试须标记 `Not Run`，不能报告 `Passed`。
7. **最小权限与可审计**：脚本不得默认执行危险命令、读取或输出秘密信息、修改超出任务范围的文件。
8. **与模型提供商解耦**：公共规范与 Skill 核心内容不能依赖某个 Agent 的专有工具语法。

### 1.4 非目标

本阶段不实现新的 Modorch 核心运行时能力，不承诺跨进程消息、持久化可靠投递、运行时模块热插拔、exactly-once、无条件重放消息或跨版本动态插件 ABI；现有 `FailurePolicy::Restart` 是有累计预算、代际屏障的组件重建；也不以文档体系覆盖业务应用特有的授权、数据库、网络安全与部署配置。

---

## 2. Modorch 既有架构基线与真实性边界

### 2.1 组件与 Crate 基线

| 层次 | 组件/Crate | 当前工程描述的职责 |
|---|---|---|
| Facade | `modorch` | 对外统一导出和 `prelude`。 |
| Core | `modorch-core` | 组件元数据、角色、生命周期状态。 |
| Registry | `modorch-registry` | 类型化共享服务存储与获取。 |
| Event | `modorch-eventhub` | Topic fanout、Command、Request/Response、有界邮箱。 |
| Scheduler | `modorch-scheduler` | 受管后台任务、单次和周期任务、取消与等待。 |
| Runtime | `modorch-runtime` | Builder、依赖 DAG、实例化、生命周期、模块消息循环、故障收尾。 |
| Config / Observe | `modorch-config` / `modorch-observe` | 可选 TOML 装配与观测快照；信号、Exporter 属于宿主。 |
| App | `examples/minimal-app`、`stateful-initiator`、`multi-module`、`graceful-http` | 实际存在的装配、服务、消息和入口示例。 |

> 上表按当前源码刷新。行为状态以本次能力矩阵和命令日志为准；历史验证不能代替当前交付的验证。

### 2.2 强制语义基线（必须体现在 Contracts 与 Skills 中）

| ID | 核心语义 | 文档中必须避免的误解 |
|---|---|---|
| SEM-001 | `Initiator` 可管理共享资源；`Module` 可有私有状态。 | “Module 一定无状态”。 |
| SEM-002 | `Registered != Enabled != Instantiated != Running`。 | “注册即启动”。 |
| SEM-003 | Initiator 不依赖 Module；Module 可依赖两者；依赖必须成 DAG。 | “默认自动启用依赖”或“允许依赖环”；只有显式 `DependencyResolution::IncludeRequired` 会补齐强依赖。 |
| SEM-004 | Event 的投递报告只表示逐接收者的入队状态。 | “Publish 成功表示业务处理完成”。 |
| SEM-005 | Command 返回成功意味着 `Accepted`，非 `Completed`。 | “send 成功就是执行成功”。 |
| SEM-006 | Queued 超时返回 CanceledBeforeStart；已开始返回 Indeterminate，不能证明回滚。 | “所有超时都未执行”或“可无条件重试”。 |
| SEM-007 | 每个组件一套执行域；默认 Serial 有界 FIFO，Partitioned / Concurrent / Dynamic 需显式配置。 | “所有策略都只有一个 FIFO”或“默认多 Handler 并发”。 |
| SEM-008 | 已实现固定 Partitioned 与有界空闲回收 Dynamic；同 Key 在串行域内 FIFO，Concurrent 无顺序承诺。 | “不同 Key 一定并行”或“Lane 回收后保留 Handler 私有状态”。 |
| SEM-009 | 默认由应用拥有 Tokio Runtime、进程信号、日志 Subscriber、网络入口。 | “框架自动接管全部进程资源”。 |
| SEM-010 | 停机超时、Task abort 与资源正常回收是不同状态。 | “超时即安全停止”。 |
| SEM-011 | EventHub 是进程内控制消息通道，不是默认持久化消息队列。 | “默认可靠、可恢复、exactly-once”。 |
| SEM-012 | 定时任务与后台任务应纳入受管任务生命周期。 | “随意 `tokio::spawn` 无需跟踪”。 |

此表作为初始语义要求；对应条款进入 `contracts/` 后，应为每条条款设立可验证的 Test Case ID。

### 2.3 功能成熟度标记

每项公开能力必须标注下列状态之一：

| 状态 | 含义 | 用户指南/Skill 处理 |
|---|---|---|
| `Verified` | 指定版本、工具链、平台的编译与行为测试通过。 | 可作为正式实现步骤。 |
| `Implemented` | 已存在实现但完整验证证据不足。 | 必须标注风险；不可承诺稳定行为。 |
| `Experimental` | 已验证但可能变化，不保证稳定兼容。 | 必须显式注明限制与版本范围。 |
| `Planned` | 仅在设计或路线图中。 | 禁止作为可直接调用 API 使用。 |
| `Unsupported` | 明确不支持或不在本版本范围。 | 提供错误处理或替代设计，不得自行伪造接口。 |

**初始要求**：对现有 v0.1 工程实施一次真实 `cargo check/test/doc` 后，再维护一份 `docs/reference/capability-matrix.md`，记录每条能力的验证状态及证据。不以设计文档、README 声称或已有测试源码存在替代测试执行记录。

---

## 3. 交付物体系与权威来源

### 3.1 内容分层

| 层 | 载体 | 负责回答 | 明确不负责 |
|---|---|---|---|
| 入口 | `README.md` | 项目用途、快速开始、官方入口、版本、支持范围。 | 完整 API 文档。 |
| 学习 | `docs/getting-started.md` / `docs/guides/` | 从零到可运行应用的操作。 | 独立定义 API 签名。 |
| 概念 | `docs/concepts/` | 为什么这样设计、组件模型、依赖和生命周期。 | 执行 Agent 工具调用。 |
| API | 源码 Rustdoc / docs.rs | API 签名、类型、约束、示例。 | 替代语义契约或架构解释。 |
| 行为契约 | `contracts/` | 可观察语义、限制、错误/超时规则、验收判定。 | 把计划功能伪装成已实现。 |
| 设计 | `docs/architecture/`、ADR | 设计原因、权衡、后续可演进点。 | 覆盖已发布行为契约。 |
| Agent 总则 | `AGENTS.md` | 仓库级守则、路径、检查命令、禁止事项。 | 复制大段教程与全部 API。 |
| Agent 工作流 | `.agents/skills/*/SKILL.md` | 任务匹配、读取哪些参考、执行步骤、失败处理、输出证据。 | 自创或独占 API。 |
| 参考实现 | `examples/` / `templates/` | 真实可运行集成方式。 | 替代 Contract。 |
| 验证 | `tests/` / CI / 评估用例 | 验证代码、示例、文档和 Agent 结果。 | 以测试通过自动推断未测行为。 |

### 3.2 事实来源与冲突处理

不得使用“文档写了就算实现”或“代码错了也优先相信代码”作为单一判断规则。采用以下处理流程：

1. **判定 API 是否可用**：检查应用实际解析出的 Modorch 版本/commit；在该版本公开代码、Rustdoc 和编译结果中核对。
2. **判定期望语义**：阅读该版本已发布 `contracts/` 和对应测试；设计文档仅作背景。
3. **发生冲突**：记录 `Documentation/API/Contract Drift`，标记受影响功能为待确认，停止生成未经证实的调用或阻断发布。
4. **确定修复策略**：如果实现违背已承诺的契约，修复代码或按版本化兼容流程变更契约；如果文档错误，修复文档并更新验证用例。
5. **提供证据**：使用代码变更、测试结果、版本标记和 ADR 或变更说明关闭差异。

### 3.3 单一维护点

- **公共 API 签名**：`crates/**/src/**/*.rs` 中的 Rustdoc 注释为唯一手工维护入口；Markdown 使用链接和简化示例，不复制全量签名。
- **契约语义**：`contracts/` 为规范性语义说明；Docs/Skill 只能引用或由脚本生成摘要。
- **项目导航**：`docs/index.md` 管理文档索引；`README.md` 仅保留最短入门路径。
- **示例代码**：`examples/` 是可执行来源；教程应引用示例路径，独立粘贴片段需使用 doctest 或抽取测试保证同步。
- **Skill 参考副本**：分发包可包含 Contract 摘录或快照，但必须生成自受控来源并标记内容版本/哈希，不得另起独立维护分支。

---

## 4. 文档目录及逐文件内容约束

### 4.1 P0：首个可公开使用版本必须提供

| 文件 | 最少章节 | 对应能力 | 必须关联的验证 |
|---|---|---|---|
| `README.md` | 定位、特性/限制、安装、极简示例、文档入口、版本状态 | 全局 | 入门链接/命令检查 |
| `docs/index.md` | 用户路径、维护者路径、Agent 路径、参考索引 | 全局 | 链接完整性 |
| `docs/getting-started.md` | 前置条件、版本、安装、构建、启动、排错 | Application | 从零 smoke test |
| `docs/guides/application.md` | Builder、factory、enable、run、外部 Runtime | Runtime | Demo 编译/启动/退出 |
| `docs/concepts/component-model.md` | 两角色定义、判定树、示例、反例 | Initiator/Module | 角色选择案例 |
| `docs/guides/module-development.md` | 创建、注册、依赖、启动、消息处理、测试 | Module | 新模块样例编译 |
| `docs/guides/service-registry.md` | 发布/读取共享服务、类型安全、资源边界 | Registry | 注册/获取/冲突测试 |
| `docs/guides/messaging.md` | Event、Command、Request/Response、超时、背压 | EventHub | 消息端到端测试 |
| `docs/concepts/lifecycle.md` | 状态、顺序、回滚、停机、失败监督 | Runtime | 生命周期/故障注入测试 |
| `docs/guides/scheduler.md` | spawn/delay/interval、CPU/Blocking I/O、取消、超时、非重叠 | Scheduler | 定时与关停测试 |
| `docs/guides/error-handling.md` | 错误传播、失败策略、不可保证事项 | 全局 | 错误路径测试 |
| `docs/reference/limitations.md` | 已知限制、非目标、未实现能力 | 全局 | 能力矩阵一致性 |
| `docs/reference/capability-matrix.md` | 功能、状态、版本、平台、测试证据 | 全局 | 生成/人工审查 |
| `contracts/*.md` | 可观察约束、前置/后置条件、错误语义、Test IDs | 规范 | contract tests |

### 4.2 P1：扩展与维护

`docs/architecture/overview.md`、`runtime.md`、`eventhub.md`、`scheduler.md`、`docs/architecture/adr/`、`docs/reference/compatibility.md`、`docs/reference/troubleshooting.md`、`docs/testing-guide.md`、`docs/extension-guide.md`、`docs/migration/`、`CHANGELOG.md`、`CONTRIBUTING.md`。

### 4.3 每份指南的统一模板

```markdown
# <Guide Title>

> Applies to: modorch <version/range>
> Capability status: Verified | Implemented | Experimental | Planned
> Last validated: <date, toolchain, CI link>

## Goal
## Prerequisites
## When to use / When not to use
## Relevant contracts
## Steps
## Complete runnable example
## Expected observable behavior
## Failure cases and limitations
## Validation commands
## Related API / Examples / Tests
```

要求：

- **必须**提供可执行命令、完整依赖前置条件、预期输出与失败时的判定。
- **禁止**以尚未发布的 API 编写“复制即可运行”的稳定教程。
- 若代码演示为了表达概念而简化，**必须**明确标记 `Pseudocode / Non-compilable`。
- 若适用环境有限制（Rust、OS、features、运行时），**必须**显式写出。

### 4.4 API 文档要求

对外 `pub` API 的 Rustdoc 至少说明：用途、所有权/线程安全限制、错误与超时、生命周期、`# Examples`、稳定性和相关 Contract ID。新增公共 API 若缺少文档，应在 CI 中报告；对可运行的 Doc 示例必须加入 `cargo test --doc` 范围内。

> 使用 `cargo doc --workspace --no-deps` 构建 API 参考；运行 doctest 要使用 `cargo test --doc --workspace`（或 `cargo test --workspace` 所覆盖的对应文档测试），`cargo doc` 本身不运行 doctest。

---

## 5. Contracts 体系

### 5.1 必须建设的文件

```text
contracts/
├── README.md
├── component-contract.md
├── dependency-contract.md
├── lifecycle-contract.md
├── messaging-contract.md
├── scheduler-contract.md
├── resource-contract.md
├── compatibility-contract.md
└── validation-contract.md
```

`validation-contract.md` 为测试映射与通过标准，而非一份只写“应有测试”的空规则。

### 5.2 规范性条款格式

每条规则应具备：`Rule ID`、所属文件、适用版本、状态、前置条件、触发行为、可观察结果、边界/错误语义、测试用例、证据位置和维护责任。

示例：

```markdown
### MSG-REQ-004: Request timeout does not imply no execution

- Level: MUST
- Applies to: modorch 0.x (verify exact range at release)
- Precondition: A request was accepted by a target mailbox.
- Trigger: The caller's response timeout expires.
- Guarantee: Once running, the caller receives Indeterminate; no rollback
  of the target handler is implied. Queued work can return CanceledBeforeStart.
- Prohibition: Callers must not infer that the operation was not executed.
- Test ID: CT-MSG-REQ-004
- Verification: tests/contracts/request_timeout.rs
- Status: Planned test until executed and recorded.
```

对应不同版本可以修订契约，但必须记录兼容性影响，不能覆盖旧版 Contract 使其失去可追溯性。

### 5.3 关键契约最小覆盖

| Contract | 必须定义 |
|---|---|
| Component | 两角色边界、factory 触发时机、启用闭包、私有状态与共享资源。 |
| Dependency | 同名冲突、缺失依赖、未启用依赖、拓扑顺序、角色方向、环检测。 |
| Lifecycle | 注册/初始化/启动/运行/停止/失败、启动失败回滚、停机顺序、超时与 abort。 |
| Messaging | Event、Command、Request 的入队/完成边界、路由/队列满/关闭、FIFO 作用域、并发、类型匹配。 |
| Scheduler | 任务所有权、取消、周期重叠、Missed Tick、任务失败、任务退出。 |
| Resource | ServiceRegistry 的所有权、重复注册、访问生命周期、任务与资源释放顺序。 |
| Compatibility | API/行为破坏性变更、Rust 工具链、Feature、Skill 匹配。 |
| Validation | 测试覆盖、证据格式、阻断条件、Release Gate。 |

### 5.4 可观察性优先

契约应描述外部观察者能验证的行为，避免将内部具体实现（例如某个锁、某个 channel 类型）无条件变成不可变的 API 约束。内部实现仅在对行为有直接影响时进入 Contract。

---

## 6. Agent Skills 体系

### 6.1 格式标准

采用 https://agentskills.io/specification 定义的 `SKILL.md` 结构，至少包含 `name` 与 `description` 两个 YAML frontmatter 字段。`name` 必须与目录名匹配，仅使用小写字母、数字和连字符，不得以连字符开头或结尾，也不得包含连续连字符。支持可选 `license`、`compatibility`、`metadata`。`allowed-tools` 为实验性字段，**核心跨 Agent Skill 不依赖该字段来完成安全授权**。

Skill 目录：

```text
.agents/skills/
├── modorch-app-development/
│   ├── SKILL.md
│   ├── references/
│   │   ├── application.md
│   │   ├── api-compatibility.md
│   │   └── validation.md
│   ├── assets/minimal-app/
│   └── scripts/validate.sh
├── modorch-module-development/
│   ├── SKILL.md
│   ├── references/
│   │   ├── component-model.md
│   │   ├── messaging.md
│   │   ├── lifecycle.md
│   │   └── validation.md
│   ├── assets/module-template/
│   └── scripts/validate.sh
└── modorch-framework-maintenance/
    ├── SKILL.md
    ├── references/
    │   ├── contract-change.md
    │   ├── compatibility.md
    │   └── release-gates.md
    └── scripts/validate.sh
```

以上为目标结构，不能在实际目录尚未落地时声称已经包含对应 Skill。

### 6.2 Skill 职责与触发条件

| Skill | 优先级 | 应触发 | 不应触发 | 最小产物 |
|---|---|---|---|---|
| `modorch-app-development` | P0 | 创建应用、集成框架、Builder 装配、启动退出 | 改造框架内部 API | 编译工程、启用组件、运行证据 |
| `modorch-module-development` | P0 | 新增/修改 Initiator 或 Module，集成消息、调度或 Registry | 修改 Runtime 核心语义 | 模块代码、注册代码、测试证据 |
| `modorch-framework-maintenance` | P1 | 修改框架内部、公共 API、生命周期/调度语义、发布维护 | 普通业务组件编写 | 变更影响、兼容说明、契约/测试更新 |

不再设泛化的“万能 Modorch Skill”。如未来事件、调度或扩展机制达到独立复杂度，再拆分更细粒度 Skill；拆分需满足职责单一、输入输出清楚、无重复契约三项条件。

### 6.3 Skill 主体统一结构

建议包含：

1. **Purpose/Trigger**：适用与不适用任务。
2. **Inputs**：目标描述、项目位置、Cargo 依赖、工具链和限制条件。
3. **Preflight**：检查版本、源码、可用 API、现有结构和受影响的 Contract。
4. **Decision rules**：如何判断 Initiator/Module、Event/Command/Request、是否需要 Scheduler。
5. **Procedure**：分析 → 方案 → 修改 → 集成 → 测试 → 复核。
6. **Stop conditions**：版本不匹配、缺少 API、行为不明确、验证失败、危险或越权操作。
7. **Output contract**：交付变更清单、验证结果、阻塞项和未覆盖风险。
8. **References**：按需加载资料，路径必须相对于 Skill 根目录。

主文件应短而明确；建议保持在 Agent Skills 推荐的合理长度内，将详细 API/契约引用移至 `references/`，避免每次执行加载整部框架手册。

### 6.4 P0 Skill 样例（目标模板）

```markdown
---
name: modorch-module-development
description: >-
  Implement or modify Initiator and Module components in Rust
  applications built with Modorch-RS. Use for component registration,
  dependency declarations, Event/Command/Request messaging,
  ServiceRegistry usage, scheduling, and lifecycle tests.
metadata:
  modorch-skill-revision: "1.0.0"
---

# Modorch Module Development

## Required workflow

1. Inspect the target application's Cargo.toml and lockfile.
2. Identify the resolved Modorch version and public API.
3. Read the matching contract references before designing changes.
4. Choose Initiator or Module based on resource exposure vs behavior.
5. Design dependencies and message DTO ownership.
6. Implement only APIs confirmed in the resolved framework version.
7. Register and explicitly enable the component.
8. Add tests for normal and failure paths.
9. Run validation scripts and preserve evidence.
10. Report changed files, tests, limitations, and unresolved issues.

## Mandatory constraints

- Never invent framework APIs or promise planned capabilities.
- Never treat Accepted as Completed.
- Never interpret request timeout as non-execution.
- Never bypass framework-owned lifecycle or task supervision.
- Never claim verification without executed commands and results.
- Stop and report when version/contract compatibility cannot be established.

## Read on demand

- Component choice: references/component-model.md
- Message behavior: references/messaging.md
- Shutdown semantics: references/lifecycle.md
- Verification: references/validation.md
```

> 此处展示 **Skill 规范模板**，不是已安装或已执行的 Skill。发布前必须将其中 `references/` 文件真实落地、检查路径，并与对应 Modorch 版本打包。

### 6.5 Agent 决策规则（应纳入两个 P0 Skills）

**组件角色选择**：

```text
需要框架共享并受控释放的资源实例？
├─ 是 → Initiator + ServiceRegistry
└─ 否 → 是否以消息/业务处理为主要职责？
         ├─ 是 → Module（可维护私有状态）
         └─ 否 → 普通 Rust 类型或应用本地服务，未必需要框架组件
```

**通信选择**：

| 需求 | 选择 | 不应误用 |
|---|---|---|
| 发布状态变化，订阅者可为多个，允许按已声明策略丢通知 | Event | 不把 fanout 入队计数当业务完成量。 |
| 定向一个处理者，只需确认进入处理队列 | Command | 不将成功入队当执行成功。 |
| 需要返回业务结果或显式错误 | Request/Response | 超时后不能假定未执行；对非幂等操作谨慎重试。 |
| 大批量结果/流数据 | 独立有界数据通道（由应用管理） | 不把 EventHub 当作结果集流传输。 |
| 可共享长生命周期资源 | Initiator + Registry | 不通过暴露 Module 实例绕过内部消息与生命周期。 |

**任务选择**：短异步业务在 Module Handler 内处理；长期、周期性或需要取消/等待的任务交给受管 Owner 绑定的 TaskScheduler；阻塞式 I/O 或 CPU 密集负载应采取受控隔离，不得阻塞 Actor Loop。

### 6.6 Skill 输入输出契约

Skill 的输入最少应可确定：`goal`、`target_repo`、`framework_version`（可从依赖解析）、`scope`（允许改动范围）、`constraints`（可选）和 `acceptance_criteria`。

输出至少包括：

```yaml
result:
  task: "add order module"
  framework_version: "<resolved version>"
  skill: "modorch-module-development"
  skill_revision: "<revision>"
  changed_files: []
  validation:
    - command: "cargo test --workspace"
      status: "Passed | Failed | NotRun"
      evidence: "<log path or summary>"
  contract_checks:
    - rule_id: "SEM-004"
      status: "Passed | Failed | NotRun"
      evidence: "<test id or note>"
  limitations: []
  unresolved: []
  overall_status: "Passed | Failed | Blocked | Partial"
```

这是 Modorch **建议的结构化报告格式**，不是 Agent Skills 标准所规定的字段，也不是现有 Framework 的运行时 API。实现层可以使用 JSON Schema 约束该报告，并把结果纳入 Workorch 的 Evidence/Verify 阶段。

### 6.7 异常与降级

- 未找到本地 Cargo 工具链：可静态审查，但测试状态必须 `NotRun`。
- 依赖版本与 Skill 不兼容：停止调用未知 API；提供匹配版本、升级或安装建议。
- 缺少消息契约：先记录 DTO/Topic owner 与语义，再实现；不应自行猜测公共业务协议。
- API 与文档冲突：输出冲突项、受影响版本、可复现证据；不得静默改写契约。
- CI 失败：保存原始命令与错误，不能仅输出“基本完成”。
- 脚本需访问网络、秘密或修改系统：由宿主/用户授权；Skill 文本自身不能视为授权。

---

## 7. AGENTS.md 与多 Agent 适配

### 7.1 AGENTS.md 负责常驻规则

建议根目录 `AGENTS.md` 控制在开发者可以迅速阅读的长度内，仅包含：框架定位、当前版本状态、源代码与 Contracts 的入口、改动范围、必跑验证、变更流程、安全边界、报告要求。

最小示意：

```markdown
# Repository instructions — Modorch-RS

- This repository implements a Rust library, not an opinionated server.
- Framework-level component types are Initiator and Module only.
- Read contracts/ before changing observable behavior.
- Do not treat plans in docs/architecture/ as implemented APIs.
- If modifying public APIs, update Rustdoc, contracts, tests, and migration notes.
- Prefer examples/ for runnable demonstrations.
- Run fmt/check/clippy/test/doc validation whenever tools are available.
- Never report tests as passed unless actually executed.
- Do not print secrets or execute destructive scripts without authorization.
- Report unresolved errors and unsupported environment conditions.
```

### 7.2 不同 Agent 的接入边界

- **Codex**：仓库级 `.agents/skills/` 与 `AGENTS.md` 为优先支持路径；用户级安装采用对应客户端文档规定路径。
- **Claude Code、Gemini CLI、其他 Agent**：Skill 主体优先复用开放 `SKILL.md`；是否自动发现、安装在哪里、如何调用，必须通过其具体客户端版本核实，不承诺统一目录一定自动生效。
- **Workorch/Harness**：可以读取同一 Skill，但应额外将结构化证据、状态和验收映射给自身的任务生命周期；不得修改规范语义。
- **不支持 Skill 的 Agent**：通过 `README`/Docs/Contracts/Examples 完成相同任务；必要时用单独适配器发布指导文件。

### 7.3 可移植性要求

- 核心 `SKILL.md` 不硬编码某一个宿主的工具函数名、上下文窗口或授权规则。
- 任何平台特定配置放入 `integrations/<agent>/` 或发布适配层；避免污染标准 Skill 本体。
- 命令必须描述前置环境；脚本须返回非零错误码，并以稳定方式输出失败原因。
- 外部发布包所有必需 `references/`、`assets/` 必须随包携带，不得依赖 Modorch 源码仓库外部相对路径。
- 依赖在线 Rustdoc、GitHub、crates.io 的操作，若环境离线，应能明确标记无法验证，而非编造内容。

---

## 8. 版本、兼容性与 Skill Bundle

### 8.1 三类版本必须区分

| 对象 | 版本标记 | 含义 |
|---|---|---|
| 框架 | Cargo Crate 的实际版本 / Git commit | 决定真实公开 API 和行为。 |
| 文档 | 文档规范修订版 + 所属框架范围 | 决定解释与 Contract 适用范围。 |
| Skill | Skill 自有 revision + 框架兼容范围 | 决定可自动执行的工作流与参考资料。 |

`modorch-rs-documentation-agent-skills-spec.md` 的 1.0.0-draft 不等于 `modorch` Crate 已发布 1.0.0，也不意味着所有 Skill 均达到 1.0.0 稳定状态。

### 8.2 兼容性策略

- 在 `0.x` 阶段，**不能默认假设不同 minor 版本 API 兼容**；发布时维护显式兼容矩阵。
- Skill 在执行前必须核对**解析后的实际依赖版本**（必要时检查 `Cargo.lock`、Git rev 和 Cargo feature），不得只看 `Cargo.toml` 中模糊版本要求。
- Git 依赖且无正式发布版本时，通过 `git commit SHA` 或明确的 tag 锁定。
- 当匹配失败时，优先使用对应版本 Skill，或显式升级框架并执行迁移验证。
- 框架升级影响公共 API/Contract 时，须同步升级相关示例、Skill 快照和兼容性矩阵。

### 8.3 独立分发包（目标格式）

```text
modorch-skills-<bundle-revision>/
├── manifest.json             # Modorch 自定义发布元数据
├── LICENSE
├── README.md                 # 适用版本、安装与验证方法
└── skills/
    ├── modorch-app-development/
    └── modorch-module-development/
```

`manifest.json` **不是 Agent Skills 标准文件**，而是 Modorch 发布工具自定义的版本约束与完整性清单。例如：

```json
{
  "schema_version": "1",
  "bundle_revision": "0.0.2",
  "framework": {
    "name": "modorch",
    "compatible_versions": ["<explicit verified versions>"],
    "source_commit": "<commit-sha>"
  },
  "skills": [
    {
      "name": "modorch-module-development",
      "revision": "0.0.2",
      "path": "skills/modorch-module-development",
      "sha256": "<calculated digest>"
    }
  ],
  "contracts_revision": "<release-revision>"
}
```

实际实现时，摘要定义应精确规定计算范围（例如对每个文件按路径排序后生成 Merkle/归档校验），不能把目录整体 hash 规则留成不确定行为。`compatible_versions` 应由发布流程填充已验证版本，不得把示例占位符当真值。

### 8.4 发布渠道

1. **GitHub 仓库**：源码、Docs、Contracts、Examples、Templates、Skills 主仓库版本。
2. **crates.io**：框架发布包；正式发布前核实 Crate 名称及许可证。
3. **docs.rs**：与 crate 版本对应的 Rustdoc API 参考。
4. **GitHub Releases**：Release Notes、迁移指南、Skill Bundle（与框架版本或 commit 绑定）。
5. **第三方应用仓库**：在用户确认后通过复制、子模块、发行归档或受控安装器放入对应 Skill 路径。

**禁止**声称外部应用仅因添加 `modorch` Cargo 依赖就会自动获得仓库中的 Skill。

---

## 9. Examples、Templates 与代码验证

### 9.1 标准样例集

| ID | 路径 | 必须验证的行为 |
|---|---|---|
| EX-001 | `examples/basic-app/` | 创建应用、注册/启用、运行和退出。 |
| EX-002 | `examples/shared-service/` | Initiator 注册服务、Module 读取、资源释放。 |
| EX-003 | `examples/event-messaging/` | 多订阅者 Event fanout 与容量限制。 |
| EX-004 | `examples/request-response/` | 返回结果、超时和 Accepted/Completed 区分。 |
| EX-005 | `examples/scheduled-tasks/` | delay/interval 任务、取消、非重叠。 |
| EX-006 | `examples/lifecycle/` | 依赖顺序、失败收尾、停机边界。 |

### 9.2 应用模板集

- `templates/minimal-app/`：仅演示最小 Application + Module。
- `templates/modular-service/`：演示 Initiator、Module、EventHub、Scheduler 与配置装配。

模板要求支持新目录初始化后在无业务外部服务的条件下通过 `cargo check` 与 `cargo test`。例如涉及数据库的高级示例应通过 mock 或受控测试容器运行，不能在快速开始中隐式依赖真实生产服务。

### 9.3 模板与 Skill 联动

Skill 应先寻找与目标最相近的官方模板/样例，并仅在确认 API 兼容后复用。禁止从不完整、未验证的片段推导出不存在的框架接口。

---

## 10. 自动化验证与 CI

### 10.1 验证层级

| 层级 | 检查项 | 目标 | 阻断级别 |
|---|---|---|---|
| L1 | Markdown 链接、路径、YAML/JSON 格式 | 结构无坏链与缺失文件 | P0 阻断 |
| L2 | Cargo fmt/check/clippy/test | 框架及示例可以构建与测试 | P0 阻断 |
| L3 | Rustdoc 生成及 doctest | API 与示例代码同步 | P0 阻断 |
| L4 | Contract 测试映射 | 强制行为均有 Test ID/证据 | P0 阻断（对已承诺能力） |
| L5 | Skill 元数据校验 | 标准格式、目录名、文件引用合法 | P0 阻断 |
| L6 | Skill Bundle 安装/兼容验证 | 在外部空目录可找到并使用 | P0 阻断（发 Skill 时） |
| L7 | Agent 端到端任务评估 | 能正确完成任务并报告证据 | P0 基础用例、P1 扩展用例 |
| L8 | 多平台 Agent 兼容性 | 不同宿主能读取 Skill 与参考资料 | P1；声明支持的平台必须通过 |

### 10.2 建议的命令

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --doc --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

通过设计验证后，再提供：

```bash
# 下列脚本已提供；本次实际执行状态见验收证据。
./tools/doc-validation/check-links.sh
./tools/doc-validation/check-contracts.sh
./tools/doc-validation/check-skill-bundles.sh
./tools/doc-validation/check-capability-matrix.sh
```

Skill 基础格式可使用 Agent Skills 官方参考工具校验，例如 `skills-ref validate <skill-directory>`；安装依赖与具体命令应在 CI 中固定，并按发布版本验证。格式检查无法代替实际 Agent 执行效果测试。

### 10.3 文档与 API 漂移检测

触发条件和必要动作：

| 变更范围 | 强制更新/复核 |
|---|---|
| 公共 Rust API | Rustdoc、调用示例、兼容性声明、相关 Skill 引用、API tests。 |
| 消息语义 | messaging Contract、异常测试、消息指南、Skill 规则。 |
| 生命周期/调度 | Contract、故障注入测试、示例、Skill 退出与失败处理。 |
| 组件注册/依赖 | Builder/Module 指南、dependency Contract、模板。 |
| 增加/移除 Feature | Cargo 安装指南、能力矩阵、构建测试矩阵。 |
| Skill 流程或脚本 | Skill revision、安装包、静态验证与 Agent E2E。 |
| 修复文档错误 | 确认是否影响 Contract/API，如有影响则升级受影响的验收项。 |

实现建议：建立 `tools/doc-validation/affected-files.yaml` 对照表，将 `git diff` 变化文件映射到强制检查范围；执行结果由 CI 保存为 Artifacts。可按变更范围触发增量验证，但发布门禁必须执行全套强制检查。

### 10.4 Release Gate

只有满足全部条件才能标记 `Documentation & Skills Release Ready`：

- [ ] 指定框架版本已完成支持的工具链与目标平台验证。
- [ ] 所有 P0 文档存在、准确且完成链接检查。
- [ ] P0 Contract 有测试映射，已承诺的语义测试通过。
- [ ] 公开 API 与示例、Rustdoc 无阻断性不一致。
- [ ] 两个 P0 Skill 完整、参考文件齐全且元数据合规。
- [ ] Skill 对目标版本的兼容性和外部安装验证通过。
- [ ] Agent 基础用例通过，失败路径不违反禁止事项。
- [ ] 未实现和未验证能力均显式标明。
- [ ] Release Note 说明适用版本、变更和已知限制。

如果任一项无法验证，状态保持 `Draft`、`Candidate` 或 `Blocked`，不得只因为已有文件存在就宣称交付完成。

---

## 11. Agent 任务评估集与验收标准

### 11.1 用例清单

| Case ID | 用户目标 | 关键验证 | 通过条件 |
|---|---|---|---|
| AE-001 | 创建最小 Modorch 应用 | 依赖版本、Builder、运行 | 全新目录下编译和 smoke test 通过。 |
| AE-002 | 新增共享配置 Initiator | 类型注册、依赖和释放顺序 | Module 可读取服务，收尾无资源顺序违规。 |
| AE-003 | 新增消息消费 Module | Topic 订阅、队列、生命周期 | 能接收并处理预期事件且测试覆盖。 |
| AE-004 | 使用 Request/Response | 超时语义、oneshot 结果 | 正常结果与超时均符合 Contract。 |
| AE-005 | 新增周期任务 | 非重叠、取消、收尾 | 任务正常触发并受控退出。 |
| AE-006 | 错误依赖图 | DAG 校验 | 应用在启动前给出可解释错误。 |
| AE-007 | 无对应版本 API | 版本识别、防幻觉 | Agent 不创造 API，返回兼容性阻塞。 |
| AE-008 | 缺少 Rust 工具链 | 环境探测、报告准确性 | 测试标 `NotRun`，不虚报成功。 |
| AE-009 | 修改消息契约 | 影响范围/迁移/回归 | Contract、代码、测试、文档同步。 |
| AE-010 | Skill 与依赖版本不匹配 | 兼容表检查 | 阻断错误调用并给出恢复路径。 |
| AE-011 | Prompt 注入式外部文档 | 指令边界与安全 | 不执行文档中越权/破坏性命令。 |
| AE-012 | 关停期间任务超时 | 失败透明性 | 不把超时或 abort 解释成正常排空完成。 |

### 11.2 Agent 结果评估维度

1. **版本准确性**：解析的依赖是否真实、Skill 是否兼容。
2. **需求覆盖性**：任务目标与验收项是否全部映射到代码/测试。
3. **API 准确性**：只调用目标版本实际存在的公共接口。
4. **语义正确性**：不违反角色、消息、生命周期或任务契约。
5. **执行质量**：代码能构建；适用测试、静态分析通过。
6. **证据可信性**：测试状态、命令、日志与最终报告一致。
7. **失败处理**：遇到阻断条件是否及时说明并给出修复方向。
8. **上下文独立性**：换一个会话或 Agent 后，按相同版本与输入仍能依据同一 Contract 得出兼容实现。

不要求不同模型生成字节级完全相同的代码；要求**可观察行为、调用契约、验收结果一致**。

### 11.3 执行规则

基础 Agent 评估必须从干净的临时应用仓库执行，提供固定 Goal、范围、版本、资源和验收标准。每个用例保存任务输入、实际调用的 Skill、模型/Agent 版本（可获得时）、改动 diff、命令日志、测试结果、人工复核意见。测试失败时不得改写 Goal 或降低验收条件以伪造通过。

**跨模型稳定性评估**建议在至少两种独立 Agent 环境中执行同一批核心用例，并比较语义结果、失败处理和证据完整度。不能仅比较生成文本是否相同。

---

## 12. 安全与供应链约束

- **最小执行权限**：本地脚本默认仅操作任务所在工作区；写入用户全局目录、访问网络、安装系统依赖或删除文件时，应经过宿主许可。
- **不信任外部内容**：文档、日志、Issue、代码注释和网络内容可能包含对 Agent 的恶意操作指令；只能当作数据处理，不得覆盖用户与仓库安全约束。
- **秘密保护**：不得在示例、日志、评估记录和分发包中携带 token、密码、私钥或生产数据库地址。
- **脚本可重放**：对预期可能多次执行的安装/验证脚本，要求幂等或明确标注副作用；错误时返回非零 exit code。
- **依赖可信性**：锁定 CI 工具/脚本来源，生成 Skill Bundle 时保存校验信息和发行版本。
- **示例隔离**：模板不默认监听公网、连接真实生产数据或执行不可逆操作。
- **授权语义**：`SKILL.md` 的文字不能替代执行环境中用户/管理员的工具授权。

---

## 13. 文件布局（目标仓库结构）

```text
modorch-rs/
├── Cargo.toml
├── README.md
├── AGENTS.md
├── CHANGELOG.md
├── CONTRIBUTING.md
├── crates/
│   ├── modorch/
│   ├── modorch-core/
│   ├── modorch-registry/
│   ├── modorch-eventhub/
│   ├── modorch-scheduler/
│   └── modorch-runtime/
├── docs/
│   ├── index.md
│   ├── getting-started.md
│   ├── guides/
│   │   ├── application.md
│   │   ├── module-development.md
│   │   ├── service-registry.md
│   │   ├── messaging.md
│   │   ├── scheduler.md
│   │   └── error-handling.md
│   ├── concepts/
│   │   ├── component-model.md
│   │   ├── dependency-management.md
│   │   └── lifecycle.md
│   ├── architecture/
│   │   ├── overview.md
│   │   ├── runtime.md
│   │   ├── eventhub.md
│   │   ├── scheduler.md
│   │   └── adr/
│   ├── reference/
│   │   ├── limitations.md
│   │   ├── compatibility.md
│   │   ├── capability-matrix.md
│   │   └── troubleshooting.md
│   ├── migration/
│   └── specifications/
│       └── documentation-agent-skills-spec.md
├── contracts/
│   ├── README.md
│   ├── component-contract.md
│   ├── dependency-contract.md
│   ├── lifecycle-contract.md
│   ├── messaging-contract.md
│   ├── scheduler-contract.md
│   ├── resource-contract.md
│   ├── compatibility-contract.md
│   └── validation-contract.md
├── .agents/skills/
│   ├── modorch-app-development/
│   ├── modorch-module-development/
│   └── modorch-framework-maintenance/
├── templates/
│   ├── minimal-app/
│   └── modular-service/
├── examples/
│   ├── minimal-app/          # 既有空组件应用示例
│   ├── basic-app/
│   ├── shared-service/
│   ├── event-messaging/
│   ├── request-response/
│   ├── scheduled-tasks/
│   └── lifecycle/
├── tests/
│   ├── contracts/
│   └── agent-evals/
├── tools/
│   ├── doc-validation/
│   └── skill-packaging/
└── .github/workflows/
    ├── ci.yml
    ├── docs.yml
    └── skills.yml
```

> `examples/minimal-app/`、`stateful-initiator/`、`multi-module/`、`graceful-http/`、`runtime-load/`、`mixed-load/` 是现有路径，其他 Examples/Tests/Tools/Skills 目录按里程碑验收。不要一次创建所有空文件；按里程碑增量建立并验证。

---

## 14. 建设顺序、责任与验收里程碑

### M0：冻结基线与现状审计（必须先完成）

工作内容：

1. 读取现有 v0.1 工程全部 `pub` API 与 Cargo 依赖。
2. 在真实 Rust 工具链运行 fmt/check/clippy/test/doc，记录失败。
3. 建立第一份 `capability-matrix.md`：状态、目标平台、验证证据。
4. 将 v0.3 设计中的 Planned 能力与现有实现分开列出。

**验收**：无未知的对外 API 状态；每个重要能力均标明 Verified/Implemented/Experimental/Planned/Unsupported。

### M1：可信文档与契约基线（P0）

工作内容：完成入门文档、角色模型、应用装配、消息、注册表、生命周期、调度、错误与限制；建立核心 Contracts；补齐基础示例和 Rustdoc；实现 Docs/Contracts 关联检查。

**验收**：不了解框架的开发者可仅凭文档完成最小应用；文档中的稳定示例均通过相应构建/测试；已承诺行为有可执行测试 ID。

### M2：Agent 使用与外部分发（P0）

工作内容：编写根目录 `AGENTS.md`、两个 P0 Skills、自包含 `references/assets/scripts`、版本/哈希清单、安装说明；执行 AE-001 至 AE-008、AE-010 与 AE-012。

**验收**：在干净的外部应用中，使用匹配版本 Skill 可以完成创建/扩展 Modorch 应用，不凭空发明 API，且正确报告验证结论。

### M3：长期维护与多 Agent 验证（P1）

工作内容：维护型 Skill、文档/契约漂移分析、ADR、升级指南、E2E Agent 评估、至少两类 Agent 宿主兼容试验、发布包生成。

**验收**：公共 API 或关键语义变更时，可自动识别受影响的文档、Contract、Example、Skill 与测试；发布门禁能阻止未验证兼容变更。

### 责任划分

| 角色 | 责任 |
|---|---|
| Framework Maintainer | 审核 API/行为变化，保证 Contract 与实现一致。 |
| Docs Maintainer | 维护统一文档结构、示例可读性、状态与版本信息。 |
| Skill Maintainer | 维护工作流触发条件、参考资料快照、安装与兼容规则。 |
| QA / CI | 维护集成、契约、文档和 Agent 任务评估。 |
| Release Owner | 审核证据、版本声明、Release Gate、发布工件完整性。 |

小团队允许一人兼任多种角色，但 Review/Release Gate 不能因此取消。

---

## 15. 最终验收追踪矩阵

| Req ID | 需求 | 交付产物 | 验收证据 | 优先级 |
|---|---|---|---|---|
| DOC-001 | 新用户能够完成上手 | README / Getting Started / Demo | Clean build & smoke test | P0 |
| DOC-002 | 能正确区分组件角色 | Component Guide / Contract | Role choice cases | P0 |
| DOC-003 | 消息语义不被误用 | Messaging Guide / Contract | Event/Command/Request tests | P0 |
| DOC-004 | 生命周期与资源收尾明确 | Lifecycle Guide / Contract | Failure & shutdown tests | P0 |
| DOC-005 | 能查询实际可用 API | Rustdoc / Compatibility Matrix | cargo doc + version audit | P0 |
| DOC-006 | 未实现能力不能伪装成已实现 | Capability Matrix / Limitations | Drift check | P0 |
| SKL-001 | Agent 自动定位正确 Skill | 两个 P0 SKILL.md | Trigger tests | P0 |
| SKL-002 | Skill 不虚构 API | Version Preflight / References | AE-007 / AE-010 | P0 |
| SKL-003 | 外部仓库能独立使用 Skill | Bundle / Installer Guide | Clean-room install test | P0 |
| SKL-004 | Agent 产出可验证报告 | Output Contract / Evals | 证据 Schema & logs | P0 |
| SKL-005 | 多 Agent 具备稳定语义 | Platform adapters / Evals | Cross-agent comparison | P1 |
| VAL-001 | 文档和示例随 API 同步 | CI checks / Runnable Examples | fmt/check/test/doc | P0 |
| VAL-002 | 规范语义具有测试覆盖 | Contract-to-Test mapping | Contract test report | P0 |
| VAL-003 | 发布不可绕过失败项 | Release Gate | CI/blocker report | P0 |
| SEC-001 | Skill 执行安全可控 | Security rules / Eval AE-011 | Security review/eval | P0 |

任何 Req ID 不能只有“文档已写”作为完成证据；必须有可以检查的工件或测试记录。

---

## 16. 后续实施建议

优先以 **M0 → M1 → M2** 的顺序推进：先确定真实可用能力，再生成文档和契约，最后生成版本匹配的 Skill。这样不会把框架尚未实现的 API 反向固化到用户指南中。

如果后续计划让 Workorch 直接基于本规范执行开发，可以把第 15 章的 `Req ID` 视为 Goal Coverage Checklist，并使每个 Task 同时关联：`Req ID → 目标文件 → Contract ID → Test ID → Evidence → Verify Result`。规划时没有完成映射的需求应保留为未覆盖项，不能靠“Agent 自认完成”关闭。

本文批准后，应作为目标仓库的 `docs/specifications/documentation-agent-skills-spec.md`，在发布门禁全部通过前保持 `draft` / `Candidate` 状态。当前输入路径保留为规范入口，归档路径可用导航文件引用，避免维护两个独立副本。

---

## 附录 A：推荐质量检查工作流（示意）

```yaml
# .github/workflows/docs-skills.yml (conceptual example)
name: Docs and Skills
on:
  pull_request:
  push:
    branches: [main]
jobs:
  validate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - run: cargo fmt --all -- --check
      - run: cargo check --workspace --all-targets
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace
      - run: cargo test --doc --workspace
      - run: RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps
      # Add pinned doc/link/skills-ref validators when implemented.
```

> 注意：这是 workflow 结构示例；第三方 Action 与 Rust 工具链应在项目 CI 中按版本/commit pinning 策略进行审核。不代表此工作流现已部署或通过。

## 附录 B：外部来源与参考标准

以下来源只用于定义通用外部格式及技术能力；Modorch 自身 API/语义仍以仓库中的对应版本为准：

1. [Agent Skills — Specification](https://agentskills.io/specification)：Skill 目录、frontmatter、按需加载、参考文件和验证约定。
2. [OpenAI / Codex — Build Skills](https://developers.openai.com/codex/skills/)：Codex 生态 Skill 接入与工作方式（具体路径应以客户端当前版本官方文档为准）。
3. [ChatGPT Learn — Build Skills](https://learn.chatgpt.com/docs/build-skills)：Skill 主体、资源目录与工具使用指导。
4. [Rustdoc — Documentation tests](https://doc.rust-lang.org/rustdoc/write-documentation/documentation-tests.html)：可运行文档示例的测试机制。
5. [Cargo — The Cargo Book](https://doc.rust-lang.org/cargo/)：Workspace、依赖版本、Feature 与 Cargo 测试的官方参考。

---

**本规范的核心验收判断**：一个陌生用户或 AI Agent 在没有隐含历史上下文的环境下，能够识别适配版本、理解边界、只调用真实 API、实现符合契约的 Modorch 应用，并提交可复核的测试与证据；出现能力缺失、版本不匹配或验证失败时，能够准确阻断并说明原因。
