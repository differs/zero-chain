#!/bin/bash
# ZeroChain 测试网启动脚本

set -e

echo "🌐 启动 ZeroChain 测试网"

# 检查构建
if [ ! -f "target/release/zerocchain" ]; then
    echo "📦 构建项目..."
    cargo build --release
fi

# 创建数据目录
DATA_DIR="${HOME}/.zerocchain/testnet"
mkdir -p "$DATA_DIR"

# 测试网配置
cat > "$DATA_DIR/config.toml" << 'TOML'
[network]
port = 30303
bootnodes = [
    "enode://testnet-bootnode-1.zerocchain.io:30303",
    "enode://testnet-bootnode-2.zerocchain.io:30303"
]
max_peers = 50

[rpc]
http_enabled = true
http_port = 8545
http_addr = "0.0.0.0"
ws_enabled = true
ws_port = 8546
ws_addr = "0.0.0.0"

[mining]
enabled = false

[logging]
level = "info"
format = "json"
TOML

echo "✅ 配置文件已创建：$DATA_DIR/config.toml"

# 启动节点
echo "🔗 启动节点..."
./target/release/zerocchain run --config "$DATA_DIR/config.toml" --datadir "$DATA_DIR"
