# ZeroChain API（Native-Only）

## Endpoints

- HTTP JSON-RPC: `http://127.0.0.1:8545`
- WebSocket: `ws://127.0.0.1:8546`

## 方法分组

- 基础：`web3_*`、`net_*`
- 原生扩展：`zero_*`

## 常用 RPC 示例

### 1) 查询账户

```bash
curl -X POST http://127.0.0.1:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"zero_getAccount","params":["ZER0x526Dc404e751C7d52F6fFF75d563d8D0857C94E9"],"id":1}'
```

### 2) 查询最新区块

```bash
curl -X POST http://127.0.0.1:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"zero_getLatestBlock","params":[],"id":1}'
```

### 3) 领取矿工工作

```bash
curl -X POST http://127.0.0.1:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"zero_getWork","params":[],"id":1}'
```

### 4) 提交矿工工作

```bash
curl -X POST http://127.0.0.1:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"zero_submitWork","params":[{"work_id":"work-...","nonce":7,"hash_hex":"0x...","miner":"miner-1"}],"id":1}'
```

### 5) 提交 Compute 交易

```bash
curl -X POST http://127.0.0.1:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"zero_submitComputeTx","params":[{"tx_id":"0x...","domain_id":0,"command":"Transfer","input_set":[],"output_proposals":[],"witness":{"signatures":[{"scheme":"ed25519","public_key":"0x...","signature":"0x..."}]}}],"id":1}'
```

### 6) 查询 Compute 结果

```bash
curl -X POST http://127.0.0.1:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"zero_getComputeTxResult","params":["0x..."],"id":1}'
```

### 7) 简化余额转账（测试用途）

```bash
curl -X POST http://127.0.0.1:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"zero_transfer","params":[{"from":"ZER0x...","to":"ZER0x...","value":"0x64"}],"id":1}'
```

## WebSocket 订阅

### 建立订阅

```json
{"jsonrpc":"2.0","method":"zero_subscribe","params":["newHeads"],"id":1}
```

### 取消订阅

```json
{"jsonrpc":"2.0","method":"zero_unsubscribe","params":["<subscription-id>"],"id":2}
```

### 事件推送格式

- `method`: `zero_subscription`
- `params.subscription`: 订阅 ID
- `params.result`: 事件内容

## 错误码

| code | 含义 |
|---|---|
| -32700 | Parse error |
| -32600 | Invalid Request |
| -32601 | Method not found |
| -32602 | Invalid params |
| -32603 | Internal error |
