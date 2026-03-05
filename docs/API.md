# ZeroChain API 文档

## JSON-RPC 端点

- HTTP: `http://localhost:8545`
- WebSocket: `ws://localhost:8546`

## 标准 Ethereum 方法

### web3_*

#### web3_clientVersion

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"web3_clientVersion","id":1}'
```

响应:
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "ZeroChain/v0.1.0/linux/rustc1.75"
}
```

#### web3_sha3

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "method":"web3_sha3",
    "params":["0x68656c6c6f20776f726c64"],
    "id":1
  }'
```

### eth_*

#### eth_blockNumber

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","id":1}'
```

#### eth_getBalance

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "method":"eth_getBalance",
    "params":["0xAddress", "latest"],
    "id":1
  }'
```

#### eth_sendRawTransaction

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "method":"eth_sendRawTransaction",
    "params":["0xSignedTx"],
    "id":1
  }'
```

## ZeroChain 扩展方法

### zero_getAccount

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "method":"zero_getAccount",
    "params":["0xAddress"],
    "id":1
  }'
```

### zero_getUtxos

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "method":"zero_getUtxos",
    "params":["0xAddress"],
    "id":1
  }'
```

## WebSocket 订阅

### 订阅新区块

```javascript
const ws = new WebSocket('ws://localhost:8546');

ws.onopen = () => {
  ws.send(JSON.stringify({
    jsonrpc: '2.0',
    method: 'eth_subscribe',
    params: ['newHeads'],
    id: 1
  }));
};

ws.onmessage = (msg) => {
  console.log(JSON.parse(msg.data));
};
```

### 订阅新交易

```javascript
ws.send(JSON.stringify({
  jsonrpc: '2.0',
  method: 'eth_subscribe',
  params: ['newPendingTransactions'],
  id: 2
}));
```

## 错误码

| 错误码 | 描述 |
|--------|------|
| -32700 | 解析错误 |
| -32600 | 无效请求 |
| -32601 | 方法不存在 |
| -32602 | 无效参数 |
| -32000 | 服务器错误 |
