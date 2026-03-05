# ZeroChain

A next-generation blockchain with hybrid account model, EVM compatibility, and PoW consensus.

## Features

- **Hybrid Account Model**: Combines balance-based and UTXO models for flexibility and privacy
- **EVM Compatible**: Full Ethereum Virtual Machine compatibility with custom precompiles
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

# Run a full node
./target/release/zerocchain run

# Run a mining node
./target/release/zerocchain run --mine --coinbase 0xYourAddress
```

### CLI Commands

```bash
# Show help
zerocchain --help

# Create account
zerocchain account new

# List accounts
zerocchain account list

# Check balance
zerocchain account balance --address 0x...

# Send transaction
zerocchain transaction send --from 0x... --to 0x... --amount 100

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
  -d '{"jsonrpc":"2.0","method":"zero_getAccount","params":["0x..."],"id":1}'
```

## Development

### Run Tests

```bash
# All tests
cargo test

# Specific crate
cargo test -p zerocore

# With output
cargo test -- --nocapture
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
- [ ] P2P networking
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
