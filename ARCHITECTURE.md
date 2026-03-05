# ZeroChain 架构设计文档

## 1. 系统概述

ZeroChain 是一个新一代区块链系统，具有以下核心特性：

- **混合账户模型**: 结合余额模型和 UTXO 模型的优势
- **EVM 兼容**: 完全兼容以太坊虚拟机
- **PoW 共识**: 抗 ASIC 的工作量证明机制
- **账户抽象**: 内置智能合约钱包支持

## 2. 核心架构

### 2.1 分层架构

```
┌─────────────────────────────────────────────────┐
│              应用层 (Application)                │
│  Wallet | DApps | Explorer | Dashboard          │
├─────────────────────────────────────────────────┤
│              API 层 (API/RPC)                    │
│  JSON-RPC | REST API | WebSocket                │
├─────────────────────────────────────────────────┤
│            核心协议层 (Core Protocol)             │
│  Account | EVM | Consensus | TxPool | State    │
├─────────────────────────────────────────────────┤
│              网络层 (Network)                    │
│  P2P | Sync | Discovery | Propagation          │
├─────────────────────────────────────────────────┤
│              存储层 (Storage)                    │
│  MPT Trie | LevelDB | UTXO DB | Index          │
└─────────────────────────────────────────────────┘
```

### 2.2 模块依赖关系

```
zerocli (CLI/Node)
    ├── zeroapi (RPC/REST/WS)
    │   └── zerocore (Core Protocol)
    │       ├── account (账户管理)
    │       ├── evm (EVM 引擎)
    │       ├── consensus (共识机制)
    │       ├── transaction (交易处理)
    │       ├── block (区块管理)
    │       └── state (状态机)
    └── zeronet (P2P 网络)
        └── zerostore (存储层)
```

## 3. 账户模型设计

### 3.1 混合账户模型

```
┌─────────────────────────────────────────────────┐
│            ZeroChain 账户类型                    │
├─────────────────────────────────────────────────┤
│  1. 外部拥有账户 (EOA)                           │
│     - 公私钥控制                                 │
│     - 支持多签名                                 │
│     - 社交恢复                                   │
├─────────────────────────────────────────────────┤
│  2. 智能合约账户                                 │
│     - 代码控制                                   │
│     - 可升级合约                                 │
│     - 状态存储                                   │
├─────────────────────────────────────────────────┤
│  3. 账户抽象账户                                 │
│     - 验证器合约                                 │
│     - 多所有者                                   │
│     - 阈值签名                                   │
├─────────────────────────────────────────────────┤
│  4. 多签账户                                     │
│     - 多个签名者                                 │
│     - 可配置阈值                                 │
│     - 每日限额                                   │
└─────────────────────────────────────────────────┘
```

### 3.2 账户数据结构

```rust
pub struct Account {
    pub address: Address,           // 20 字节地址
    pub account_type: AccountType,  // 账户类型
    pub balance: U256,              // 余额
    pub nonce: u64,                 // 交易计数器
    pub storage_root: Hash,         // 存储 Trie 根
    pub code_hash: Hash,            // 合约代码哈希
    pub utxo_refs: Vec<UtxoRef>,    // UTXO 引用 (可选)
    pub config: AccountConfig,      // 配置
    pub state: AccountState,        // 状态机状态
}
```

## 4. EVM 兼容层

### 4.1 执行引擎架构

```
Solidity/Vyper 合约
       ↓
   EVM Bytecode
       ↓
┌──────────────────────────────┐
│      EVM 执行引擎             │
│  ┌────────┐ ┌────────┐       │
│  │解释器   │ │JIT(可选)│      │
│  └────────┘ └────────┘       │
│  ┌────────────────────────┐  │
│  │   预编译合约            │  │
│  │ - ECDSA 恢复            │  │
│  │ - SHA256/RIPEMD160     │  │
│  │ - 账户抽象验证 (0x100)   │  │
│  │ - UTXO 验证 (0x101)     │  │
│  │ - 批量转账 (0x102)      │  │
│  └────────────────────────┘  │
└──────────────────────────────┘
       ↓
   State Interface
       ↓
   ZeroChain 存储层
```

### 4.2 自定义预编译合约

| 地址 | 名称 | 功能 | Gas |
|------|------|------|-----|
| 0x01-0x09 | 标准预编译 | ECDSA, SHA256 等 | 标准 |
| 0x100 | AccountValidator | 账户抽象签名验证 | 5000+ |
| 0x101 | UtxoValidator | UTXO 花费验证 | 10000+ |
| 0x102 | BatchTransfer | 批量转账优化 | 21000+ |

## 5. 共识机制

### 5.1 PoW 算法

```
┌─────────────────────────────────────────────────┐
│           ZeroChain PoW 共识                     │
├─────────────────────────────────────────────────┤
│  算法选择:                                       │
│  ├── RandomX (CPU 友好，默认)                    │
│  ├── ProgPoW (GPU 友好，可选)                   │
│  └── LightHash (轻量验证)                        │
├─────────────────────────────────────────────────┤
│  难度调整:                                       │
│  - 每区块调整                                    │
│  - 目标出块时间：10 秒                            │
│  - EMA 算法                                      │
│  - 最大调整幅度：±10%                           │
├─────────────────────────────────────────────────┤
│  区块奖励:                                       │
│  - 初始奖励：5 ZC                               │
│  - 最小奖励：2 ZC                               │
│  - 减半周期：2,100,000 区块 (~4 年)              │
│  - 叔父奖励：93.75%                             │
└─────────────────────────────────────────────────┘
```

### 5.2 难度调整公式

```rust
fn calculate_difficulty(parent: &BlockHeader, timestamp: u64) -> U256 {
    let actual_time = timestamp - parent.timestamp;
    let target_time = 10;  // 秒
    
    let ratio = if actual_time < target_time {
        (target_time / actual_time).min(1.1)
    } else {
        (actual_time / target_time).max(0.9)
    };
    
    parent.difficulty * ratio
}
```

## 6. 存储架构

### 6.1 数据分层

```
┌─────────────────────────────────────────────────┐
│              存储引擎层                          │
│  LevelDB | RocksDB | Redb (可选)                │
├─────────────────────────────────────────────────┤
│              数据结构层                          │
│  MPT Trie | Merkle Tree | LRU Cache            │
├─────────────────────────────────────────────────┤
│              数据抽象层                          │
│  StateDB | BlockDB | TxIndexDB | UtxoDB        │
└─────────────────────────────────────────────────┘
```

### 6.2 键空间设计

```rust
pub mod db_keys {
    pub const BLOCK_PREFIX: &[u8] = b"b";
    pub const BLOCK_HASH_PREFIX: &[u8] = b"bh";
    pub const TX_PREFIX: &[u8] = b"t";
    pub const TX_HASH_PREFIX: &[u8] = b"th";
    pub const ACCOUNT_PREFIX: &[u8] = b"a";
    pub const CODE_PREFIX: &[u8] = b"c";
    pub const TRIE_NODE_PREFIX: &[u8] = b"n";
    pub const UTXO_PREFIX: &[u8] = b"u";
    pub const META_PREFIX: &[u8] = b"m";
}
```

## 7. API 设计

### 7.1 JSON-RPC 方法

#### 标准 Ethereum 方法
- `eth_blockNumber` - 获取最新区块号
- `eth_getBalance` - 获取账户余额
- `eth_sendRawTransaction` - 发送交易
- `eth_call` - 执行合约调用
- `eth_getLogs` - 获取日志

#### ZeroChain 扩展方法
- `zero_getAccount` - 获取账户详情
- `zero_getUtxos` - 获取 UTXO 列表
- `zero_getHybridBalance` - 获取混合余额

### 7.2 WebSocket 订阅

```rust
// 订阅新区块
ws.subscribe("newHeads")

// 订阅新交易
ws.subscribe("newPendingTransactions")

// 订阅日志
ws.subscribe("logs", {
    "address": "0x...",
    "topics": ["0x..."]
})
```

## 8. 性能优化

### 8.1 并行执行

```
交易批次
    ↓
依赖图构建
    ↓
拓扑排序分组
    ↓
并行执行每组
    ↓
结果合并
```

### 8.2 缓存策略

| 缓存类型 | 大小 | TTL |
|---------|------|-----|
| 账户缓存 | 100,000 | 1 小时 |
| 代码缓存 | 10,000 | 永久 |
| 存储缓存 | 1,000,000 | 1 小时 |
| 交易缓存 | 100,000 | 10 分钟 |
| 区块缓存 | 1,000 | 永久 |

### 8.3 网络优化

- **紧凑区块传播**: 短 ID + 交易恢复
- **交易广播优化**: 避免重复广播
- **节点发现**: Kademlia DHT

## 9. 安全考虑

### 9.1 密码学安全

- secp256k1 签名 (Ethereum 兼容)
- Keccak256 哈希
- 支持 Ed25519 和 BLS (可选)

### 9.2 重放保护

- Chain ID (EIP-155)
- Nonce 机制
- 交易哈希唯一性

### 9.3 权限控制

- 账户权限级别
- 操作白名单/黑名单
- 多签要求配置

## 10. 监控和指标

### 10.1 Prometheus 指标

```
# 区块指标
zerocchain_block_height
zerocchain_block_time_seconds
zerocchain_block_size_bytes

# 交易指标
zerocchain_tx_pool_size
zerocchain_tx_throughput
zerocchain_tx_execution_time_seconds

# 网络指标
zerocchain_peer_count
zerocchain_network_bytes_sent
zerocchain_network_bytes_received

# 共识指标
zerocchain_mining_hashrate
zerocchain_difficulty
```

## 11. 扩展路线图

### Phase 1: 基础功能 (已完成)
- [x] 核心协议设计
- [x] 账户模型实现
- [x] EVM 引擎
- [x] PoW 共识

### Phase 2: 网络层
- [ ] P2P 协议实现
- [ ] 区块同步
- [ ] 交易广播

### Phase 3: 性能优化
- [ ] 并行执行
- [ ] 状态过期
- [ ] 快照同步

### Phase 4: 高级功能
- [ ] 分片支持
- [ ] 跨链桥接
- [ ] 隐私交易

## 12. 参考实现

- Ethereum: https://github.com/ethereum
- Bitcoin: https://github.com/bitcoin
- Polkadot: https://github.com/paritytech/polkadot
- Cosmos SDK: https://github.com/cosmos/cosmos-sdk
