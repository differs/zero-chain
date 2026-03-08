# ZeroChain

A blockchain focused on native UTXO Compute execution and PoW security.

## Features

- Native UTXO Compute canonical path
- ed25519 native account and signing flow
- PoW consensus and P2P networking
- JSON-RPC + WebSocket service surface
- CLI for node, wallet, account, transaction, and block operations

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
# Initialize data directory
./target/release/zerochain init

# Run local profile
./target/release/zerochain --network local run

# Run testnet profile
./target/release/zerochain --network testnet run

# Run mainnet profile
./target/release/zerochain --network mainnet run
```

## CLI Examples

```bash
# Create native wallet account
zerochain wallet new --name native-1 --scheme ed25519 --passphrase "StrongPassphrase123!"

# List wallet accounts
zerochain wallet list

# Account alias command (delegates to wallet)
zerochain account new --name native-2 --scheme ed25519 --passphrase "StrongPassphrase123!"
zerochain account list

# Sign message
zerochain wallet sign --name native-1 --message "hello" --passphrase "StrongPassphrase123!"

# Unlock then sign without passphrase
zerochain wallet unlock --name native-1 --passphrase "StrongPassphrase123!" --ttl-secs 600
zerochain wallet sign --name native-1 --message "hello"

# Submit native compute transaction from JSON file
zerochain transaction send --tx-file ./tx.json --account-name native-1 --passphrase "StrongPassphrase123!"

# Query native compute transaction result
zerochain transaction get --tx-id 0x...
```

## RPC Example

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"zero_getAccount","params":["ZER0x..."],"id":1}'
```

## Development

```bash
# Redline guard (禁止 silent fallback)
bash scripts/no_silent_fallback.sh

# Format
cargo fmt

# Lint
cargo clippy -- -D warnings

# Tests
cargo test
```

## Engineering Redlines

- 规范文档：`docs/ENGINEERING_REDLINES.md`
- CI 阻断：`.github/workflows/redline-guard.yml`
- 发布门禁包含 redline 检查：`scripts/run_tests.sh`

## Mainnet Checklist

```bash
# Public local + remote + observer + explorer checklist
./scripts/mainnet_checklist.sh
```

Key checks include:
- local/remote/observer RPC reachability, peerCount, block heights, `zero_syncStatus`
- local/remote block-gap threshold
- explorer `/health`, `/api/overview`, `/api/txs/recent`, account balance + account tx endpoints
- public soak monitor health and RPC/SSH error counters

## License

MIT OR Apache-2.0
