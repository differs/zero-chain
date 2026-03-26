# ZeroChain 架构

## 1. 目标

ZeroChain 采用原生 UTXO Compute 路径，统一执行、状态与资源表达，默认签名方案为 `ed25519`。

## 2. 分层结构

```text
┌──────────────────────────────────────────────┐
│ Clients: Wallet / CLI / Explorer / Mining   │
├──────────────────────────────────────────────┤
│ API: HTTP JSON-RPC + WebSocket + REST       │
│      Methods: zero_clientVersion /          │
│               zero_keccak256 / net_* / zero_* │
├──────────────────────────────────────────────┤
│ Core: UTXO Compute / Tx Pool / Block Import │
│       Account / Domain / Object / Policy    │
├──────────────────────────────────────────────┤
│ Storage & Network: StateDB / ComputeStore   │
│                    P2P / PoW / Mining        │
└──────────────────────────────────────────────┘
```

## 3. 交易与执行

- Canonical 写路径：`zero_submitComputeTx`
- 查询路径：`zero_getComputeTxResult`、`zero_getObject`、`zero_getOutput`
- 账户查询：`zero_getAccount`、`zero_getUtxos`
- 区块与挖矿：`zero_getLatestBlock`、`zero_getWork`、`zero_submitWork`

## 4. 签名与地址

- 默认签名：`ed25519`
- 地址格式：`ZER0x...`
- Witness 以原生签名结构提交，按阈值规则进行验证。

## 5. WebSocket

- 订阅：`zero_subscribe`
- 取消订阅：`zero_unsubscribe`
- 推送事件：`zero_subscription`

## 6. 运行时约束

- 默认配置对外暴露 ZeroChain RPC 方法集与网络探针方法。
- 对象状态变更由 Compute 执行器与策略层共同约束。
- 生产配置建议启用鉴权、限流与观测指标。
