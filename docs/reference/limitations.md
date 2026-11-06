# 限制与非目标

适用 0.0.3；状态及证据以[能力矩阵](capability-matrix.md)为准。

- 只承诺单进程、单 Application 内受管通信。无持久化队列、跨进程 Transport、exactly-once、跨资源事务和动态插件 ABI。
- 请求超时不是回滚；Restart 不重放消息。技术 retryable 不等于业务幂等。
- Concurrent 不保序，框架的 Request 环检测不推断业务锁；同组件使用保守等待域。
- Dynamic 的工厂同步调用且须轻量，期限不能中断正在执行的工厂；空闲回收后 Handler 私有副本可丢失。
- payload_bytes 是声明成本，预算不测量 Any 的深堆大小、不限制调用者未轮询 Future、业务缓存及保留回复。
- Snapshot 不承诺跨字段同一时刻一致；Handler 延迟桶不是端到端延迟。精确标签有容量与字符串预算。
- cancel/force 不证明同步闭包或外部副作用已退出；StopIncomplete 保留资源，需要宿主维持 Runtime 并决定重试/退出。
- 没有全局 Runtime、信号、Subscriber、Exporter、配置路径或业务鉴权。Registry 依赖限制不构成不可信插件沙箱。
- 当前未正式发布 crates.io / docs.rs，也未以本地 Linux 通过推定其他 OS 或远程 CI 通过。多宿主 Skill 自动发现必须逐版本验证。
