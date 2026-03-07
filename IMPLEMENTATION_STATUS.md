# 实现状态（Native-Only）

## 核心模块

| 模块 | 状态 | 说明 |
|---|---|---|
| UTXO Compute 执行 | ✅ | Canonical 写路径已可用 |
| 账户与资源模型 | ✅ | `zero_getAccount` / `zero_getUtxos` 可查询 |
| PoW 与挖矿接口 | ✅ | `zero_getWork` / `zero_submitWork` 可用 |
| RPC 服务 | ✅ | 默认仅保留 native 语义方法 |
| WebSocket 订阅 | ✅ | `zero_subscribe` / `zero_unsubscribe` |
| 持久化后端 | ✅ | Mem / RocksDB / Redb |

## 最近收敛

- CLI `transaction send` 收敛到 `zero_submitComputeTx`
- 钱包端统一 `ed25519`
- 节点 API 入口与文档去 legacy 兼容语义

## 验证建议

```bash
cargo test -p zerocli
cargo test -p zeroapi
```
