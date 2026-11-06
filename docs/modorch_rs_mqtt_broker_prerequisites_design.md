# Modorch-RS 面向可嵌入 MQTT Broker 的框架前置调整设计

> 文档状态：评审后实施规范，**代码与本地门禁已完成，Candidate**｜目标：Modorch-RS 0.0.3 Candidate｜核对日期：2026-11-06  
> 仓库：<https://github.com/muidea/modorch-rs>  
> 核对基线：`master` commit `84f22cddd46be1cf5f20023bfec201d1f1fe280d`  
> 关联文档：`modorch_mqtt_broker_full_design.md`（未随仓库提供，不作为本次实现或验收依据）  
> 本文仅定义 **Modorch-RS 框架本身**应实施的改动；不在 Modorch-RS 中实现 MQTT Broker、MQTT 编解码、QoS 或 Session。

## 1. 背景、目标与决策

### 1.1 已确认需求

1. 一个 Rust 服务器在集成 `modorch-rs` 与 `modorch-mqtt` 时，**只有一个 Application 和一个底层 EventHub 实例**。所有 Module 通过各自的 Context 访问同一 EventHub；允许 clone 句柄，禁止另建 Hub。
2. `modorch-mqtt` 是独立的、可 Cargo 依赖的 Broker 库；核心 MQTT 通信由 **普通 Module** 承担，可选 Auth/Storage/Admin/Metrics/Bridge 由其他 Module 承担。
3. Topic Name / Topic Filter 使用 MQTT 3.1.1/5.0 标准规则，不恢复 magicCommon 的 `:id` 通配或中间 `#`。
4. 同一 Application 既运行业务 Module，也运行 MQTT Core Module；宿主拥有 Tokio Runtime、配置、信号与退出流程。
5. Module 对外监听的启动时机必须位于 **整个 Application 的就绪屏障之后**；失败后要能检查式清理。重建对受影响依赖闭包执行相同 ready → activate 顺序，不自动恢复其他已停止组件；应用可按既有规则回到 Degraded。

### 1.2 实施结论

**本次已实施（P0）**：为普通 `Module` 增加可选 `activate_ingress` 生命周期 Hook，并让初始启动、监督重建调用同一分发路径；补齐失败补偿和验收。  
**暂不需要实施**：新增 EventHub 实例、修改 ServiceRegistry 授权、修改标准 Topic 匹配、为模块新增独立 Runtime、在 EventHub 实现 MQTT QoS、Session 或网络层。

如果不修改框架，也可以由专用 Initiator 激活 Listener；但这违背“仅启用 MQTT Core Module 即能提供基础 Broker 服务”的首要集成形态。故推荐通用 Hook 扩展。

## 2. 当前仓库能力核查（基线）

| 维度 | 当前源码证据 | 结论 |
|---|---|---|
| 单 Application / EventHub | `modorch-runtime/src/lib.rs::Inner::context()` 将 `self.hub.clone()` 放入 `ComponentContext.event_hub` | 已满足；clone 指向同一底层 Hub |
| Module 注册 | `ApplicationBuilder::register_module` / `ComponentSelection::enable_module` | 已满足；注册不等于启用 |
| 生命周期 | `modorch-runtime/src/component.rs::Lifecycle` 有 setup/start/ready/begin_shutdown/quiesce/teardown | 已满足基本框架 |
| 入口激活屏障 | `Application::initialize()` 在所有 ready 之后 `tracker.open()`，再调用 Initiator::activate_ingress | **普通 Module 不支持** |
| 重建入口激活 | `modorch-runtime/src/supervision.rs` 仅对 `Instance::Initiator` 调用 `activate_ingress` | **普通 Module 不支持** |
| 受管任务 | `ComponentContext.tasks: TaskScheduler`，`TaskScheduler::spawn` 接收 `CancellationToken` | 已满足基础需求；高连接数预算需显式配置 |
| EventHub 外部入口 | `EventHub::external()` 为网络请求建立新的根准入上下文 | 已满足；应由 MQTT 使用 |
| 标准 Topic | `modorch-eventhub/src/topic.rs` 校验 Name/Filter、空层、`$`、`+/#`，`types.rs::topic_matches` 使用同一实现 | 0.0.2 已收口；无需再次改匹配规则 |
| EventHub 投递 | `PublishReport` 为每目标**准入**，不是 Handler 完成；组件级去重 | 保留，不改变 MQTT QoS 语义 |
| 服务所有权 | `Initiator::export_services`，Module 无 ServicePublisher | 保留；扩展 Module 默认通过 EventHub 协作 |
| 失败/清理 | 已有 StopIncomplete、重试、代际监督机制 | 需将 Module Ingress 纳入相同契约 |

源码链接（均按上述 commit 固定，避免后续 master 漂移）：

- [`component.rs`](https://github.com/muidea/modorch-rs/blob/84f22cddd46be1cf5f20023bfec201d1f1fe280d/crates/modorch-runtime/src/component.rs)
- [`runtime/lib.rs`](https://github.com/muidea/modorch-rs/blob/84f22cddd46be1cf5f20023bfec201d1f1fe280d/crates/modorch-runtime/src/lib.rs)
- [`supervision.rs`](https://github.com/muidea/modorch-rs/blob/84f22cddd46be1cf5f20023bfec201d1f1fe280d/crates/modorch-runtime/src/supervision.rs)
- [`scheduler/lib.rs`](https://github.com/muidea/modorch-rs/blob/84f22cddd46be1cf5f20023bfec201d1f1fe280d/crates/modorch-scheduler/src/lib.rs)
- [`eventhub/topic.rs`](https://github.com/muidea/modorch-rs/blob/84f22cddd46be1cf5f20023bfec201d1f1fe280d/crates/modorch-eventhub/src/topic.rs)
- [`component-contract.md`](https://github.com/muidea/modorch-rs/blob/84f22cddd46be1cf5f20023bfec201d1f1fe280d/contracts/component-contract.md)
- [`lifecycle-contract.md`](https://github.com/muidea/modorch-rs/blob/84f22cddd46be1cf5f20023bfec201d1f1fe280d/contracts/lifecycle-contract.md)

## 3. 架构边界（不变）

```text
Host Tokio Runtime
  └── Application (唯一，宿主拥有)
      ├── EventHub (唯一，Application 拥有)
      ├── Scheduler / OperationTracker / ServiceRegistry
      ├── Business Module A ─┐
      ├── Business Module B ─┼─> 同一个 EventHub 共享句柄
      ├── MQTT Core Module ──┤
      └── MQTT Extension Modules ─┘
```

- `EventHub` 的 `Arc`/句柄共享不意味着可以任意重置或销毁底层实例。
- 同一进程可以创建**多个彼此独立** Application；这里的一致性约束仅针对**同一个 Application**。
- 不引入全局静态 Hub、全局可变 Registry 或嵌套 Application。
- EventHub 的 Topic 与 Destination 决定接收者，LaneKey 只决定执行域。MQTT 客户端 Session 不是框架组件。
- EventHub 无跨进程、持久化或 MQTT QoS 保证，这些由后续库处理。

## 4. P0：统一外部入口激活生命周期

### 4.1 设计动机与时序

当前启动过程：Factory.create → setup（创建资源）→ handlers 安装 → start → ready（全体完成）→ tracker.open → **仅 Initiator** activate_ingress → Application Ready。

推荐：将入口激活提升为 `Instance` 上的**通用可选能力**，不改变 Initiator/Module 两种框架角色。

```text
[所有选中组件] create + setup + install endpoints
                ↓
[所有选中组件] start
                ↓
[所有选中组件] ready
                ↓
OperationTracker.open (开启根准入)
                ↓
[所有选中组件] activate_ingress（按计划顺序）
                ↓
ApplicationState::Ready
```

为避免错误宣称已 Ready，规定：所有入口激活返回成功前，Application 不发布 Ready；Ingress 激活中失败应进入已有的启动失败补偿路径。

### 4.2 公共 Trait 的加法变更（已实施）

修改 `crates/modorch-runtime/src/component.rs`：

```rust
#[async_trait]
pub trait Module: Lifecycle {
    fn descriptor(&self) -> ComponentDescriptor;

    fn handlers(&mut self) -> FrameworkResult<Option<HandlerRegistration>> {
        Ok(None)
    }

    /// 所有组件 ready 之后，统一开启对外新请求入口。
    /// 不应长期阻塞；Listener 的长期任务交给 ctx.tasks。
    async fn activate_ingress(
        &mut self,
        _ctx: &RuntimeContext,
    ) -> FrameworkResult<()> {
        Ok(())
    }
}
```

设计要点：

- 默认为无操作：不修改任何既有 Module 的业务逻辑。
- 不改变 `Lifecycle`，不允许 Module 通过 `export_services` 发布服务。
- 将 `Instance` 的入口激活作为统一分发，降低初始启动与监督重建产生分叉的风险：

```rust
impl Instance {
    pub async fn activate_ingress(
        &mut self,
        ctx: &RuntimeContext,
    ) -> FrameworkResult<()> {
        match self {
            Self::Initiator(v) => v.activate_ingress(ctx).await,
            Self::Module(v) => v.activate_ingress(ctx).await,
        }
    }
}
```

以上补丁已在本开发树实施；已发布 v0.0.2 tag 不含此 API 增强，消费者须匹配本轮框架指纹。

### 4.3 初次启动改动

修改 `crates/modorch-runtime/src/lib.rs::Application::initialize()`：

1. 继续完成所有 `setup` / `start` / `ready`。
2. 继续在所有 ready 通过后调用 `tracker.open()`。
3. 替换 `if let Instance::Initiator(...)` 的特殊分支，调用 `entry.instance.activate_ingress(&ctx)`。
4. 继续用 `checked_hook(Phase::ActivateIngress, ...)` 记录失败主因。
5. 任何失败不标记 `ApplicationState::Ready`；复用 `StartupError` 及可重试清理流程。

**不要**在 `Module::start()` 或 `ready()` 内绕过激活屏障直接开始接受 TCP 连接。本次采用 TCP 层屏障：setup 可用 `TcpSocket::bind` 预留端口，但不得 `listen`；仅在 activate_ingress 中调用 `listen` 或 `TcpListener::bind`。延后 `accept` 不足以阻止 TCP 握手，不属于未激活 Listener。失败启动后的 socket 必须检查式释放。框架不能拦截用户任意网络调用，此限制由组件契约、示例与真实 TCP 测试验证。对于端口占用或宿主启动失败，原始 FrameworkError 应在 `source` 中可诊断；现有框架错误不支持任意 std::io::Error 类型，网络示例保留其错误文本。

### 4.4 监督重建改动

修改 `crates/modorch-runtime/src/supervision.rs::recover()`：

1. 沿用关闭受影响 Owner / 依赖闭包、排空旧任务与租约、递增 Generation、重建组件、重新安装 Handler 的当前机制。
2. `rebuild` 中完成全体受影响组件 `start/ready` 后，Owner 的根准入重新打开。
3. 对受影响所有 `Instance` 逐个调用统一 `activate_ingress`，不仅 Initiator；每次调用记录 ActivateIngress 阶段及原始错误。
4. 将 rebuild、open_owners 和全部 activate_ingress 纳入同一次恢复尝试的结果；激活失败也进入现有失败补偿分支，重新关闭准入、停止任务、排空工作/租约、移除 Handler 后 teardown。补偿未完成不得推进代际或开始下一次尝试；全部激活成功后才统一标记受影响组件 Ready。
5. 旧 Generation 的受管任务须实际退出；旧 TaskScheduler、ServiceReader/ServiceHandle 失效。应用级 EventHub/ExternalEventHub 不绑定生产者代际，也不随重建失效；不得把其存活等同于旧 Listener 仍可接单。组件以受管任务取消、实际退出和 socket 所有权阻止旧入口存活；本次不新增入口代际能力。重新绑定失败消耗现有累计重启预算，补偿成功后在剩余预算内重试，耗尽转 FailFast。

### 4.5 生命周期幂等与清理契约

| 阶段 | 入口组件要求 |
|---|---|
| setup | 创建必要的本地资源；可 bind 未 listen 的 TcpSocket；不得监听或接受外部请求 |
| start | 准备 Handler、会话状态、受管任务依赖；不得开启外部新入口 |
| ready | 检查资源、配置及依赖就绪；不可宣称全局 Ready |
| activate_ingress | 激活 Listener/accept loop；快速返回；成功后才允许对外访问 |
| begin_shutdown | 立刻停止接受新连接；拒绝新根事件；通知在途连接关闭 |
| quiesce | 等待生产者退出与协议级有界排空；保留可重试状态 |
| teardown | 只有工作/租约完成后才释放 socket、session/store 等资源 |

- Hook 必须快速返回，长期等待交给受管 Task；沿用现有生命周期 Hook 预算边界，框架不为激活新增强制超时，不允许以 timeout-drop 丢弃资源所有权。调用者取消 start 的等待不会取消启动事务。
- 激活失败后必须允许进入 `begin_shutdown/quiesce/teardown`，包括部分成功（部分端口已激活）情形。
- 同一 Generation 的 Hook 不应重复激活同一 Listener；若未来新增可重复激活 API，须先定义幂等 Key。
- `shutdown_checked` 超时只能报告 `StopIncomplete`，不得销毁仍被使用的资源；`force_shutdown` 不能声称正常完成 MQTT 交付。
- 受管 Task 返回非取消错误时，应按既有失败上报/监督链处理；若任务正常退出但 Listener 不应退出，由 MQTT Module 明确将意外退出升级为故障，不可静默“Ready 但端口已死”。

## 5. 不需要更改的框架功能

### 5.1 EventHub 唯一实例

`ComponentContext.event_hub = self.hub.clone()` 已提供共享句柄。`ApplicationHandle::event_hub()` 也返回同一 Application 所有的 Hub 句柄；`modorch-mqtt` 不得调用 `EventHub::new`。增加集成测试确认同一业务 Module 与 MQTT Module 的订阅/发布互通即可。

### 5.2 MQTT Topic 标准

0.0.2 已实现：UTF-8 非 NUL 字符串、字节长度上限、`+` 可匹配空层、尾部 `#` 零层匹配、根通配不匹配 `$` 前缀、`:id` 为普通字符。保持 `topic_matches`、订阅预解析和实际 `RouteIndex` 行为一致。MQTT 5.0 的 `$share/{group}/{filter}` **共享订阅是协议特例**，不能仅借 EventHub 的普通 TopicFilter 完成组内单目标派发；由 `modorch-mqtt` 专门实现。

### 5.3 消息背压和交付语义

保留 `PublishReport` 的逐目标准入、非原子 fanout、无订阅者返回 `Unroutable`。MQTT Core 不应把 `PublishReport.accepted()` 解释成 PUBACK 或外部客户端收到消息，也不要求 EventHub 提供消息持久化。

### 5.4 Module / Initiator 服务边界

保留 `Initiator::export_services` 发布窄共享服务、Module 无发布权限。MQTT 扩展优先通过 EventHub 的 Event/Command/Request 与 MQTT Core 交互；需要直接可借用资源时再使用符合现有 Registry 契约的 Initiator，而不是放开 Module 权限。

### 5.5 Scheduler

`ctx.tasks.spawn` 已将任务绑定 Owner/Generation，带 CancellationToken。MQTT 先按该模型实现 Listener 与 Connection 任务，配置宿主 `ApplicationBuilder::task_limits` 并限制最大连接数。高连接数场景若触发 Task 配额/跟踪性能瓶颈，属于**后续有测量数据支持的优化**，不是本次 P0 前置改造。

## 6. 代码调整清单

| 优先级 | 文件 | 操作 |
|---|---|---|
| P0 | `crates/modorch-runtime/src/component.rs` | 在 Module 增加默认 `activate_ingress`；在 Instance 增加统一转发 |
| P0 | `crates/modorch-runtime/src/lib.rs` | 启动阶段对所有组件执行统一入口激活 |
| P0 | `crates/modorch-runtime/src/supervision.rs` | 监督重建时使用相同的入口激活机制 |
| P0 | `crates/modorch/tests/ingress.rs` | 加入全局 Ready 屏障、激活失败补偿、单 Application/Hub 共享测试 |
| P0 | `crates/modorch/tests/ingress.rs`（及既有 supervision 回归） | 加入 Module 激活后重启/旧代际失效/端口恢复测试 |
| P0 | `contracts/lifecycle-contract.md` | 新增生命周期规范及 Test ID |
| P0 | `contracts/component-contract.md` | 明确 Module 可作为网络入口组件，不增加第三类角色 |
| P0 | `docs/guides/module-development.md` / `docs/architecture/runtime.md` | 提供使用方法、时序、错误语义 |
| P0 | `docs/reference/capability-matrix.md` / `tests/contracts/mapping.json` | 增加规范映射和可验证声明 |
| P0 | `.agents/skills/**` 及打包工具 | 更新匹配当前版本指纹的维护/开发 Skill、示例和预检 |
| P0 | `CHANGELOG.md` / release notes / migration | 写明新增 Hook 的兼容性边界与迁移说明 |

本次不建议修改 `modorch-eventhub/src/topic.rs`、`modorch-registry` 和 `modorch-core` 的核心类型。

## 7. 新增契约建议

| 建议 Contract ID | MUST 保证 | 失败表现 | 验收测试 |
|---|---|---|---|
| LIFE-INGRESS-001 | Module 和 Initiator 均在所有组件 Ready 后才能 activate | 启动拒绝、StartupError | CT-LIFE-INGRESS-001 |
| LIFE-INGRESS-002 | ActivateIngress 失败不可进入 Ready，必须受控补偿 | 保留原错误和清理结果 | CT-LIFE-INGRESS-002 |
| LIFE-INGRESS-003 | 组件重建恢复对应 Ingress，旧受管任务退出且旧任务/服务能力失效 | 监督失败按既有策略报错 | CT-LIFE-INGRESS-003 |
| LIFE-INGRESS-004 | 关闭时先停止生产、再 Drain、再 Teardown | 超时 StopIncomplete 不假装释放 | CT-LIFE-INGRESS-004 |
| LIFE-INGRESS-005 | 同一 Application 的组件 Context 共享 Hub，多个 Application 相互隔离 | Context 互通及跨 Application 隔离断言失败 | CT-LIFE-INGRESS-005 |

新增规范需复核与现有 LIFE/COMP/MSGRULE 编号，避免正式并入时 ID 冲突。

## 8. 必须覆盖的验收用例

1. **普通 Module 入口在 Ready 后才启动**：两个 Module + 一个 Initiator；一个 Module 的 ready 通过后仍不得接单，直到其他全部 ready；通过真实 TCP connect 验证激活前不可连接，同时验证 Initiator 与 Module。
2. **构建零副作用**：`builder.build()` 不 bind 端口、不 spawn 监听任务。
3. **激活成功**：对外可建立 TCP 连接；健康状态为 Ready。
4. **激活失败**：先有其他入口成功、后一个入口激活失败；不能残留端口，返回主错误+清理结果。
5. **可重试关闭**：慢连接导致 StopIncomplete 时资源不得提早释放，随后可继续检查式退出。
6. **受管任务取消**：Listener 与 Connection Task 在停止时按预算释放，无 orphan task。
7. **监督重启**：Module 意外故障、依赖闭包重建、旧 TaskScheduler/服务能力失效，保留的 ExternalEventHub 在恢复后仍可使用，正常情况下重新开放端口；在重建 ready 屏障处验证 TCP 尚不可连接。增加“入口 A 已开放、入口 B 重建激活失败”的成功重试及预算耗尽测试，验证清理先于下一代、阶段错误可诊断、无端口残留。
8. **EventHub 共享**：两个业务 Module 和 MQTT 模拟 Module 收到同一 Hub 的测试事件；对多个 Application 反向测试互相隔离。
9. **兼容回归**：旧 Module 不实现新 Hook，现有示例、契约测试结果不回退。
10. **任务限额**：达到连接/任务预算时明确拒绝或背压，不出现失管任务。

### 推荐实际运行命令

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo doc --workspace --all-features --no-deps --locked
cargo check -p modorch --all-targets --locked
```

还应运行仓库现有 Contract/Agent Skill 检查脚本、针对启动和重建的真实 TCP 集成测试，以及构建时声明的 MSRV 兼容测试。**本轮已执行的完整命令、退出码和日志见[验收报告](validation/2026-11-06/module-ingress/README.md)；不能将本地通过扩大为跨平台或独立 Agent 通过。**

## 9. 实施工作包与验收门禁

| 工作包 | 前置 | 交付物 | 退出条件 |
|---|---|---|---|
| FR-01 Trait + Instance | 无 | 通用 Hook 代码 | 无行为破坏、旧 Module 编译通过 |
| FR-02 Startup | FR-01 | 单一激活分发 | Ready 屏障/失败补偿测试通过 |
| FR-03 Supervision | FR-02 | 代际重建入口恢复 | 故障重建/旧连接测试通过 |
| FR-04 Contracts & Docs | FR-01–03 | 契约、示例、能力矩阵和 Skill | 文档与源码指纹一致 |
| FR-05 Release Gate | FR-01–04 | 有时间和平台边界的验证报告 | 全部 MUST 用例有真实执行证据 |

## 10. 禁止事项与非目标

- 禁止在框架核心加入 `MqttSession`、`QosState`、`PacketCodec`、`ClientSubscription` 等 MQTT 专属类型。
- 禁止为 Broker 创建第二 EventHub、隐藏 Application 或全局 Tokio Runtime。
- 禁止将 Module 扩张为可任意发布共享服务，破坏 Initiator 所有权模型。
- 禁止把入口 `activate_ingress` 作为常驻 accept loop；常驻工作必须提交受管 Task。
- 禁止由 EventHub 准入结果推导 MQTT 交付确认、可靠持久化或 exactly-once。
- 不在本次实现动态模块热加载、多节点集群或跨进程 EventHub。

## 11. 最终完成定义（DoD）

所有 P0 文件修改完成，Module/Initiator 对外入口均遵守全局 Ready→Activate 顺序；初始化失败、重启和停止具备真实回归证据；旧组件不需改代码便可继续工作；共享 EventHub 和 Topic 语义不退化；契约/文档/Skills/测试矩阵一致后，才可标记本调整完成。

---

**说明**：本文件先对照固定基线完成评审，再在工作区实施。代码尚未提交或发布，最新 API 以本轮源码/Rustdoc 和指纹为准。

## 12. 评审收口决策（2026-11-06）

1. 激活纳入单次重建尝试及补偿，不再直接从成功分支提前返回错误。
2. 入口屏障采用 TCP 层语义，预绑定使用 TcpSocket；不以延迟 accept 冒充未监听。
3. 旧代际保证限定任务与服务能力，应用级 Hub 保持有效；不新增全局 Hub 或入口能力类型。
4. Skill revision/快照、迁移、示例和执行证据均为本次完成门禁。框架包版本在发布阶段统一为 0.0.3，使用 Skill 1.0.7 的新指纹 Bundle 匹配，不覆盖已发布 Bundle。独立 Agent 未执行时保持 Candidate。

## 13. 实施与证据映射

| 工作包 | 本轮交付 | 证据 |
|---|---|---|
| FR-01 | Module 默认 Hook、Instance 转发；既有 Module 无需覆盖 | workspace check/test、MSRV 1.85 check |
| FR-02 | 全体 ready 后激活；启动错误/panic/取消等待/配额失败补偿 | ingress.rs 的屏障、补偿、绑定与配额测试 |
| FR-03 | 激活纳入重建尝试；排空后重试，耗尽 FailFast | ingress.rs 的恢复、慢连接、预算测试及既有 supervision.rs |
| FR-04 | LIFE-INGRESS-001—005、指南、迁移、HTTP Module 示例、Contract 1.2.0 / Skill 1.0.6 | tests/contracts/mapping.json、文档与 Skill 门禁 |
| FR-05 | 完整本地验证和干净目录分发回放通过；独立 Agent NotRun | [本轮验收](validation/2026-11-06/module-ingress/README.md) |

本轮不实现 Broker/QoS/Session，不对高连接数内存或吞吐作未测量承诺；缺实际 Agent 证据维持 Candidate。

发布归档：本增强纳入 [0.0.3 Candidate](releases/0.0.3.md)，发布验证见 [验收](validation/2026-11-06/release-0.0.3/README.md)；第 2 节仍保留固定 84f22cd 的 0.0.2 基线核对。
