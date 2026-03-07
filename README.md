# ZeroChain

A next-generation blockchain with hybrid account model, EVM compatibility, and PoW consensus.

## Features

- **Hybrid Account Model**: Combines balance-based and UTXO models for flexibility and privacy
- **EVM Compatible**: Full Ethereum Virtual Machine compatibility with custom precompiles
- **Dual Signature Model**: EVM path uses secp256k1; native compute path supports ed25519
- **PoW Consensus**: ASIC-resistant mining with RandomX and ProgPoW algorithms
- **Account Abstraction**: Built-in smart contract wallet support
- **High Performance**: Parallel transaction execution and optimized state management

## Architecture

```
┌─────────────────────────────────────────┐
│           ZeroChain Node                │
├─────────────────────────────────────────┤
│  API Layer (RPC/REST/WebSocket)         │
│  ├── JSON-RPC (Ethereum Compatible)     │
│  ├── REST API                           │
│  └── WebSocket Subscriptions            │
├─────────────────────────────────────────┤
│  Core Protocol                          │
│  ├── Account Manager (Hybrid)           │
│  ├── EVM Engine                         │
│  ├── PoW Consensus                      │
│  ├── Transaction Pool                   │
│  └── State Machine                      │
├─────────────────────────────────────────┤
│  Network Layer (P2P)                    │
│  ├── Kademlia DHT                       │
│  ├── Block Propagation                  │
│  └── Transaction Broadcast              │
├─────────────────────────────────────────┤
│  Storage Layer                          │
│  ├── Merkle Patricia Trie               │
│  ├── LevelDB/RocksDB                    │
│  └── UTXO Database                      │
└─────────────────────────────────────────┘
```

## Project Structure

```
zero-chain/
├── Cargo.toml              # Workspace configuration
├── crates/
│   ├── zerocore/           # Core protocol
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── crypto.rs   # Cryptographic primitives
│   │   │   ├── account/    # Account management
│   │   │   ├── transaction/
│   │   │   ├── block/
│   │   │   ├── consensus/
│   │   │   ├── evm/
│   │   │   └── state/
│   ├── zeronet/            # P2P networking
│   ├── zerostore/          # Storage layer
│   ├── zeroapi/            # API services
│   └── zerocli/            # CLI and node
└── tests/                  # Integration tests
```

## Quick Start

### Prerequisites

- Rust 1.75+
- Cargo

### Build

```bash
# Clone the repository
git clone https://github.com/zerocchain/zero-chain.git
cd zero-chain

# Build in release mode
cargo build --release

# Run tests
cargo test
```

### Run a Node

```bash
# Initialize data directory
./target/release/zerocchain init

# Run a local node (default profile)
./target/release/zerocchain --network local run

# Run testnet profile
./target/release/zerocchain --network testnet run

# Run mainnet profile
./target/release/zerocchain --network mainnet run

# Run a mining node
./target/release/zerocchain run --mine --coinbase 0xYourAddress

# Override chain/network id at runtime
./target/release/zerocchain --network mainnet run --chain-id 0x276e --rpc-network-id 10086

# Multi-node bootstrap example (P2P)
# node-1 (bootnode listener)
./target/release/zerocchain run --http-port 18645 --ws-port 18646 \
  --p2p-listen-addr 0.0.0.0 --p2p-listen-port 30331

# node-2 (connect to node-1)
./target/release/zerocchain run --http-port 28645 --ws-port 28646 \
  --p2p-listen-addr 0.0.0.0 --p2p-listen-port 30332 \
  --bootnode enode://bootnode1@<node1-ip>:30331

# verify p2p peer count
curl -s -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"net_peerCount","params":[]}' \
  http://127.0.0.1:28645

# hardened node baseline (rpc auth+rate limit + p2p dos guardrails)
./target/release/zerocchain run \
  --rpc-auth-token "replace-with-long-token" \
  --rpc-rate-limit-per-minute 600 \
  --p2p-ban-duration-secs 600 \
  --p2p-max-inbound-per-ip 8 \
  --p2p-max-inbound-rate-per-minute 120 \
  --p2p-max-gossip-per-peer-per-minute 240 \
  --p2p-bootnode-retry-interval-secs 15

# inspect detailed peers
curl -s -H "Content-Type: application/json" \
  -H "Authorization: Bearer replace-with-long-token" \
  -d '{"jsonrpc":"2.0","id":1,"method":"zero_peers","params":[]}' \
  http://127.0.0.1:8545
```

### CLI Commands

```bash
# Show help
zerocchain --help

# Create encrypted native wallet (ed25519)
zerocchain wallet new --name native-1 --scheme ed25519 --passphrase "StrongPassphrase123!"

# Create encrypted EVM wallet (secp256k1)
zerocchain wallet new --name evm-1 --scheme secp256k1 --passphrase "StrongPassphrase123!"

# List wallet accounts
zerocchain wallet list

# Sign with passphrase
zerocchain wallet sign --name native-1 --message "hello" --passphrase "StrongPassphrase123!"

# Unlock account for temporary session then sign without passphrase
zerocchain wallet unlock --name native-1 --passphrase "StrongPassphrase123!" --ttl-secs 600
zerocchain wallet sign --name native-1 --message "hello"

# Rotate account passphrase
zerocchain wallet rotate-passphrase --name native-1 --old-passphrase "StrongPassphrase123!" --new-passphrase "NewPassphrase123!"

# Migrate legacy plaintext wallet to encrypted format
zerocchain wallet migrate-v1 --passphrase "StrongPassphrase123!"

# Get block info
zerocchain block latest
zerocchain block get --number 12345
```

## Configuration

Create `~/.zerocchain/config.toml`:

```toml
[network]
port = 30303
bootnodes = ["enode://..."]

[rpc]
http_enabled = true
http_port = 8545
ws_enabled = true
ws_port = 8546

[mining]
enabled = false
threads = 4
```

## API Examples

### JSON-RPC (Ethereum Compatible)

```bash
# Get balance
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_getBalance","params":["0x...", "latest"],"id":1}'

# Send transaction
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_sendRawTransaction","params":["0x..."],"id":1}'

# ZeroChain extensions
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"zero_getAccount","params":["ZER0x..."],"id":1}'
```

`zero_getAccount` / `zero_getUtxos` 现在推荐使用原生地址格式：`ZER0x` + 40 hex（checksum）。

## Development

### Run Tests

```bash
# All tests
cargo test

# Specific crate
cargo test -p zerocore

# With output
cargo test -- --nocapture

# Release gate (fmt/check/test + go/no-go report)
bash scripts/run_tests.sh
```

### Observability (OpenTelemetry)

```bash
# Start local collector + Jaeger
cd deploy/observability && docker compose up -d

# Run node with OTel export
zerocchain --otel-enabled --otel-endpoint http://127.0.0.1:4317 --network testnet run
```

See `docs/OBSERVABILITY.md` for details.

Release gate report path:

```text
artifacts/release-gate/go-no-go-report.md
```

Latest full-chain integration record:

```text
docs/FULL_CHAIN_E2E_2026-03-07.md
```

P0 release blockers tracker:

```text
docs/P0_RELEASE_BLOCKERS_2026-03.md
```

Run full-chain E2E locally:

```bash
bash scripts/full_chain_e2e.sh
```

Run 3-node P2P convergence smoke locally:

```bash
bash scripts/p2p_three_node_smoke.sh
```

### Benchmark

```bash
cargo bench
```

### Code Quality

```bash
# Format code
cargo fmt

# Lint
cargo clippy -- -D warnings

# Security audit
cargo audit
```

## Roadmap

- [x] Core protocol design
- [x] Hybrid account model
- [x] EVM implementation
- [x] PoW consensus
- [x] P2P networking (basic discovery + peer connectivity)
- [ ] State sync
- [ ] Light client
- [ ] Sharding support

## License

MIT OR Apache-2.0

## Contributing

1. Fork the repository
2. Create your feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add some amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## Community

- Website: https://zerocchain.io
- Twitter: @ZeroChain
- Discord: https://discord.gg/zerocchain
- Telegram: https://t.me/zerocchain
