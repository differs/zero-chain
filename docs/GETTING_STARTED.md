# ZeroChain 快速入门指南

## 环境要求

- Rust 1.75+
- Cargo
- Linux/macOS/Windows

## 安装

### 从源码构建

```bash
# 克隆仓库
git clone https://github.com/zerochain/zero-chain.git
cd zero-chain

# 构建
cargo build --release

# 验证安装
./target/release/zerochain --version
```

## 快速启动

### 运行开发节点

```bash
# 使用脚本快速启动
./scripts/devnet.sh

# 或手动启动
./target/release/zerochain run --dev
```

### 创建账户

```bash
# 创建新账户
./target/release/zerochain account new \
  --name evm-1 \
  --scheme secp256k1 \
  --passphrase "StrongPassphrase123!"

# 查看账户列表
./target/release/zerochain account list
```

### 发送交易

```bash
./target/release/zerochain transaction send \
  --from 0xYourAddress \
  --to 0xRecipientAddress \
  --amount 1000000000000000000 \
  --account-name evm-1 \
  --passphrase "StrongPassphrase123!"

# 若返回 -32010，请在开发环境重启节点并开启写 RPC：
./target/release/zerochain run --rpc-enable-eth-write-rpcs
```

## 连接测试网

```bash
./scripts/testnet.sh
```

## RPC 查询

### 获取区块号

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","id":1}'
```

### 获取余额

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0",
    "method":"eth_getBalance",
    "params":["0xYourAddress", "latest"],
    "id":1
  }'
```

## 下一步

- 阅读 [架构文档](../ARCHITECTURE.md)
- 查看 [API 文档](../docs/API.md)
- 加入 [Discord 社区](https://discord.gg/zerochain)
