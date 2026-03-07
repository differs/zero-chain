# ZeroChain API 文档

## JSON-RPC 端点

- HTTP: `http://localhost:8545`
- WebSocket: `ws://localhost:8546`

### Compute 持久化后端配置（RPC）

`RpcConfig` 新增字段：

- `compute_backend`: `ComputeBackend::{Mem|RocksDb|Redb}`
- `compute_db_path`: 文件型后端（`rocksdb`/`redb`）的路径

默认值：

- `compute_backend = ComputeBackend::Mem`
- `compute_db_path = "./data/compute-db"`

### RPC 安全基线配置

`RpcConfig` 新增安全字段：

- `auth_token`: 可选静态 token。配置后所有 JSON-RPC 请求都需携带认证头。
- `rate_limit_per_minute`: 每客户端每分钟请求上限，`0` 表示关闭限流。

支持两种认证头：

- `Authorization: Bearer <token>`
- `x-zero-token: <token>`

示例：

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer demo-token" \
  -d '{"jsonrpc":"2.0","method":"web3_clientVersion","params":[],"id":1}'
```

CLI 启用：

```bash
zerocchain run \
  --rpc-auth-token demo-token \
  --rpc-rate-limit-per-minute 600
```

### CLI 多网络运行（mainnet/testnet/devnet/local）

`zerocchain run` 已支持按网络 profile 启动：

- `--network mainnet`
- `--network testnet`
- `--network devnet`
- `--network local`（默认）

每个 profile 会设置默认的 `chain_id/network_id/ports`，并将 compute DB 路径按网络隔离。

可通过参数覆盖：

- `--chain-id <hex|dec>` 覆盖 `eth_chainId`
- `--rpc-network-id <dec>` 覆盖 `net_version`
- `--rpc-coinbase <0x...>` 覆盖 `eth_coinbase`

示例：

```bash
# 启动 testnet
zerocchain --network testnet run --compute-backend rocksdb --compute-db-path ~/.zerocchain/testnet/compute-db

# 启动 mainnet（显式链参数）
zerocchain --network mainnet run --chain-id 0x276e --rpc-network-id 10086
```

校验规则：

- 当 `compute_backend` 为 `RocksDb` 或 `Redb` 时，`compute_db_path` 不能为空。

CLI 使用建议：

- 生成默认配置：`zerocchain init --data-dir <dir>`（会生成 `<dir>/api-config.json`）
- 启动并加载配置：`zerocchain --config <dir>/api-config.json run --http-port 8545 --ws-port 8546`
- CLI 参数优先级高于配置文件（例如 `--compute-backend`、`--compute-db-path` 会覆盖配置文件值）

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
    "params":["ZER0xAddress"],
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
    "params":["ZER0xAddress"],
    "id":1
  }'
```

说明：`zero_getAccount` / `zero_getUtxos` 推荐使用原生地址 `ZER0x` + 40 hex（checksum）；EVM `0x...` 地址在兼容路径下仍可解析。

### zero_peers

用于查询当前节点已连接 peer 的实时详情（联调与运维观测）。

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "method":"zero_peers",
    "params":[],
    "id":1
  }'
```

响应示例：

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": [
    {
      "peer_id": "node-xxxxxxxx",
      "network_id": 10086,
      "protocol_version": 1,
      "client_version": "Unknown",
      "remote_addr": "127.0.0.1:31332",
      "local_addr": "0.0.0.0:0",
      "connected_at": 1760000000,
      "last_activity": 1760000015,
      "idle_secs": 2,
      "reputation": 100,
      "capabilities": []
    }
  ]
}
```

### zero_getDomain

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "method":"zero_getDomain",
    "params":[0],
    "id":1
  }'
```

### zero_getObject

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "method":"zero_getObject",
    "params":["0x7777777777777777777777777777777777777777777777777777777777777777"],
    "id":1
  }'
```

### zero_getOutput

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "method":"zero_getOutput",
    "params":["0x6666666666666666666666666666666666666666666666666666666666666666"],
    "id":1
  }'
```

### zero_simulateComputeTx

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "method":"zero_simulateComputeTx",
    "params":[{
      "tx_id":"0x5555555555555555555555555555555555555555555555555555555555555555",
      "domain_id":0,
      "chain_id":10086,
      "network_id":1,
      "command":"Mint",
      "input_set":[],
      "read_set":[],
      "output_proposals":[{
        "output_id":"0x6666666666666666666666666666666666666666666666666666666666666666",
        "object_id":"0x7777777777777777777777777777777777777777777777777777777777777777",
        "domain_id":0,
        "kind":"State",
        "owner":{"type":"Shared"},
        "predecessor":null,
        "version":1,
        "state":"0x010203",
        "logic":null
      }],
      "payload":"0x",
      "deadline_unix_secs":null,
      "witness":{
        "signatures":[
          "0x010101010101010101010101010101010101010101010101010101010101010102020202020202020202020202020202020202020202020202020202020202021b"
        ],
        "threshold":1
      }
    }],
    "id":1
  }'
```

响应示例：

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "ok": true,
    "inputs": 0,
    "reads": 0,
    "outputs": 1,
    "tx_id": "0x5555555555555555555555555555555555555555555555555555555555555555"
  }
}
```

失败响应（结构化错误）示例：

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "ok": false,
    "error": {
      "numeric_code": 1001,
      "code": "domain_error",
      "category": "domain",
      "message": "Domain is not registered: 9"
    }
  }
}
```

错误码约定（`zero_simulateComputeTx`）：

- `1001` `domain_error`
- `2001` `readset_error`
- `3001` `authorization_error`
- `3002` `ownership_check_failed`
- `3003` `invalid_signature`
- `3004` `signature_owner_mismatch`
- `3005` `tx_id_mismatch`
- `3006` `unsupported_signature_scheme`
- `4001` `state_error`
- `5001` `resource_error`
- `6001` `tx_error`

其中授权相关失败还包含：

- 无足够签名（threshold 未满足）
- `ownership_check_failed`（输入 owner 非法）
- `invalid_signature`（签名无法恢复/格式非法）
- `signature_owner_mismatch`（签名恢复地址与 owner 不匹配）
- `tx_id_mismatch`（`tx_id` 与签名 preimage 的规范哈希不一致）

Witness 签名格式支持两种：

- 兼容格式（默认按 `secp256k1` 解析）：
  - `"0x<65-byte-signature-hex>"`
- 显式对象格式：
  - `{"scheme":"secp256k1","signature":"0x..."}`
  - `{"scheme":"ed25519","signature":"0x...","public_key":"0x<32-byte-pubkey>"}`

`owner` 也支持原生 ed25519 所有权：

- `{"type":"NativeEd25519","public_key":"0x<32-byte-pubkey>"}`

### ComputeTx 签名与 anti-replay 规则

- 签名消息采用域分离前缀：`ZEROCHAIN-COMPUTE-SIGNING-V1`
- 签名 preimage 绑定字段包括：
  - `domain_id`、`command`
  - `input_set`、`read_set`、`output_proposals`
  - `payload`、`deadline_unix_secs`
  - `chain_id`、`network_id`
  - `witness.threshold`
- 节点校验规则：`tx_id == keccak256(signing_preimage)`（不满足则返回 `tx_id_mismatch`）
- 为兼容旧请求，`chain_id`/`network_id` 可省略；省略时按 `null` 参与签名 preimage。

### zero_submitComputeTx

`zero_submitComputeTx` 的请求体与 `zero_simulateComputeTx` 相同。

响应示例：

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "ok": true,
    "tx_id": "0x5555555555555555555555555555555555555555555555555555555555555555",
    "consumed_inputs": 0,
    "read_objects": 0,
    "created_outputs": 1
  }
}
```

### zero_getComputeTxResult

根据 `tx_id` 查询最近一次 `zero_submitComputeTx` 的执行结果。

- 找到结果时返回提交时的 result 对象。
- 未找到时返回 `null`。

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "method":"zero_getComputeTxResult",
    "params":["0x5555555555555555555555555555555555555555555555555555555555555555"],
    "id":1
  }'
```

重复提交（相同 `tx_id`）响应示例：

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "ok": true,
    "duplicate": true,
    "result": {
      "ok": true,
      "tx_id": "0x5555555555555555555555555555555555555555555555555555555555555555",
      "consumed_inputs": 0,
      "read_objects": 0,
      "created_outputs": 1
    }
  }
}
```

### zero_getWork

获取节点当前挖矿工作模板（MVP）。

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "method":"zero_getWork",
    "params":[],
    "id":1
  }'
```

响应示例：

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "work_id": "work-42-1710000000",
    "prev_hash": "0x...",
    "height": 42,
    "target_leading_zero_bytes": 2,
    "coinbase": "0x0000000000000000000000000000000000000000"
  }
}
```

### zero_submitWork

提交矿工计算结果（share/work）。

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "method":"zero_submitWork",
    "params":[{
      "work_id":"work-42-1710000000",
      "nonce":123456,
      "hash_hex":"0x...",
      "miner":"miner-local-1"
    }],
    "id":1
  }'
```

成功响应示例：

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "accepted": true,
    "block_hash": "0x...",
    "height": 42
  }
}
```

失败响应示例：

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "accepted": false,
    "reason": "low_difficulty_share"
  }
}
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
