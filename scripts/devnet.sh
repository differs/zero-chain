#!/bin/bash
# ZeroChain 开发网络启动脚本

set -e

echo "🚀 启动 ZeroChain 开发网络"

# 检查构建
if [ ! -f "target/release/zerocchain" ]; then
    echo "📦 构建项目..."
    cargo build --release
fi

# 创建数据目录
DATA_DIR="${HOME}/.zerocchain/devnet"
mkdir -p "$DATA_DIR"

# 生成配置文件
cat > "$DATA_DIR/config.toml" << 'TOML'
[network]
port = 30303
bootnodes = []
max_peers = 25

[rpc]
http_enabled = true
http_port = 8545
http_addr = "127.0.0.1"
ws_enabled = true
ws_port = 8546
ws_addr = "127.0.0.1"

[mining]
enabled = true
threads = 2
coinbase = "0x0000000000000000000000000000000000000000"

[logging]
level = "info"
format = "json"
TOML

echo "✅ 配置文件已创建：$DATA_DIR/config.toml"

# 启动节点
echo "🔗 启动节点..."
./target/release/zerocchain run --config "$DATA_DIR/config.toml" --datadir "$DATA_DIR"
