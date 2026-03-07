# ZeroChain 对标成熟公链差距清单与整改路线

> 日期：2026-03-07
> 目标：将当前“可联调”提升到“可大规模分布式部署、可运营、可审计”的工程成熟度。

## 0. 与成熟公链对比仍缺什么（高优先级）

1. 全功能节点发现与连接治理：已接入 `discv5` 事件流与 Kademlia 路由表，但仍缺跨公网大规模连通性与扰动回归。
2. 完整同步能力：已具备 `header-first + body + state sync` 请求链路，但快照导入导出、断点续传与重组恢复仍待完善。
3. Gossip 与传播面：交易/区块广播还未形成去重缓存、背压、速率限制、中继策略。
4. 共识与链选择工程化：缺少复杂分叉场景验证、长时稳定压测、跨节点一致性回归体系。
5. 执行层与兼容性：EVM 兼容矩阵、基准合约回归、跨版本状态兼容测试还不完整。
6. 生产级安全：缺少系统化 fuzz、依赖漏洞门禁、RPC 鉴权限流、密钥托管与轮换策略。
7. 运维可用性：缺少标准化部署模板（systemd/k8s）、SLO 告警、72h soak 与回滚演练。

## 1. 网络与共识层

- [x] N1. 基础 P2P 监听/入站连接/bootnode 出站连接
- [x] N2. `net_peerCount` 返回真实连接数
- [x] N3. 基础握手协议与 network_id 校验（防止跨网络误连）
- [x] N4. 完整节点发现协议（Kademlia/discv5 真正上线流量，不是简化 UDP 协议）
- [ ] N5. 完整 peer 生命周期（握手、心跳、超时、断线重连、黑名单持久化）
  - [x] N5-a. 已完成基础心跳（PING/PONG）+ 空闲超时剔除（防僵尸连接）
  - [x] N5-b. 已完成 bootnode 自动重连 + 黑名单持久化（基础版）
- [ ] N6. Gossip + 广播去重 + 背压（交易/区块传播）
  - [x] N6-a. 已完成交易/区块 hash 级别 gossip、去重缓存、每 peer 速率限制（基础版）
- [ ] N7. 链同步（header-first / block-body / catch-up / fork-choice）
  - [x] N7-a. 已完成最小同步状态机（Syncing/Recovering/Complete）与恢复骨架
  - [x] N7-b. 已完成真实 `GET_HEADERS/HEADERS`、`GET_BLOCK_BODY/BLOCK_BODY`、`GET_STATE_SNAPSHOT/STATE_SNAPSHOT` 链路
  - [ ] N7-c. fork-choice、断点续传、重组恢复
- [ ] N8. 多节点一致性测试（3/5/7 节点，断网重连后收敛）
  - [x] N8-a. 已提供 3 节点互联收敛 smoke 脚本
  - [ ] N8-b. 5/7 节点+网络抖动/分区恢复自动化

## 2. 执行与状态层

- [ ] E1. 状态同步与快照（snapshot/export/import）
- [ ] E2. 数据修剪（pruning）与归档模式区分
- [ ] E3. 状态恢复演练（宕机、磁盘损坏、回放）
- [ ] E4. EVM 兼容性回归（基础合约套件）
- [ ] E5. UTXO Compute 批量/高并发一致性验证

## 3. 交易池与出块层

- [ ] T1. 交易池替换/过期/限速策略完整化
- [ ] T2. 出块打包策略可配置（收益最大化 + 公平性）
- [ ] T3. 跨节点 mempool 传播一致性验证

## 4. RPC / API / 兼容性

- [x] R1. 基础 `net_*` / `eth_*` / `zero_*` 服务可用
- [ ] R2. 关键 RPC 语义与 Ethereum 客户端兼容矩阵
- [x] R3. 真实 peer 列表查询接口（`zero_peers`）
- [ ] R4. 限流/鉴权/审计日志（生产暴露接口必须）
  - [x] R4-a. 已完成 RPC token 鉴权 + 每客户端每分钟限流（基础版）
  - [ ] R4-b. 审计日志、细粒度方法级鉴权、配额分层

## 5. 安全与密钥管理

- [ ] S1. 依赖安全扫描与漏洞基线（`cargo audit` + CI）
- [ ] S2. 密钥托管、轮换、最小权限访问控制
- [ ] S3. P2P 输入数据 fuzz/边界测试（防崩溃）
- [ ] S4. DoS 保护（连接上限、每 IP 频控、消息体上限）
  - [x] S4-a. 已完成连接速率限制、每 IP 连接上限、gossip 速率限制
  - [ ] S4-b. SYN flood / 资源隔离 / 自适应封禁策略

## 6. 运维与发布

- [x] O1. release gate + full-chain e2e 已自动化
- [ ] O2. 24h/72h soak test 自动化
- [ ] O3. SLO/告警（错误率、延迟、同步滞后、peer 波动）
- [ ] O4. 一键部署（systemd/docker-compose/k8s）
- [ ] O5. 回滚演练（RTO/RPO 量化）

## 7. 本轮已整改（含本次新增）

1. `net_peerCount` 改为实时值（不再写死 `0x0`）。
2. `discovery.start()` 升级为真实 `discv5` 服务（ENR + Kademlia 路由 + 事件流查询循环）。
3. `start_listening()` 改为真实 TCP listener + accept。
4. `PeerManager::add_peer()` 改为真实入表、去重、计数更新。
5. CLI `run` 增加 P2P 参数：
   - `--p2p-listen-addr`
   - `--p2p-listen-port`
   - `--bootnode`（可重复）
   - `--max-peers`
   - `--disable-discovery`
   - `--disable-sync`
6. README 路线图与多节点启动示例更新。
7. 新增基础握手协议：`ZERO/1 <network_id> <peer_id>`，连接时强制 network_id 校验。
8. 新增 `zero_peers` RPC：可返回真实 peer 元数据（连接时间、最近活跃、idle 秒数等）。
9. 新增基础 peer 心跳：`ZERO/PING` / `ZERO/PONG`，并在空闲超时后自动清理连接。
10. 新增 3 节点 P2P 收敛脚本：`scripts/p2p_three_node_smoke.sh`。
11. 新增 P2P 治理能力：每 IP 连接上限、连接频控、gossip 频控、黑名单持久化、bootnode 自动重连。
12. 新增真实同步协议流：`GET_HEADERS/HEADERS`、`GET_BLOCK_BODY/BLOCK_BODY`、`GET_STATE_SNAPSHOT/STATE_SNAPSHOT` 三阶段校验与推进。
13. 新增 RPC 安全基线：静态 token 鉴权（Bearer / `x-zero-token`）+ 每客户端每分钟限流。

## 8. 下一批建议立即整改（按优先级）

1. P0：完成 `N7-c`（fork-choice、断点续传、重组恢复）并补充分叉对抗测试。
2. P0：补 `R4-b`（审计日志、方法级鉴权、租户配额）。
3. P1：推进 `N8-b` 多节点扰动测试（网络分区、重连收敛、一致性断言）。
4. P1：补 `S1/S3`（`cargo audit` CI 门禁 + P2P fuzz 测试）。
5. P1：VPS 一键部署（systemd + healthcheck + journald + metrics）。
