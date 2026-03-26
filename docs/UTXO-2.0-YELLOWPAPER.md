# UTXO Compute 技术黄皮书
## 去中心化计算操作系统：基于UTXO 2.0的通用计算网络

**版本 1.0**  
**2026年3月**

---

## 1. 引言

区块链技术自比特币诞生以来，主要沿着两条路径演进：以比特币为代表的UTXO模型和以以太坊为代表的账户模型。UTXO模型以其原子性、并行潜力和安全性著称，但在可编程性和状态管理方面受限。账户模型支持复杂智能合约，却面临全局状态竞争和串行执行瓶颈。随着去中心化应用对性能、隐私和互操作性要求的提高，我们需要一种融合两者优点且超越两者的新型架构。

UTXO Compute 重新定义了UTXO模型，将其从单纯的“未花费交易输出”扩展为**自包含的可验证计算单元**。每个UTXO不仅可以持有多种数字资源（主币、代币、数据、模型、计算任务），还可以附带可编程逻辑（`logic`）和演化状态（`state`）。通过这一设计，UTXO Compute构建了一个去中心化计算操作系统，能够以原生并行的方式执行通用计算任务，同时保持比特币级别的安全性与原子性。

本黄皮书详细阐述了UTXO Compute的核心概念、数据结构、交易处理、共识机制、代理调度、分层扩展、经济模型及实现规范，旨在为开发者提供完整的技术参考。

---

## 2. 核心概念

### 2.1 UTXO 2.0：通用资源单元

在UTXO Compute中，每个UTXO是一个**资源容器**，可以持有：

- **原生代币**（如链的Gas代币，主币）
- **多种同质化/非同质化资产**（自定义代币、NFT）
- **数据块**（文件、结构化数据）
- **代码**（可执行脚本、智能合约逻辑）
- **计算任务**（待执行的作业描述）
- **状态**（持续演化的持久化数据）

每个UTXO携带两个脚本：
- **`lock` 脚本**：定义谁能操作该UTXO（花费、更新等）。
- **`logic` 脚本**（可选）：定义UTXO的行为逻辑，在UTXO被操作时自动执行，实现智能合约功能。

UTXO通过`parents`和`children`字段形成状态演化链，支持多次更新而不被销毁。通过`resources`字段的`Ref`类型，UTXO可以引用其他UTXO，实现递归组合。

### 2.2 代理 UTXO

代理UTXO是一种特殊的UTXO，具备**自主执行能力**。除了标准字段外，它还包含：
- **调度策略**（`schedule`）：定义何时唤醒执行（定时、条件触发、事件驱动）。
- **目标函数**（`objective`）：用于评估代理绩效。
- **权限列表**（`permissions`）：允许的操作范围。
- **内部状态**（`state`）：持久化的记忆。
- **Gas预算**（`gas_budget`）：每次执行愿意支付的最大Gas。

代理UTXO由节点调度器定期唤醒，独立执行逻辑，并产生内部交易。它们可以代表用户自动执行策略、充当游戏NPC、管理资产等。

### 2.3 资源抽象

一切数字事物都被抽象为**资源**，以`ResourceMap`形式存储在UTXO中。每个资源由`AssetId`标识，值可以是：
- `Amount(u64)`：同质化资产数量。
- `Data(Vec<u8>)`：非结构化数据。
- `Ref(OutPoint)`：引用另一个UTXO。
- `RefBatch(Vec<OutPoint>)`：批量引用。

这种抽象使得资产、数据、引用在协议层统一处理，为跨域、跨链互操作奠定基础。

### 2.4 域与分层

UTXO Compute支持多域架构：
- **域0**：主链，负责最终结算、跨域锚定和治理。
- **域1..65535**：侧链或状态通道，每个域独立运行，可自定义共识参数。
- 每个UTXO的`domain`字段标识其所属域。

跨域交易通过主链驱动的两阶段提交实现原子性，资产可在域间安全转移。

---

## 3. 数据结构规范

所有数据结构使用确定性序列化格式（如bincode或自定义紧凑二进制），必须保证相同内容产生相同字节序列。

### 3.1 基础类型

```rust
type Hash = [u8; 32];          // Blake3哈希
type Ed25519PublicKey = [u8; 32];   // Ed25519公钥
type Ed25519Signature = [u8; 64];   // Ed25519签名
type AssetId = Hash;           // 资产标识符
type DomainId = u32;           // 域标识符，0为主链
```

### 3.2 资源值 (ResourceValue)

```rust
enum ResourceValue {
    Amount(u64),
    Data(Vec<u8>),
    Ref(Hash),                  // UTXO ID
    RefBatch(Vec<Hash>),
}
```

资源映射（`ResourceMap`）定义为有序向量，以保证序列化确定性：

```rust
type ResourceMap = Vec<(AssetId, ResourceValue)>;
```

### 3.3 脚本 (Script)

```rust
struct Script {
    vm: u8,                     // 虚拟机类型：0=BitcoinScript, 1=WASM
    code: Vec<u8>,
}
```

### 3.4 UTXO (Utxo)

```rust
struct Utxo {
    id: Hash,                   // 由(txid, output_index)或内容哈希生成
    version: u8,                // 协议版本
    domain: DomainId,            // 所属域

    resources: ResourceMap,      // 持有的资源集合
    lock: Script,                // 所有权脚本
    logic: Option<Script>,       // 行为逻辑（可选）
    state: Option<Vec<u8>>,      // 逻辑脚本的持久化状态

    parents: Vec<Hash>,          // 前驱UTXO ID列表（用于追溯）
    children: Vec<Hash>,         // 预声明的后继UTXO ID列表

    created_at: u64,             // 创建区块高度
    ttl: Option<u64>,            // 过期区块高度（None表示永不过期）
    flags: u32,                  // 特性标志（见3.4.1）

    extensions: Vec<(String, Vec<u8>)>, // 扩展字段
}
```

**字段说明**：
- `id`：通常由`(txid, output_index)`组合哈希生成，但若UTXO通过内容寻址（如静态数据UTXO），也可直接由内容哈希生成，需在`flags`中标记。
- `flags`：位标志定义：
  - `0x01`：可合并（允许与其他UTXO合并）
  - `0x02`：可分割（允许通过Split操作分割）
  - `0x04`：已冻结（因租金不足）
  - `0x08`：代理UTXO（启用调度）
  - `0x10`：通道UTXO（状态通道专用）
  - 其余位保留。

### 3.5 交易输入 (Input)

```rust
struct Input {
    prev_out: Hash,              // 引用的UTXO ID
    unlock: Script,              // 解锁脚本
    witness: Vec<u8>,            // 见证数据（签名等）
}
```

### 3.6 Compute 操作 (ComputeTx)

```rust
struct ComputeTx {
    inputs: Vec<Input>,
    outputs: Vec<Utxo>,
    fee: u64,                    // 支付给矿工的主币手续费
    nonce: Option<u64>,           // 防重放随机数（可选）
    metadata: Vec<(String, Vec<u8>)>, // 元数据（如跨域证明）
    domain: DomainId,              // 操作所在域（输入UTXO必须属于该域，跨域操作特殊处理）
}
```

Compute 操作的唯一标识`txid`由整个操作的序列化哈希计算得出。

### 3.7 区块头 (BlockHeader)

```rust
struct BlockHeader {
    version: u8,
    prev_hash: Hash,              // 父区块哈希
    merkle_root: Hash,            // 交易Merkle树根
    timestamp: u64,               // 区块时间戳（秒）
    height: u64,                  // 区块高度
    nonce: u64,                   // PoW随机数
    difficulty: u32,               // 当前难度目标
}
```

### 3.8 区块 (Block)

```rust
struct Block {
    header: BlockHeader,
    ops: Vec<ComputeTx>,           // Compute 操作列表
}
```

---

## 4. 交易验证规则

### 4.1 基本验证

节点收到 Compute 操作后，执行以下检查：
1. **输入存在性**：所有`prev_out`引用的UTXO必须存在于当前UTXO集中，且未被花费。
2. **域一致性**：所有输入的`domain`必须等于操作的`domain`（除非是跨域操作，见第8节）。
3. **解锁验证**：对每个输入，执行`unlock`脚本与对应UTXO的`lock`脚本。解锁成功条件：
   - 若`lock.vm`为0（Bitcoin Script），则按照比特币脚本规则验证栈执行结果是否为真。
   - 若`lock.vm`为1（WASM），则调用WASM运行时，传入`unlock`脚本和见证数据，返回布尔值。
4. **逻辑脚本执行**：如果输入UTXO包含`logic`脚本，则在验证通过后执行该脚本。执行环境包括：
   - 当前UTXO的完整数据（`resources`, `state`等）
   - 输入见证数据
   - 当前区块头信息（时间戳、高度、随机种子）
   - 输出列表（新UTXO的草稿）
  脚本必须返回`success`标志，并可修改`state`和输出UTXO的`resources`。若返回失败，整个交易失败。
5. **资源守恒**：除非`logic`脚本明确授权铸币或销毁（通过特殊指令或签名），否则所有输入UTXO中的资源总和（按资产ID汇总）必须等于输出UTXO中的资源总和加上`fee`（主币）。
6. **输出有效性**：
   - 新UTXO的`id`必须正确计算（通常由交易ID和输出索引组合哈希）。
   - `created_at`必须设为当前区块高度。
   - `ttl`不能小于当前高度。
   - `resources`中不能包含重复的资产ID。
7. **原子性**：所有步骤必须全部成功，否则整个操作无效，状态回滚。

### 4.2 特殊操作

UTXO Compute支持以下原子操作类型，通过交易结构隐式表达：
- **Spend**：消耗输入UTXO，产生新UTXO（传统消费）。
- **Update**：输入和输出引用同一个UTXO（通过`prev_out`指向自己），表示更新该UTXO的`state`或`resources`，但不销毁它。此时UTXO的`id`不变，但`parents`添加旧ID，`children`可选。
- **Split**：一个输入UTXO，多个输出UTXO，且输出UTXO的资源总和等于输入。通常用于拆分多资产UTXO。
- **Merge**：多个输入UTXO，一个输出UTXO，且输出资源等于输入总和。
- **Delegate**：通过`lock`脚本和特定见证数据，临时授权他人操作。

### 4.3 交易排序与依赖

区块中的交易必须按照拓扑顺序排列：如果交易B引用了交易A创建的UTXO，则A必须在B之前。矿工在打包时需构建依赖图。

---

## 5. 共识机制（PoW）

原型阶段采用简化工作量证明（PoW）共识，每10秒出一个区块。

- **难度调整**：每2016个区块调整一次，目标是维持10秒出块时间。
- **挖矿**：矿工收集交易，构造区块，计算`BlockHeader`哈希，要求小于当前难度目标。
- **最长链规则**：节点始终选择累积难度最大的链作为主链。

---

## 6. 代理UTXO调度器

调度器负责在每生成新区块前，找出待执行的代理UTXO，执行其逻辑，并生成内部交易加入区块。

### 6.1 调度策略表示

代理UTXO的调度策略存储在`extensions`中，键为`"schedule"`，值为以下结构的序列化：

```rust
enum Schedule {
    Interval(u32),                // 每N个区块执行一次
    AtBlock(u64),                 // 在指定区块高度执行
    Cron { modulus: u64, remainder: u64 }, // 区块高度模条件
    Trigger {                      // 条件触发
        utxo_id: Hash,             // 监控的UTXO
        field: String,              // 监控字段路径（如"state.price"）
        operator: u8,                // 0: ==, 1: >, 2: <, 3: changed
        threshold: Vec<u8>,          // 比较值（序列化）
    },
    Event {                         // 事件驱动
        event_type: u8,              // 0: NewUtxo, 1: Transfer, 2: ContractCall
        filter: Vec<u8>,              // 过滤条件（如合约地址）
    },
    Never,                          // 永不自动执行
}
```

代理UTXO的`flags`中必须设置`0x08`位（代理UTXO）才能被调度。

### 6.2 索引与唤醒

节点维护以下索引：
- **时间索引**：以`(next_block, agent_id)`为键，存储代理ID。每当代理执行或创建时，根据`schedule`计算下一次执行高度，插入索引。
- **触发索引**：对于依赖其他UTXO的代理，以被监控UTXO ID为键，存储依赖代理列表。当监控的UTXO被更新时，节点将这些代理加入待处理队列。
- **事件索引**：对于事件驱动的代理，维护布隆过滤器或倒排索引。

在区块生成前，调度器执行：
1. 从时间索引中获取所有`next_block <= current_height`的代理ID。
2. 从触发队列中获取满足条件的代理ID（需重新验证条件）。
3. 合并去重，得到待执行代理集合。

### 6.3 依赖分析与并行执行

- 每个代理执行时，读取自身UTXO和可能访问的其他UTXO（通过`permissions`或`state`中引用的UTXO）。这些依赖需在`extensions`中声明（键`"deps"`，值为UTXO ID列表）。
- 调度器构建依赖图，将无依赖的代理分配到不同线程并行执行。
- 执行结果（内部交易）收集后，按代理ID排序追加到区块交易列表。

### 6.4 执行环境

代理在WASM虚拟机中执行，可访问以下导入函数：
- `get_current_height() -> u64`
- `get_block_hash(height: u64) -> Hash`
- `get_utxo(id: Hash) -> Option<Utxo>`（需权限）
- `create_transaction(tx: Transaction) -> bool`（生成内部交易，但需满足权限和资源约束）
- `log(message: &str)`

执行时限、内存和Gas由节点强制执行。

### 6.5 Gas与费用

代理执行消耗的Gas从代理自己的主币资源中扣除。若Gas不足或执行失败，代理状态回滚，但已消耗的Gas仍需支付。连续失败的代理可能被暂停。

---

## 7. 存储与索引

节点使用RocksDB存储以下列族（Column Family）：

| 列族名 | 键 | 值 | 说明 |
|--------|----|----|------|
| `utxo` | UTXO ID | 序列化Utxo | 主UTXO集 |
| `tx` | 交易ID | 序列化Transaction | 已确认交易 |
| `block` | 区块高度 | 序列化Block | 区块数据 |
| `utxo_by_domain` | (domain, utxo_id) | 空 | 按域遍历UTXO |
| `agent_schedule` | (next_block, agent_id) | 空 | 时间索引 |
| `agent_deps` | 被监控UTXO ID | [agent_id] | 触发索引 |
| `metadata` | 常量键（如`"height"`） | 值 | 链状态 |

所有键使用大端序编码以保证顺序扫描。

---

## 8. 分层扩展与跨域原子交换

### 8.1 域注册

新域（侧链）需在主链上创建**域注册UTXO**，包含：
- `domain` ID
- 创世区块哈希
- 共识参数（出块时间、难度等）
- 锚定地址（用于锁定资产）

### 8.2 跨域交易

跨域交易是指输入UTXO来自不同域的交易。此类交易必须包含证明（如源链的Merkle证明），并经过主链协调的两阶段提交：

1. **准备阶段**：
   - 用户构造跨域交易，提交到主链。
   - 主链验证源域证明，锁定源域输入UTXO（标记为pending），并创建**票据UTXO**（包含交易哈希、目标域、目标地址）。
   - 票据UTXO广播给目标域节点。

2. **提交阶段**：
   - 目标域节点验证票据，在目标域创建输出UTXO。
   - 目标域向主链提交完成证明。
   - 主链收到证明后，最终删除源域输入，销毁票据。

若超时未收到完成证明，主链可解锁源域输入，回滚交易。

### 8.3 资产跨链转移

主链与侧链之间的资产转移采用**锁定-铸造/销毁-解锁**机制：
- **主链→侧链**：用户在主链锁定资产（创建锁定UTXO），提交证明到侧链，侧链铸造等值资产。
- **侧链→主链**：用户在侧链销毁资产，提交证明到主链，主链解锁对应资产。

---

## 9. 虚拟机与脚本执行

### 9.1 支持的虚拟机类型

| `vm`值 | 类型 | 说明 |
|--------|------|------|
| 0 | Bitcoin Script | 非图灵完备，用于简单条件 |
| 1 | WASM | 图灵完备，支持多语言编译 |
| 2 | 扩展虚拟机（预留） | 预留给未来迁移与扩展场景 |

### 9.2 WASM运行时

- 使用Wasmtime作为高性能WASM引擎。
- 提供导入函数集，封装链上操作。
- 每个合约实例独立沙箱，内存限制1MB，执行步数限制（通过Gas折算）。
- 支持预编译，提升执行效率。

### 9.3 Gas计量

每条WASM指令消耗固定Gas，复杂操作（如哈希计算）额外计费。Gas价格由市场动态决定。

---

## 10. 经济模型

### 10.1 原生代币（COMP）

- 用途：支付Gas费、存储租金、跨域费用。
- 总量：210亿，通过PoW挖矿产出，每4年减半。
- 初始分配：60%挖矿，20%生态基金，10%团队（4年线性解锁），10%早期贡献者。

### 10.2 存储租金

每个UTXO根据其字节大小和存活时间支付租金，租金率`rate`动态调整：

```
rent = size * rate * cycles
```

- `cycles`：自上次支付租金以来的计费周期数（每1000个区块为一个周期）。
- `rate`动态调整公式：

```
rate_new = rate_base * (1 + α * (utilization - target))
```

其中`utilization`为当前UTXO集总大小与目标上限之比，`target`通常设为70%。

租金从UTXO的主币资源中自动扣除。若余额不足，UTXO进入冻结状态，宽限期30天后可被矿工回收。

### 10.3 交易手续费

用户需支付`fee`（主币）给矿工，费用可自行设定，矿工按高费率优先打包。

### 10.4 代理执行费用

代理执行消耗的Gas从代理自身主币中扣除。代理可设置`gas_budget`，若预算不足，执行失败。

---

## 11. 开发工具与SDK

### 11.1 核心库（primitives）
提供数据结构定义、序列化、哈希、签名等基础功能。

### 11.2 虚拟机SDK（vm-sdk）
为WASM合约提供Rust SDK，包含链上交互的导入函数封装。示例：

```rust
use utxo_compute_sdk::*;

#[no_mangle]
pub fn execute() {
    let height = get_current_height();
    let balance = get_balance();
    // ... 业务逻辑
}
```

### 11.3 节点客户端
提供RPC接口供钱包和浏览器查询。

### 11.4 区块浏览器
可视化链上数据，支持UTXO追踪和代理监控。

---

## 12. 路线图

| 阶段 | 时间 | 里程碑 |
|------|------|--------|
| Alpha | Q3 2026 | 核心协议实现（UTXO基本操作、WASM集成、单节点） |
| 测试网 | Q4 2026 | 多节点P2P网络、调度器、代理UTXO、基础跨域支持 |
| 主网启动 | Q2 2027 | 主网上线、租金机制、跨链锚定 |
| 资源市场 | Q4 2027 | 资源UTXO市场、自治代理框架 |
| 意图层 | Q2 2028 | IntentScript语言、意图解析引擎 |
| 阶段5 | 2029+ | 去中心化计算操作系统、跨链资源路由 |

---

## 13. 总结

UTXO Compute通过重新设计UTXO模型，将区块链从单纯的账本升级为去中心化计算操作系统。其核心贡献在于：
- **UTXO 2.0**：通用资源单元，支持多资产、可编程逻辑、状态演化。
- **代理UTXO**：自主执行的链上智能体。
- **原生并行**：基于UTXO独立性的并行验证。
- **分层扩展**：主链+侧链+通道架构，突破物理极限。
- **统一资源抽象**：为跨链、跨域互操作奠定基础。

本黄皮书为开发者提供了完整的技术规范，任何团队均可依据此文档实现兼容节点。我们期待与社区共同构建这一新一代计算网络。

---

*文档结束*
