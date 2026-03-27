# ZeroChain

A blockchain focused on native UTXO Compute execution and PoW security.

## Features

- Native UTXO Compute canonical path
- ed25519 account and signing flow
- PoW consensus and P2P networking
- JSON-RPC + WebSocket service surface
- CLI for node, wallet, account, compute, and block operations

## Quick Start

```bash
# Clone
git clone https://github.com/zerochain/zero-chain.git
cd zero-chain

# Build
cargo build --release

# Run tests
cargo test
```

## Run a Node

```bash
# Initialize data directory (once per network profile)
./target/release/zerochain --network local init
./target/release/zerochain --network testnet init
./target/release/zerochain --network devnet init
./target/release/zerochain --network mainnet init

# Run local profile
./target/release/zerochain --network local run

# Run testnet profile
./target/release/zerochain --network testnet run

# Run devnet profile
./target/release/zerochain --network devnet run

# Run mainnet profile
./target/release/zerochain --network mainnet run
```

## CLI Examples

```bash
# Create native wallet account
zerochain wallet new --name ed25519-1 --scheme ed25519 --passphrase "StrongPassphrase123!"

# List wallet accounts
zerochain wallet list

# Account alias command (delegates to wallet)
zerochain account new --name ed25519-2 --scheme ed25519 --passphrase "StrongPassphrase123!"
zerochain account list

# Sign message
zerochain wallet sign --name ed25519-1 --message "hello" --passphrase "StrongPassphrase123!"

# Unlock then sign without passphrase
zerochain wallet unlock --name ed25519-1 --passphrase "StrongPassphrase123!" --ttl-secs 600
zerochain wallet sign --name ed25519-1 --message "hello"

# Submit compute operation from JSON file
zerochain compute send --tx-file ./tx.json

# Query compute operation result
zerochain compute get --tx-id 0x...
```

## RPC Example

```bash
# Default RPC ports:
# - local/mainnet: 8545
# - testnet: 18545
# - devnet: 28545
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"zero_getAccount","params":["ZER0x..."],"id":1}'
```

## Development

```bash
# Redline guard (禁止 silent fallback)
bash scripts/no_silent_fallback.sh

# 指定目录检查（可重复 -d）
bash scripts/no_silent_fallback.sh -d ../zero-chain -d ../zero-explore

# Format
cargo fmt

# Lint
cargo clippy -- -D warnings

# Tests
cargo test
```

## Engineering Redlines

- 设计理念：`docs/DESIGN_PHILOSOPHY.md`
- 规范文档：`docs/ENGINEERING_REDLINES.md`
- CI 阻断：`.github/workflows/redline-guard.yml`
- 发布门禁包含 redline 检查：`scripts/run_tests.sh`

## Mainnet Checklist

```bash
# Public local + remote + observer + explorer checklist
./scripts/mainnet_checklist.sh
```

## Mainnet Bring-up

受控启网与受控真实挖矿 runbook：

- `docs/MAINNET_BRINGUP_RUNBOOK.md`
- `docs/MAINNET_NODE_MATRIX.md`
- `docs/MAINNET_FIRST_WAVE_COMMANDS.md`
- `docs/MAINNET_LOCAL_BRINGUP.md`
- `docs/MAINNET_REMOTE_BRINGUP.md`

主节点启动入口：

```bash
./scripts/mainnet.sh start bootnode --mine --coinbase ZER0xYOUR_COINBASE
```

## Workspace Acceptance

统一验收当前多仓工作区：

```bash
cd zero-chain
bash scripts/workspace_acceptance.sh --quick
```

完整模式：

```bash
cd zero-chain
bash scripts/workspace_acceptance.sh --full
```

详细口径见：

- `docs/WORKSPACE_ACCEPTANCE_CHECKLIST.md`

Key checks include:
- local/remote/observer RPC reachability, peerCount, block heights, `zero_syncStatus`
- local/remote block-gap threshold
- explorer `/health`, `/api/overview`, `/api/txs/recent`, account balance + account tx endpoints
- public soak monitor health and RPC/SSH error counters

## License

MIT OR Apache-2.0
