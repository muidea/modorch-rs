# Modorch-RS EventHub — MQTT Topic 标准化调整设计

> 文档状态：实施规范 / 已落地；实际通过范围见[验收报告](validation/2026-11-04/mqtt-topic-alignment/README.md)  
> 版本：1.0 · 2026-11-04  
> 目标仓库：[muidea/modorch-rs](https://github.com/muidea/modorch-rs)（`master`，以本次源码核查为基线）  
> 适用架构：Modorch-RS 0.0.2 开发期 Candidate / Rust 2024 / MSRV Rust 1.85  
> 变更范围：`modorch-eventhub` 的 Topic 解析、匹配、索引、订阅、消息入站校验及 Contracts/文档/Skills  
> **独立交付要求**：本文件可独立指导 Modorch-RS 改造，不依赖另一份 Go 设计文档。

## 1. 目标与强制架构决策

### 1.1 设计目标

Modorch-RS 目前的 `EventHub` 已提供 Topic/Destination 路由、Event/Command/Request、订阅控制面、Serial/Partitioned/Concurrent/Dynamic 执行域、LaneKey、有界消息准入和受控关闭。**保留全部现有架构，精确修复 Topic 语义，使 EventHub 的 Topic Name / Topic Filter 与 MQTT 3.1.1、MQTT 5.0 标准一致**，为后续可选 MQTT Core Module 提供无歧义的基础接口。

### 1.2 决策 MR-TOPIC-ADR-001

1. MQTT Topic 规则是 EventHub 公开 Topic API 的唯一正式规则；不长期保留 Legacy Matcher。
2. **Topic Name ≠ Topic Filter**。发布使用 Topic Name，订阅注册使用 Topic Filter；API、校验与错误分别定义。
3. `+`、尾部 `#` 严格遵循 MQTT；`:id` 不再有通配含义，作为普通字符串层级处理；中间 `#` 拒绝。
4. `topic_matches()`、`RouteIndex.matching()`、订阅校验必须共享同一个匹配语义（推荐同一解析器/编译结果），不能只让公共测试正确而实际路由不同。
5. **Topic 与 Destination 正交**：Topic 负责发布订阅匹配，Destination 负责额外目标约束，LaneKey 只负责执行顺序域；不得把 MQTT Topic 匹配规则扩展成目标/请求语义。
6. 本次调整不得破坏既有 Event/Command/Request 准入含义、SubscriptionId、去重、WaitGraph、背压、执行策略、ServiceRegistry 权限和组件生命周期。

### 1.3 非目标

本阶段不实现 MQTT 网络 Broker、CONNECT、QoS、Session、Retained、共享订阅分组、跨进程 Transport、消息持久化或 exactly-once；**不**开放普通 Module 的 ServicePublisher 权限，也不要求重新设计 Initiator/Module 角色。

## 2. 调整前的仓库与源码事实

以下记录调整前的基线（ccc77ce），用于解释差异；当前实现与执行结果见验收报告。来源（设计时静态审查）：

- [`crates/modorch-eventhub/src/types.rs`](https://github.com/muidea/modorch-rs/blob/master/crates/modorch-eventhub/src/types.rs)：`validate_pattern()`、`topic_matches()`、`MessageEnvelope`、`SubscriptionSpec`。
- [`crates/modorch-eventhub/src/routing.rs`](https://github.com/muidea/modorch-rs/blob/master/crates/modorch-eventhub/src/routing.rs)：`RouteIndex` 的 exact/wildcard 两种索引和内联匹配。
- [`crates/modorch-eventhub/src/lib.rs`](https://github.com/muidea/modorch-rs/blob/master/crates/modorch-eventhub/src/lib.rs)：`subscribe`、`publish`、`command`、`request`、`ExternalEventHub` 及路由准入。
- [`crates/modorch-eventhub/tests/concurrency.rs`](https://github.com/muidea/modorch-rs/blob/master/crates/modorch-eventhub/tests/concurrency.rs)：现有模式测试及并发/路由回归。
- [`contracts/messaging-contract.md`](https://github.com/muidea/modorch-rs/blob/master/contracts/messaging-contract.md)、[`docs/message-semantics.md`](https://github.com/muidea/modorch-rs/blob/master/docs/message-semantics.md)、[`docs/reference/capability-matrix.md`](https://github.com/muidea/modorch-rs/blob/master/docs/reference/capability-matrix.md)。

调整前已确认的行为：

- `validate_pattern()` 拒绝空层；要求 `#` 只能位于最后一层，`+` 必须独占层级；但允许 `:parameter` 作为扩展语义。
- `topic_matches()` 把 `segment.starts_with(':')` 当作单层通配，拒绝空层；遇到 `#` 直接返回 true，函数自身未严格验证 Filter 合法性。
- `RouteIndex.matching()` 也有独立的 segment 匹配代码；其 `value.is_empty()` 与 `segment.starts_with(':')` 需要同步修正。
- `RouteIndex` 按 `s.starts_with(':')` 分类 wildcard 索引，标准化后 **`:id` 应进入 exact 索引**；保留现有基于 Component 的路由约束和发布目标去重。
- `publish` 的 `PublishReport` 是**逐目标入队准入报告**，不是业务完成证明；Command 和 Request 仍有既有零目标/歧义/超时语义。
- `Destination::Named`、`Destination::Component`、`Destination::Broadcast` 与 `DestinationPattern` 已类型化，不必复制 Go 旧版双向 Destination 模式。

| 差异 ID | 场景 | MQTT 标准预期 | 调整前 Rust 实现 | 处理 |
|---|---|---|---|---|
| MR-D01 | `a/+` → `a/` | match | 拒绝空层 | 必修 |
| MR-D02 | `a/#` → `a` | match | 已匹配 | 保留回归 |
| MR-D03 | `a/#/b` | invalid Filter | 注册校验拒绝，但直接 `topic_matches` 缺少校验 | 统一 |
| MR-D04 | `a/:id` → `a/42` | false | true | 必修 |
| MR-D05 | `#` → `$SYS/status` | false | true | 必修 |
| MR-D06 | `a//b` 订阅 | valid | 拒绝 | 必修 |
| MR-D07 | `:id` 索引 | literal/exact | 走 wildcard index | 必修 |
| MR-D08 | 不合法 Topic Name 作为 publish topic | reject | 未有完整 MQTT Topic 校验契约 | 必修 |

## 3. 唯一 MQTT Topic 语义

规范来源：

- [OASIS MQTT 3.1.1 §4.7、§1.5.3](https://docs.oasis-open.org/mqtt/mqtt/v3.1.1/mqtt-v3.1.1.html)
- [OASIS MQTT 5.0 §4.7](https://docs.oasis-open.org/mqtt/mqtt/v5.0/mqtt-v5.0.html)

### 3.1 Topic Name

- 至少一个字符；有效 UTF-8；编码长度 **≤65,535 bytes**；不得含 U+0000、`+` 或 `#`。
- `/` 和空层均合法，包括 `a/`、`/a`、`a//b`、`/`；大小写敏感、不做 Unicode 归一化。
- `:id` 是普通文本，并非占位符。Rust `&str` 本身有效 UTF-8，仍须检查 NUL、字符/字节长度和发布非法 wildcard。
- MQTT 协议合法性与 Modorch 本地资源上限分离：目前 `HubLimits.max_topic_bytes` 默认 1024 字节；**标准合法但超过本地上限**应报明确资源限制，不得说它违反 MQTT 规范。

### 3.2 Topic Filter

- 非空、有效 UTF-8、≤65,535 字节、不含 U+0000。
- `+` 独占一个层级，匹配恰好一层（包括空层）。例如 `a/+` 匹配 `a/`，不匹配 `a`。
- `#` 只能独占最后一层或独立使用，匹配零层或多层；`a/#` 匹配 `a`、`a/`、`a/b`。
- 以 `+` 或 `#` 起始的 Filter 不得匹配以 `$` 起始的 Topic Name；`$SYS/#` 匹配 `$SYS/status`。
- `:id` 只是普通文本；`a/:id` 对 `a/42` 返回 false。
- `a/#/b`、`a+`、`a/+x`、`a/b#` 统一拒绝；不能通过任何公开 matcher、索引或订阅路径绕过。
- `$share/{group}/{filter}` 属于 MQTT 5 Shared Subscription 协议能力，不由基础 EventHub 解释为共享消费组，也不作为本阶段已经支持的功能。

## 4. 目标 API 与数据模型

优先选择对现有 API 改动最小的落地方式；下面的代码是**建议设计示例，不是当前已存在的公开 API**：

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopicName(String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopicFilter {
    raw: String,
    levels: Vec<FilterLevel>, // private, prevalidated
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FilterLevel { Literal(String), Single, Multi }

impl TopicName {
    pub fn parse(raw: &str) -> Result<Self, TopicError> { /* ... */ }
}
impl TopicFilter {
    pub fn parse(raw: &str) -> Result<Self, TopicError> { /* ... */ }
    pub fn matches(&self, topic: &TopicName) -> bool { /* ... */ }
}

// Existing helper: keep name, but delegate to the same parser/matcher.
pub fn topic_matches(pattern: &str, topic: &str) -> bool { /* ... */ }
```

设计约束：

1. 如果继续保留 `Topic = String`，也必须在 `subscribe`、`publish`、`command`、`request` 和所有 external/wait 入口按类型分别校验；**不能**要求用户仅靠约定正确。
2. `TopicFilter::parse` 保留空层、不进行 trim；`topic_matches` 对非法 Filter 或非法 Topic Name 返回 false；`subscribe` 对非法 Filter 返回 `FrameworkError::InvalidConfig`（或新增兼容的可辨错误类别）。
3. `TopicFilter` 预编译用于 `RouteIndex`；所有匹配委托单一实现；`RouteIndex` 不得再维护一个与 `topic_matches` 不同的 if/else 匹配算法。
4. 不能将 `DestinationPattern`、`LaneKey` 或消息头视为 Topic 解析的一部分；一个 Filter 的匹配结果只能由 Filter 与 Topic Name 决定。
5. `SubscriptionSpec.topic_pattern` 的公开字符串形态可继续保留，构建/订阅时编译为内部 Filter 以减少破坏性 API 面积。

## 5. 改动明细与责任文件

### MR-CHG-01：单一解析器与标准 matcher（P0）

建议新增 `crates/modorch-eventhub/src/topic.rs`，用于校验 Topic Name、解析/编译 Topic Filter 和匹配。按 Rust 2024 / MSRV 1.85 实现，禁止不必要 `unsafe`，并为公开方法补 Rustdoc。

核心算法：

```text
TopicFilter::parse(raw):
  require nonempty, valid UTF-8 string, <=65535 bytes, no NUL
  split on '/', preserve empty levels
  each level:
    if exactly '+': Single
    else if exactly '#' and at last position: Multi
    else if contains '+' or '#': error
    else: Literal(level)

TopicFilter::matches(topic):
  TopicName::parse(topic)
  if topic begins '$' and filter begins '+' or '#': false
  loop through compiled levels:
    Literal: exactly equal topic level
    Single: consume exactly one level, including empty
    Multi: consume zero or more remaining levels, succeed
  otherwise require exact exhaustion of both sequences
```

### MR-CHG-02：`types.rs` API 收口（P0）

- `validate_pattern` 改为内部编译器委托，允许空层，拒绝中间 `#`；`:id` 转为普通文本。
- `topic_matches()` 委托权威匹配器；对无效输入不触发 panic、不返回误判 true。
- `MessageEnvelope.topic`、`SubscriptionSpec.topic_pattern` 等类型不必立即破坏性重构；先用严格边界校验并文档化含义。
- 修改 crate-level Rustdoc 示例，移除 `orders/:id/changed` 的通配演示。

### MR-CHG-03：`routing.rs` 的 RouteIndex（P0）

- 订阅安装时编译 Filter，仅 `+` 与合法末尾 `#` 进入 wildcard index；`:id` 层应被视为字面量，不应导致 wildcard 分类。
- Exact 索引、按 ComponentRef 查找和去重维持既有行为；wildcard 分支消费编译 Token，而非自己判断字符串。
- 保持订阅变更/路由准入线性化，增删后不遗漏索引清理；`RouteIndex.matching` 与 `topic_matches` 对同样输入**必须同结果**。
- 对同一组件多个命中 Filter 的发布仍只入队一次，这是现有组件级去重合同；不能将内部订阅直接当成 MQTT 客户端 Session 的 Subscription。
- 若使用预编译 Filter 缓存，防止订阅删改、组件重启/代际变化造成过期路由。

### MR-CHG-04：Hub 入站校验与错误语义（P0）

- `EventHub::subscribe` 校验 Filter；无效订阅不得修改现有 registry/generation。
- `EventHub::publish`、`command`、`request` 及各自 `ExternalEventHub`/`*_wait` 入口必须拒绝非法 Topic Name；尤其拒绝包含 `+/#` 的发布。
- 保留 `PublishReport.deliveries` “逐目标准入而非完成”、部分失败、Backpressure、Unroutable 语义；禁止把改变 Topic 匹配误解为改变业务交付成功含义。
- `HubLimits.max_topic_bytes` 仍可限制资源；在标准解析与本地限制不相等时需提供稳定区分的错误信息，更新配置/文档。

### MR-CHG-05：契约、Skill 和示例同步（P0/P1）

需要修改或同步：

```text
crates/modorch-eventhub/src/topic.rs               (建议新增)
crates/modorch-eventhub/src/types.rs
crates/modorch-eventhub/src/routing.rs
crates/modorch-eventhub/src/lib.rs
crates/modorch-eventhub/tests/concurrency.rs
crates/modorch-eventhub/tests/topic_conformance.rs    (建议新增)
contracts/messaging-contract.md
contracts/compatibility-contract.md
docs/message-semantics.md
docs/architecture/eventhub.md
docs/reference/capability-matrix.md
docs/public-api.md
docs/compatibility.md
tests/contracts/mapping.json
examples/event-messaging/src/main.rs
.agents/skills/modorch-app-development/**
.agents/skills/modorch-framework-maintenance/**
.agents/skills/modorch-module-development/**
```

适配工作不必修改无关组件源代码，但必须通过 workspace 测试、rustdoc 和 Agent Bundle 快照/指纹验证确保不会发生契约漂移。

## 6. 迁移与向后兼容策略

这是一项**可观察语义的破坏性变更**，尤其对于订阅和安全边界；Modorch-RS 当前处于 0.0.2 Candidate，允许设计期收口，但不能假装无兼容影响。

- `orders/:id/changed` -> `orders/+/changed`；在 Handler 中读取实际 Topic 的第二层，不再由 Matcher 提取参数名。
- 非标准 `#` 用法拒绝；如果有既有消费者依赖中间 `#`，迁移为合法 Filter + Handler 业务过滤或改 Topic 树。
- `orders/#` 的匹配范围包括 `orders`；`orders/+` 包括 `orders/`，要求新增“匹配集合扩大”审计，防止业务处理不当或 ACL 范围增加。
- 公共 `topic_matches` 已被使用的行为断言、文档、Skill 示例需要同步变更；Legacy 语义不作为长期稳定 API。
- 由于 `contracts/compatibility-contract.md` 要求 API/语义变化更新 Rustdoc、示例、Contract、版本/Skill 快照及验证证据，交付不得仅提交两个源文件。

## 7. 必须单独通过的 conformance 向量

| ID | Filter | Topic Name | 预期 |
|---|---|---|---|
| MR-T01 | `device/+` | `device/123` | true |
| MR-T02 | `device/+` | `device/` | true |
| MR-T03 | `device/+` | `device` | false |
| MR-T04 | `device/#` | `device` | true |
| MR-T05 | `device/#` | `device/` | true |
| MR-T06 | `device/#` | `device/a/b` | true |
| MR-T07 | `device/#/status` | `device/a/status` | invalid Filter |
| MR-T08 | `device/:id` | `device/42` | false |
| MR-T09 | `device/:id` | `device/:id` | true |
| MR-T10 | `#` | `$SYS/status` | false |
| MR-T11 | `+/status` | `$SYS/status` | false |
| MR-T12 | `$SYS/#` | `$SYS/status` | true |
| MR-T13 | `device//status` | `device//status` | true |
| MR-T14 | `/` | `/` | true |
| MR-T15 | `a/+` | `a/b/c` | false |
| MR-T16 | `a/b` | `A/b` | false |
| MR-T17 | `a+` | `a+` | invalid Filter（同时 Topic 非法） |
| MR-T18 | `a/+x` | `a/x` | invalid Filter |
| MR-T19 | `a/#` | `a/#` | invalid Topic Name |
| MR-T20 | `+/+` | `/finance` | true |

额外约束：非法 UTF-8 对 Rust `&str` 输入无法出现，但 MQTT wire bytes 层未来必须先解码验证；U+0000、UTF-8 字节长度 65,535/65,536、配置上限、Unicode、大小写、前后斜线、exact/wildcard index 一致性、非法 Filter 零副作用必须覆盖。

**三重一致性**：对于所有合法向量，公共 `topic_matches`、`RouteIndex.matching` 和经 `EventHub::subscribe/publish` 的真实投递结果一致（后者还受 Destination、组件可用性和准入限制影响，测试中固定这些条件）。

## 8. 非 Topic 回归保护

本调整不会将 MQTT QoS/Session 强行写入 EventHub，也不能破坏这些既有合同：

| 原 Contract | 保持内容 |
|---|---|
| `MSG-EVT-001` | fanout 逐目标入队报告；部分失败不原子 |
| `MSG-CMD-001` | Command 仅证明接受，唯一目标 |
| `MSG-REQ-004` | 已运行 Request 超时为 Indeterminate |
| `MSG-EXEC-001` | Serial FIFO、Partitioned / Concurrent 执行域 |
| `MSG-LANE-001/002` | Dynamic / 固定 shard、LaneKey 不影响接收方 |
| `MSG-BOUND-001` | 有界、进程内、非持久 |
| `MSG-ROUTE-001` | 订阅变更生效、订阅控制面线性化 |
| `MSG-WAIT-001` | 等待图、环检测和消息准入限制 |
| `MSG-REPLY-001` | Request 回复所有权与资源释放 |
| `COMP-001`、`RES-001` | Initiator/Module 两角色；Registry 仅 Initiator 发布 Service |

强调：Topic 的名称与匹配标准统一，并**不**代表 Modorch EventHub 获得 MQTT 网络通信能力；后续 MQTT Core Module 独立提供协议处理，使用 EventHub 内部事件总线，并管理独立的 QoS/Session/Retained 语义。

## 9. 验收合同、执行与证据

建议新增至 `contracts/messaging-contract.md`：

| Contract ID | 条款 | 测试 |
|---|---|---|
| `MSG-TOPIC-001` | Topic Name/Filter 分离，UTF-8、字节上限、非法输入拒绝 | MR-T01—T20 + 边界 |
| `MSG-TOPIC-002` | `+` 空层、`#` 零层、`$` 和 `:id` 字面量 | `topic_conformance.rs` |
| `MSG-TOPIC-003` | Matcher 与 RouteIndex、实际投递同结果 | 同一 fixture 三路径 |
| `MSG-TOPIC-004` | Subscribe/Unsubscribe 线性化、错误无副作用、路由无陈旧命中 | 并发竞争测试 |
| `MSG-TOPIC-005` | Destination/Lane/背压/请求语义零回归 | `concurrency.rs` + 应用 E2E |
| `MSG-TOPIC-006` | 文档、Skill、能力矩阵、证据一致 | contract/doc/skill gate |

建议执行（实际通过情况由实施者记录）：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test -p modorch-eventhub --locked
cargo test --workspace --all-features --locked
cargo doc --workspace --all-features --no-deps --locked
```

额外运行项目已有 `tools/doc-validation` 和 Agent Skill/Bundle 校验入口；以项目现存 CI/脚本的真实参数为准。在最终证据中记录 commit、Rust 工具链、平台、命令、退出码、测试数量和失败列表。**不允许用“测试文件已存在”代替“测试已通过”。**

## 10. 实施工作包与完成定义

1. **MR-01 规范/风险冻结**：确认 MQTT 3.1.1/5.0 Topic 语义、公共 fixture、查找全部 `:id` 使用点、现有性能基线。
2. **MR-02 标准 Topic 内核**：新增/重构解析器、校验、匹配；实现无效输入拒绝。
3. **MR-03 RouteIndex 收口**：复用编译器、取消 `:id` wildcard 分类、保证订阅删除清理、固定索引与枚举结果一致。
4. **MR-04 Hub 入站与兼容性**：统一订阅/发布/API 校验，保持消息合同、背压和 Lane；修复受影响的 tests/examples。
5. **MR-05 文档/Skill/版本收口**：更新 Contracts、mapping、Rustdoc、能力矩阵、迁移说明、Agent Skill/Bundle。
6. **MR-06 全量验证**：标准 fixture、集成、并发、性能、workspace、工具链和文档门禁均有证据。

**完成定义（Definition of Done）**：所有合法/非法 Topic 向量符合 MQTT 标准，公共 helper 与实际路由零分歧，原消息和生命周期合同无回归，破坏性变更迁移已完成，Contract/Skill/版本说明与实测证据齐全。只有以上工作完成后，才可把 `MQTT Topic Alignment` 标记为 Verified；本文件保留设计约束与调整前事实，当前实施与通过范围以[验收报告](validation/2026-11-04/mqtt-topic-alignment/README.md)为准；独立 Agent 发布门禁单独评估。
