# ZeroChain 命令行钱包与挖矿完整教程

本文从零开始说明如何建立命令行钱包、启动 ZeroChain 节点、使用内置矿工挖矿，以及联动 `zero-mining-stack` 的 pool/miner 做真实挖矿闭环。

适用对象：

- 本地开发者：需要快速跑通钱包、节点、RPC、挖矿。
- 节点运维：需要准备 coinbase 地址并启动主网节点。
- 矿工/矿池开发者：需要用 `zero-mining-stack` 连接节点提交工作量证明。

## 0. 安全边界

先读这几条，再执行命令：

- 钱包私钥保存在数据目录的 `wallet.json`，默认目录通常是 `~/.zerochain`，也可以用 `--data-dir` 指定。
- `wallet.json` 是加密文件，但仍然应当备份并限制文件权限。
- passphrase 不要复用，不要提交到 Git，不要写进共享脚本。
- 人工操作时省略 `--passphrase`，CLI 会用无回显 prompt 读取密码。
- `--passphrase` 只建议用于 CI、smoke 或一次性脚本。生产环境应注意 shell history、进程参数可见性和审计日志。
- RPC 写方法需要 token。挖矿节点、矿池和运维脚本都应显式配置 `--rpc-auth-token`，客户端用 `--rpc-token` 或 `Authorization: Bearer <token>` 访问。
- 本文中的 `--mining-work-target-leading-zero-bytes 0` 只用于本地 smoke 和开发环境。主网不要使用这个参数放宽难度。

## 1. 准备源码和二进制

假设工作区结构如下：

```text
zero-chain-workspaces/
  zero-chain/
  zero-mining-stack/
```

构建 ZeroChain CLI：

```bash
cd /home/de/works/zero-chain-workspaces/zero-chain
cargo build -p zerocli --release
```

构建外部挖矿栈：

```bash
cd /home/de/works/zero-chain-workspaces/zero-mining-stack
cargo build --release
```

回到链仓库并设置常用环境变量：

```bash
cd /home/de/works/zero-chain-workspaces/zero-chain

export ZEROCHAIN_BIN="$PWD/target/release/zerochain"
export MINING_STACK_BIN="$PWD/../zero-mining-stack/target/release/zero-mining-stack"
export NODE_DATA="$PWD/.local-zerochain-node"
export RPC_TOKEN="replace-with-a-long-random-rpc-token"
```

检查二进制是否可用：

```bash
"$ZEROCHAIN_BIN" --help
"$MINING_STACK_BIN" --help
```

## 2. 建立命令行钱包

创建一个 ed25519 钱包账户：

```bash
"$ZEROCHAIN_BIN" --data-dir "$NODE_DATA" wallet new \
  --name miner-1 \
  --scheme ed25519
```

CLI 会提示输入并确认钱包密码，输入时不会回显。

输出会包含类似字段：

```text
name: miner-1
scheme: ed25519
public_key: 0x...
address: ZER0x...
private_key: encrypted (... iterations)
```

把 `address:` 后面的地址设置为 coinbase。后续区块奖励会发到这个地址：

```bash
export COINBASE="ZER0xREPLACE_WITH_YOUR_WALLET_ADDRESS"
```

查看钱包列表：

```bash
"$ZEROCHAIN_BIN" --data-dir "$NODE_DATA" wallet list
```

查看单个钱包：

```bash
"$ZEROCHAIN_BIN" --data-dir "$NODE_DATA" wallet show --name miner-1
```

验证签名能力：

```bash
SIGNATURE="$("$ZEROCHAIN_BIN" --data-dir "$NODE_DATA" wallet sign \
  --name miner-1 \
  --message "zerochain mining setup" \
  | sed -n 's/^signature_hex: //p')"

"$ZEROCHAIN_BIN" --data-dir "$NODE_DATA" wallet verify \
  --name miner-1 \
  --message "zerochain mining setup" \
  --signature "$SIGNATURE"
```

也可以临时解锁钱包，避免短时间内重复输入 passphrase：

```bash
"$ZEROCHAIN_BIN" --data-dir "$NODE_DATA" wallet unlock \
  --name miner-1 \
  --ttl-secs 600
```

命令会输出一个 `export ZEROCHAIN_WALLET_UNLOCK_...=...` 形式的环境变量。复制到当前 shell 后，`wallet sign` 可以省略 `--passphrase`，直到 token 过期。

非交互脚本可以显式传入 `--passphrase`，例如 smoke 脚本或临时测试环境：

```bash
export WALLET_PASSPHRASE="replace-with-a-strong-wallet-passphrase"

"$ZEROCHAIN_BIN" --data-dir "$NODE_DATA" wallet new \
  --name miner-ci \
  --scheme ed25519 \
  --passphrase "$WALLET_PASSPHRASE"
```

说明：挖矿启动本身只需要 coinbase 地址，不会也不应该要求钱包密码。钱包密码只在创建钱包、签名、解锁、轮换密码或迁移旧钱包时使用。

## 3. 初始化节点数据目录

本地开发网络：

```bash
"$ZEROCHAIN_BIN" --network local --data-dir "$NODE_DATA" init
```

主网：

```bash
"$ZEROCHAIN_BIN" --network mainnet init
```

说明：

- `local` 默认适合本地开发。
- `mainnet` 使用主网 profile。主网运维不要复用本地测试的数据目录。
- 如果你在同一台机器上跑多个节点，必须给每个节点指定独立 `--data-dir`、RPC 端口、P2P 端口。

## 4. 路径 A：内置矿工快速开始

这条路径最简单，适合确认节点、钱包、coinbase 和 RPC 能正常工作。

本地开发启动：

```bash
"$ZEROCHAIN_BIN" --network local --data-dir "$NODE_DATA" run \
  --mine \
  --coinbase "$COINBASE" \
  --rpc-coinbase "$COINBASE" \
  --rpc-auth-token "$RPC_TOKEN" \
  --rpc-rate-limit-per-minute 600 \
  --mining-work-target-leading-zero-bytes 0
```

另开一个终端查询最新区块：

```bash
"$ZEROCHAIN_BIN" \
  --rpc-url http://127.0.0.1:8545 \
  --rpc-token "$RPC_TOKEN" \
  block latest
```

查询网络 ID：

```bash
curl -fsS http://127.0.0.1:8545 \
  -H "content-type: application/json" \
  -H "authorization: Bearer $RPC_TOKEN" \
  --data '{"jsonrpc":"2.0","id":1,"method":"net_version","params":[]}'
```

查询挖矿任务：

```bash
curl -fsS http://127.0.0.1:8545 \
  -H "content-type: application/json" \
  -H "authorization: Bearer $RPC_TOKEN" \
  --data '{"jsonrpc":"2.0","id":1,"method":"zero_getWork","params":[]}'
```

主网启动时保留默认难度，不要加 `--mining-work-target-leading-zero-bytes 0`：

```bash
"$ZEROCHAIN_BIN" --network mainnet run \
  --mine \
  --coinbase "$COINBASE" \
  --rpc-coinbase "$COINBASE" \
  --rpc-auth-token "$RPC_TOKEN" \
  --rpc-rate-limit-per-minute 600
```

## 5. 路径 B：外部 pool/miner 挖矿闭环

这条路径更接近真实部署：ZeroChain 节点只提供链和 mining RPC，`zero-mining-stack pool` 负责取 job/提交 work，`zero-mining-stack miner` 负责计算 nonce。

### 5.1 启动 ZeroChain 节点

本地开发启动节点，并关闭内置本地 miner：

```bash
"$ZEROCHAIN_BIN" --network local --data-dir "$NODE_DATA" run \
  --mine \
  --disable-local-miner \
  --http-port 8545 \
  --ws-port 8546 \
  --p2p-listen-port 30303 \
  --coinbase "$COINBASE" \
  --rpc-coinbase "$COINBASE" \
  --rpc-auth-token "$RPC_TOKEN" \
  --rpc-rate-limit-per-minute 600 \
  --mining-work-target-leading-zero-bytes 0
```

确认节点 RPC 已就绪：

```bash
curl -fsS http://127.0.0.1:8545 \
  -H "content-type: application/json" \
  -H "authorization: Bearer $RPC_TOKEN" \
  --data '{"jsonrpc":"2.0","id":1,"method":"net_version","params":[]}'
```

### 5.2 启动矿池服务

另开一个终端：

```bash
"$MINING_STACK_BIN" pool \
  --host 127.0.0.1 \
  --port 9332 \
  --node-rpc http://127.0.0.1:8545 \
  --node-rpc-token "$RPC_TOKEN"
```

确认矿池健康：

```bash
curl -fsS http://127.0.0.1:9332/health
curl -fsS http://127.0.0.1:9332/v1/stats
```

### 5.3 启动矿工

再开一个终端：

```bash
"$MINING_STACK_BIN" miner \
  --pool-url http://127.0.0.1:9332 \
  --miner-id miner-1 \
  --metrics-host 127.0.0.1 \
  --metrics-port 9333 \
  --target-leading-zero-bytes 0 \
  --report-interval 1000
```

确认 miner 健康和指标：

```bash
curl -fsS http://127.0.0.1:9333/health
curl -fsS http://127.0.0.1:9333/metrics | grep zero_miner_shares_total
```

确认链高度增长：

```bash
"$ZEROCHAIN_BIN" \
  --rpc-url http://127.0.0.1:8545 \
  --rpc-token "$RPC_TOKEN" \
  block latest
```

确认 pool 已收到 accepted share：

```bash
curl -fsS http://127.0.0.1:9332/metrics | grep zero_pool_shares_accepted_total
```

### 5.4 主网外部挖矿

主网节点仍然使用 `--disable-local-miner`，让外部 pool/miner 负责挖矿：

```bash
"$ZEROCHAIN_BIN" --network mainnet run \
  --mine \
  --disable-local-miner \
  --coinbase "$COINBASE" \
  --rpc-coinbase "$COINBASE" \
  --rpc-auth-token "$RPC_TOKEN" \
  --rpc-rate-limit-per-minute 600
```

主网矿池连接主网节点：

```bash
"$MINING_STACK_BIN" pool \
  --host 127.0.0.1 \
  --port 9332 \
  --node-rpc http://127.0.0.1:8545 \
  --node-rpc-token "$RPC_TOKEN"
```

主网 miner 不应使用本地 smoke 的低难度目标。使用默认参数，或使用与你矿池策略一致的目标：

```bash
"$MINING_STACK_BIN" miner \
  --pool-url http://127.0.0.1:9332 \
  --miner-id miner-mainnet-1 \
  --metrics-host 127.0.0.1 \
  --metrics-port 9333
```

生产部署建议：

- 节点 RPC 只监听内网或本机。
- pool 对外暴露时使用防火墙、TLS、反向代理和独立监控。
- RPC token 使用长随机值，并按环境隔离。
- 钱包文件和节点数据目录定期备份。

## 6. 一键 smoke 验证

仓库已经提供了本地 CLI + 外部 pool/miner 的可重复 smoke：

```bash
cd /home/de/works/zero-chain-workspaces/zero-chain
bash scripts/cli_mining_smoke.sh
```

这个脚本会自动执行：

- 构建 `zerochain` CLI。
- 构建 `zero-mining-stack`。
- 启动本地节点。
- 验证 block CLI 和基础 RPC。
- 提交 compute 交易并查询结果。
- 启动 pool 和 miner。
- 验证区块高度增长、accepted share、pool/miner metrics。

如果要验证严格主网口径：

```bash
bash scripts/mainnet_strict_smoke.sh
```

严格主网口径会使用 mainnet 拓扑、RocksDB、默认限流和 RPC 鉴权。它不是本地低难度 smoke 的替代品。

## 7. 常见问题

### 端口被占用

检查端口：

```bash
ss -ltnp | grep -E ':(8545|8546|30303|9332|9333)\b'
```

解决方式：

- 换 `--http-port`、`--ws-port`、`--p2p-listen-port`、`--port` 或 `--metrics-port`。
- 停掉旧节点、旧 pool、旧 miner。

### RPC 返回 unauthorized

原因通常是节点启用了 `--rpc-auth-token`，但客户端没带 token。

CLI 访问：

```bash
"$ZEROCHAIN_BIN" --rpc-url http://127.0.0.1:8545 --rpc-token "$RPC_TOKEN" block latest
```

curl 访问：

```bash
curl -fsS http://127.0.0.1:8545 \
  -H "content-type: application/json" \
  -H "authorization: Bearer $RPC_TOKEN" \
  --data '{"jsonrpc":"2.0","id":1,"method":"net_version","params":[]}'
```

### zero_getWork 不可用

启动节点时必须加 `--mine`。如果只跑普通节点，mining RPC 不会提供有效工作。

### pool 无法提交 work

优先检查：

- `--node-rpc` 是否指向正确节点。
- `--node-rpc-token` 是否等于节点的 `--rpc-auth-token`。
- 节点是否以 `--mine` 启动。
- 本地开发是否误删了 `--mining-work-target-leading-zero-bytes 0`，导致低算力机器短时间内找不到 share。

### 钱包口令太短

钱包 passphrase 至少需要满足当前 CLI 的强度检查。使用长度足够、不可预测的 passphrase。

### 如何停止

前台启动的节点、pool、miner 用 `Ctrl-C` 停止。

如果使用 `scripts/mainnet.sh` 管理主网拓扑，按对应角色停止，例如：

```bash
bash scripts/mainnet.sh stop bootnode
```

## 8. 命令速查

| 目标 | 命令 |
|---|---|
| 创建钱包 | `zerochain wallet new --name miner-1 --scheme ed25519` |
| 查看钱包 | `zerochain wallet show --name miner-1` |
| 初始化本地节点 | `zerochain --network local --data-dir "$NODE_DATA" init` |
| 本地内置挖矿 | `zerochain --network local --data-dir "$NODE_DATA" run --mine --coinbase "$COINBASE" --rpc-coinbase "$COINBASE" --rpc-auth-token "$RPC_TOKEN" --mining-work-target-leading-zero-bytes 0` |
| 外部挖矿节点 | `zerochain --network local --data-dir "$NODE_DATA" run --mine --disable-local-miner --coinbase "$COINBASE" --rpc-coinbase "$COINBASE" --rpc-auth-token "$RPC_TOKEN" --mining-work-target-leading-zero-bytes 0` |
| 启动 pool | `zero-mining-stack pool --node-rpc http://127.0.0.1:8545 --node-rpc-token "$RPC_TOKEN"` |
| 启动 miner | `zero-mining-stack miner --pool-url http://127.0.0.1:9332 --miner-id miner-1 --target-leading-zero-bytes 0` |
| 查询最新区块 | `zerochain --rpc-url http://127.0.0.1:8545 --rpc-token "$RPC_TOKEN" block latest` |
| 本地完整 smoke | `bash scripts/cli_mining_smoke.sh` |
