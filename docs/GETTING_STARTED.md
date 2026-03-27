# ZeroChain 快速入门指南

## 环境要求

- Rust 1.75+
- Cargo
- Linux/macOS/Windows

## 安装

```bash
git clone https://github.com/zerochain/zero-chain.git
cd zero-chain
cargo build --release
./target/release/zerochain --version
```

## 快速启动

```bash
# 初始化数据目录
./target/release/zerochain --network local init

# 启动本地节点
./target/release/zerochain --network local run
```

## 创建账户

```bash
./target/release/zerochain account new \
  --name ed25519-1 \
  --scheme ed25519 \
  --passphrase "StrongPassphrase123!"

./target/release/zerochain account list
```

## 提交 Compute 操作

```bash
./target/release/zerochain compute send \
  --tx-file ./tx.json \
  --account-name ed25519-1 \
  --passphrase "StrongPassphrase123!"

./target/release/zerochain compute get --tx-id 0x...
```

## RPC 查询

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"zero_getAccount","params":["ZER0xYourAddress"],"id":1}'
```

## 下一步

- 阅读 [架构文档](../ARCHITECTURE.md)
- 查看 [API 文档](../docs/API.md)
- 加入 [Discord 社区](https://discord.gg/zerochain)
