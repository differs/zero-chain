# P0/P1/P2 执行状态（2026-03-07）

## P0 协议闭环（进行中）

已完成：
- 交易信封合法性检查已进入验证路径：`fee/nonce/metadata`
- 生命周期字段校验已进入验证路径：`ttl/rent_reserve/flags`
- `ResourcePolicy` 已从 no-op 升级为守恒 + 类型约束校验
- `lock` 脚本执行入口已接入授权阶段（最小表达式子集）
- 新增一致性测试：
  - 重放字段开启但缺 nonce
  - metadata 重复键
  - ttl 但缺 deadline
  - 资源增发（守恒失败）
  - lock 脚本失败

待完成：
- Nonce 全局去重/窗口策略（当前仅做结构与上下文合法性）
- lock/unlock 更完整指令集与 VM 隔离执行器
- 资源策略从“守恒最小集”扩展到跨域票据/复杂引用约束

## P1 状态与同步（已启动）

已完成：
- `SyncStateSnapshot` 增加 `state_proof` 字段
- 新增可插拔 `StateProofVerifier` 接口
- 默认 verifier 已接入同步流程（state root + proof 双重校验）
- 新增 `SyncCheckpoint` 导入/导出接口，支持重启恢复测试

待完成：
- 接入真实 header/body/state provider（替换 synthetic 数据源）
- proof 结构升级为可验证 Merkle/Trie 证明
- 空节点追平与断点恢复端到端测试（多节点）

## P2 网络与传播（已启动）

已完成：
- 广播路径加入背压处理：发送通道阻塞/拥塞的对端将被摘除，避免拖慢整体 gossip
- 保持既有去重、限速、banlist 机制联动

待完成：
- gossip 背压策略细化（分级降速而非直接摘除）
- 交易/区块传播指标与告警阈值（可观测性闭环）
- 多节点压测脚本与稳定性基线
