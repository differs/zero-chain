//! ZeroChain CLI - Blockchain node and client

#![allow(unused)]

use crate::commands::wallet::{WalletCommand, WalletScheme};
use anyhow::Result;
use clap::{Parser, Subcommand};
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use zeroapi::rpc::{ComputeBackend, RpcConfig};
use zeroapi::ApiConfig;

mod commands;

/// ZeroChain CLI
#[derive(Parser)]
#[command(name = "zerochain")]
#[command(author = "ZeroChain Team")]
#[command(version = "0.1.0")]
#[command(about = "ZeroChain blockchain node and client", long_about = None)]
struct Cli {
    /// Log level
    #[arg(short, long, default_value = "info")]
    log_level: String,

    /// Data directory
    #[arg(short, long, default_value = "~/.zerochain")]
    data_dir: String,

    /// Network ID
    #[arg(short, long, default_value = "10086")]
    network_id: u64,

    /// Network profile (mainnet|testnet|devnet|local)
    #[arg(long, default_value = "local")]
    network: String,

    /// JSON-RPC URL used by account/transaction query commands
    #[arg(long)]
    rpc_url: Option<String>,

    /// Optional node config file (JSON)
    #[arg(long)]
    config: Option<String>,

    /// Enable OpenTelemetry tracing export (OTLP)
    #[arg(long, default_value_t = false)]
    otel_enabled: bool,

    /// OTLP endpoint, e.g. http://127.0.0.1:4317
    #[arg(long, default_value = "http://127.0.0.1:4317")]
    otel_endpoint: String,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a full node
    Run {
        /// Enable mining
        #[arg(long)]
        mine: bool,

        /// Coinbase address
        #[arg(long)]
        coinbase: Option<String>,

        /// HTTP RPC port
        #[arg(long, default_value = "8545")]
        http_port: u16,

        /// WebSocket RPC port
        #[arg(long, default_value = "8546")]
        ws_port: u16,

        /// Compute persistent backend (mem|rocksdb|redb)
        #[arg(long, default_value = "mem")]
        compute_backend: String,

        /// Compute database path for rocksdb/redb
        #[arg(long, default_value = "./data/compute-db")]
        compute_db_path: String,

        /// Optional chain id override (hex or decimal)
        #[arg(long)]
        chain_id: Option<String>,

        /// Optional network id override (decimal)
        #[arg(long)]
        rpc_network_id: Option<u64>,

        /// Optional coinbase override
        #[arg(long)]
        rpc_coinbase: Option<String>,

        /// Optional static RPC auth token (Bearer or x-zero-token)
        #[arg(long)]
        rpc_auth_token: Option<String>,

        /// RPC rate limit budget per client per minute, 0 to disable
        #[arg(long, default_value = "600")]
        rpc_rate_limit_per_minute: u32,

        /// P2P listen address
        #[arg(long, default_value = "0.0.0.0")]
        p2p_listen_addr: String,

        /// P2P listen port
        #[arg(long, default_value = "30303")]
        p2p_listen_port: u16,

        /// Bootnode enode, repeatable.
        #[arg(long = "bootnode")]
        bootnodes: Vec<String>,

        /// Max connected peers
        #[arg(long, default_value = "50")]
        max_peers: u32,

        /// Disable discovery service
        #[arg(long, default_value_t = false)]
        disable_discovery: bool,

        /// Disable sync service
        #[arg(long, default_value_t = false)]
        disable_sync: bool,

        /// Optional persisted banlist path for P2P peers/IPs
        #[arg(long)]
        p2p_banlist_path: Option<String>,

        /// Default ban duration in seconds for abusive peers
        #[arg(long, default_value = "600")]
        p2p_ban_duration_secs: u64,

        /// Maximum active inbound peers per source IP
        #[arg(long, default_value = "8")]
        p2p_max_inbound_per_ip: u32,

        /// Maximum inbound connection attempts per source IP per minute
        #[arg(long, default_value = "120")]
        p2p_max_inbound_rate_per_minute: u32,

        /// Maximum inbound gossip frames per peer per minute
        #[arg(long, default_value = "240")]
        p2p_max_gossip_per_peer_per_minute: u32,

        /// Retry interval for reconnecting missing bootnodes
        #[arg(long, default_value = "15")]
        p2p_bootnode_retry_interval_secs: u64,
    },

    /// Initialize data directory
    Init,

    /// Account management
    Account {
        #[command(subcommand)]
        action: AccountAction,
    },

    /// Transaction commands
    Transaction {
        #[command(subcommand)]
        action: TransactionAction,
    },

    /// Block commands
    Block {
        #[command(subcommand)]
        action: BlockAction,
    },

    /// Console/REPL mode
    Console,

    /// Version information
    Version,

    /// Wallet commands
    Wallet {
        #[command(subcommand)]
        action: WalletAction,
    },
}

#[derive(Subcommand, Debug)]
enum WalletAction {
    /// Create new wallet account
    New {
        /// Optional account name
        #[arg(long)]
        name: Option<String>,
        /// Signature scheme: ed25519 (native only)
        #[arg(long, default_value = "ed25519")]
        scheme: String,
        /// Passphrase for encrypting private key (required)
        #[arg(long)]
        passphrase: String,
    },
    /// List wallet accounts
    List,
    /// Show wallet account details
    Show {
        #[arg(long)]
        name: String,
    },
    /// Sign message with wallet account
    Sign {
        #[arg(long)]
        name: String,
        #[arg(long)]
        message: String,
        /// Passphrase used to decrypt key material (optional if unlocked)
        #[arg(long)]
        passphrase: Option<String>,
    },
    /// Verify signature with wallet account public key
    Verify {
        #[arg(long)]
        name: String,
        #[arg(long)]
        message: String,
        #[arg(long)]
        signature: String,
    },
    /// Delete wallet account
    Delete {
        #[arg(long)]
        name: String,
    },

    /// Re-encrypt account with a new passphrase
    RotatePassphrase {
        #[arg(long)]
        name: String,
        #[arg(long)]
        old_passphrase: String,
        #[arg(long)]
        new_passphrase: String,
    },

    /// Unlock account for a temporary signing session
    Unlock {
        #[arg(long)]
        name: String,
        #[arg(long)]
        passphrase: String,
        #[arg(long, default_value_t = 600)]
        ttl_secs: u64,
    },

    /// Migrate legacy wallet v1 (plaintext key) to encrypted v2 format
    MigrateV1 {
        #[arg(long)]
        passphrase: String,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum AccountAction {
    /// Create new account
    New {
        /// Optional account name
        #[arg(long)]
        name: Option<String>,
        /// Signature scheme: ed25519 (native only)
        #[arg(long, default_value = "ed25519")]
        scheme: String,
        /// Passphrase for encrypting private key
        #[arg(long)]
        passphrase: String,
    },
    /// List accounts
    List,
    /// Get account balance
    Balance {
        #[arg(short, long)]
        address: String,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum TransactionAction {
    /// Send transaction
    Send {
        /// Native compute transaction JSON file path
        #[arg(long)]
        tx_file: String,
        /// Wallet account name used for local signing
        #[arg(long)]
        account_name: String,
        /// Wallet passphrase used to decrypt signing key (optional if wallet unlock token exists)
        #[arg(long)]
        passphrase: Option<String>,
    },
    /// Get native compute transaction result by tx id
    Get {
        #[arg(short, long)]
        tx_id: String,
    },
}

#[derive(Subcommand, Debug)]
enum BlockAction {
    /// Get latest block
    Latest,
    /// Get block by number
    Get {
        #[arg(short, long)]
        number: u64,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let data_dir = expand_data_dir(&cli.data_dir);
    let rpc_url = cli
        .rpc_url
        .clone()
        .unwrap_or_else(|| default_rpc_url_for_network(&cli.network));

    // Initialize logging / tracing
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        format!(
            "info,zeroccore={},zeronet={},zeroapi={}",
            cli.log_level, cli.log_level, cli.log_level
        )
        .into()
    });

    if cli.otel_enabled {
        let tracer = init_otel_tracer(&cli.otel_endpoint)?;
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer())
            .with(OpenTelemetryLayer::new(tracer))
            .init();
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer())
            .init();
    }

    match cli.command {
        Some(Commands::Run {
            mine,
            coinbase,
            http_port,
            ws_port,
            compute_backend,
            compute_db_path,
            chain_id,
            rpc_network_id,
            rpc_coinbase,
            rpc_auth_token,
            rpc_rate_limit_per_minute,
            p2p_listen_addr,
            p2p_listen_port,
            bootnodes,
            max_peers,
            disable_discovery,
            disable_sync,
            p2p_banlist_path,
            p2p_ban_duration_secs,
            p2p_max_inbound_per_ip,
            p2p_max_inbound_rate_per_minute,
            p2p_max_gossip_per_peer_per_minute,
            p2p_bootnode_retry_interval_secs,
        }) => {
            let mut api_config = if let Some(path) = &cli.config {
                commands::init::load_api_config(path)?
            } else {
                ApiConfig::default()
            };

            let profile = NetworkProfile::parse(&cli.network)?;
            profile.apply_defaults(&mut api_config, &data_dir);

            // CLI flags override config file.
            let backend = parse_compute_backend(&compute_backend)?;
            let chain_id = match chain_id {
                Some(value) => parse_u64_decimal_or_hex(&value)?,
                None => api_config.http_rpc.chain_id,
            };
            let rpc_network_id = rpc_network_id.unwrap_or(api_config.http_rpc.network_id);
            let rpc_coinbase = rpc_coinbase.unwrap_or(api_config.http_rpc.coinbase.clone());

            api_config.http_rpc = RpcConfig {
                address: "127.0.0.1".to_string(),
                port: http_port,
                compute_backend: backend,
                compute_db_path,
                chain_id,
                network_id: rpc_network_id,
                coinbase: rpc_coinbase,
                auth_token: rpc_auth_token,
                rate_limit_per_minute: rpc_rate_limit_per_minute,
                ..api_config.http_rpc
            };
            api_config.ws.port = ws_port;

            println!("🌐 Network profile: {}", profile.as_str());
            println!("   chain_id: {}", api_config.http_rpc.chain_id);
            println!("   network_id: {}", api_config.http_rpc.network_id);
            println!("   coinbase: {}", api_config.http_rpc.coinbase);
            println!(
                "   rpc: {}:{}",
                api_config.http_rpc.address, api_config.http_rpc.port
            );
            println!(
                "   rpc auth: {}",
                if api_config.http_rpc.auth_token.is_some() {
                    "enabled"
                } else {
                    "disabled"
                }
            );
            println!(
                "   rpc rate limit: {} req/min",
                api_config.http_rpc.rate_limit_per_minute
            );
            println!("   p2p: {}:{}", p2p_listen_addr, p2p_listen_port);
            if !bootnodes.is_empty() {
                println!("   bootnodes: {}", bootnodes.join(", "));
            }

            commands::run::run_node(commands::run::RunNodeConfig {
                mine,
                coinbase,
                http_port,
                ws_port,
                data_dir: data_dir.clone(),
                rpc_config: Some(api_config.http_rpc.clone()),
                p2p_listen_addr,
                p2p_listen_port,
                bootnodes,
                max_peers,
                enable_discovery: !disable_discovery,
                enable_sync: !disable_sync,
                p2p_banlist_path,
                p2p_ban_duration_secs,
                p2p_max_inbound_per_ip,
                p2p_max_inbound_rate_per_minute,
                p2p_max_gossip_per_peer_per_minute,
                p2p_bootnode_retry_interval_secs,
            })
            .await?;
        }
        Some(Commands::Init) => {
            commands::init::init_data_dir(&data_dir)?;
            let cfg_path = format!("{}/api-config.json", &data_dir);
            commands::init::write_default_api_config(&cfg_path)?;
            println!("Default API config written to {}", cfg_path);
        }
        Some(Commands::Account { action }) => {
            commands::account::handle_account(action, &data_dir, &rpc_url).await?;
        }
        Some(Commands::Transaction { action }) => {
            commands::transaction::handle_transaction(action, &data_dir, &rpc_url).await?;
        }
        Some(Commands::Block { action }) => {
            commands::block::handle_block(format!("{:?}", action)).await?;
        }
        Some(Commands::Console) => {
            commands::console::start_console().await?;
        }
        Some(Commands::Version) => {
            println!("ZeroChain v{}", env!("CARGO_PKG_VERSION"));
        }
        Some(Commands::Wallet { action }) => {
            let cmd = map_wallet_action(action)?;
            commands::wallet::handle_wallet(&data_dir, cmd).await?;
        }
        None => {
            // Default: show help
            println!("ZeroChain v{}", env!("CARGO_PKG_VERSION"));
            println!("Use --help for usage information");
        }
    }

    Ok(())
}

fn init_otel_tracer(endpoint: &str) -> Result<opentelemetry_sdk::trace::Tracer> {
    let exporter = opentelemetry_otlp::new_exporter()
        .tonic()
        .with_endpoint(endpoint.to_string());

    let tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(exporter)
        .with_trace_config(opentelemetry_sdk::trace::Config::default().with_resource(
            opentelemetry_sdk::Resource::new(vec![KeyValue::new("service.name", "zerochain")]),
        ))
        .install_batch(opentelemetry_sdk::runtime::Tokio)
        .map_err(|e| anyhow::anyhow!("failed to init otel tracer: {e}"))?;

    Ok(tracer)
}

fn expand_data_dir(input: &str) -> String {
    if input == "~" {
        return std::env::var("HOME").unwrap_or_else(|_| input.to_string());
    }
    if let Some(rest) = input.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    input.to_string()
}

fn parse_wallet_scheme(value: &str) -> Result<WalletScheme> {
    match value.to_ascii_lowercase().as_str() {
        "ed25519" => Ok(WalletScheme::Ed25519),
        other => anyhow::bail!("unsupported wallet scheme: {other}"),
    }
}

fn map_wallet_action(action: WalletAction) -> Result<WalletCommand> {
    let cmd = match action {
        WalletAction::New {
            name,
            scheme,
            passphrase,
        } => WalletCommand::New {
            name,
            scheme: parse_wallet_scheme(&scheme)?,
            passphrase,
        },
        WalletAction::List => WalletCommand::List,
        WalletAction::Show { name } => WalletCommand::Show { name },
        WalletAction::Sign {
            name,
            message,
            passphrase,
        } => WalletCommand::Sign {
            name,
            message,
            passphrase,
        },
        WalletAction::Verify {
            name,
            message,
            signature,
        } => WalletCommand::Verify {
            name,
            message,
            signature_hex: signature,
        },
        WalletAction::Delete { name } => WalletCommand::Delete { name },
        WalletAction::RotatePassphrase {
            name,
            old_passphrase,
            new_passphrase,
        } => WalletCommand::RotatePassphrase {
            name,
            old_passphrase,
            new_passphrase,
        },
        WalletAction::Unlock {
            name,
            passphrase,
            ttl_secs,
        } => WalletCommand::Unlock {
            name,
            passphrase,
            ttl_secs,
        },
        WalletAction::MigrateV1 { passphrase } => WalletCommand::MigrateV1 { passphrase },
    };
    Ok(cmd)
}

fn parse_compute_backend(value: &str) -> Result<ComputeBackend> {
    match value.to_ascii_lowercase().as_str() {
        "mem" => Ok(ComputeBackend::Mem),
        "rocksdb" => Ok(ComputeBackend::RocksDb),
        "redb" => Ok(ComputeBackend::Redb),
        other => anyhow::bail!("unsupported compute backend: {other}"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NetworkProfile {
    Mainnet,
    Testnet,
    Devnet,
    Local,
}

impl NetworkProfile {
    fn parse(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "mainnet" => Ok(Self::Mainnet),
            "testnet" => Ok(Self::Testnet),
            "devnet" => Ok(Self::Devnet),
            "local" => Ok(Self::Local),
            other => anyhow::bail!("unsupported network profile: {other}"),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Mainnet => "mainnet",
            Self::Testnet => "testnet",
            Self::Devnet => "devnet",
            Self::Local => "local",
        }
    }

    fn apply_defaults(self, cfg: &mut ApiConfig, data_dir: &str) {
        match self {
            Self::Mainnet => {
                cfg.http_rpc.port = 8545;
                cfg.ws.port = 8546;
                cfg.http_rpc.chain_id = 10086;
                cfg.http_rpc.network_id = 10086;
                if cfg.http_rpc.compute_db_path == "./data/compute-db" {
                    cfg.http_rpc.compute_db_path = format!("{}/mainnet/compute-db", data_dir);
                }
            }
            Self::Testnet => {
                cfg.http_rpc.port = 18545;
                cfg.ws.port = 18546;
                cfg.http_rpc.chain_id = 10087;
                cfg.http_rpc.network_id = 10087;
                if cfg.http_rpc.compute_db_path == "./data/compute-db" {
                    cfg.http_rpc.compute_db_path = format!("{}/testnet/compute-db", data_dir);
                }
            }
            Self::Devnet => {
                cfg.http_rpc.port = 28545;
                cfg.ws.port = 28546;
                cfg.http_rpc.chain_id = 10088;
                cfg.http_rpc.network_id = 10088;
                if cfg.http_rpc.compute_db_path == "./data/compute-db" {
                    cfg.http_rpc.compute_db_path = format!("{}/devnet/compute-db", data_dir);
                }
            }
            Self::Local => {
                cfg.http_rpc.port = 8545;
                cfg.ws.port = 8546;
                cfg.http_rpc.chain_id = 31337;
                cfg.http_rpc.network_id = 31337;
                if cfg.http_rpc.compute_db_path == "./data/compute-db" {
                    cfg.http_rpc.compute_db_path = format!("{}/local/compute-db", data_dir);
                }
            }
        }
    }
}

fn parse_u64_decimal_or_hex(value: &str) -> Result<u64> {
    let v = value.trim();
    if let Some(hex) = v.strip_prefix("0x") {
        u64::from_str_radix(hex, 16)
            .map_err(|e| anyhow::anyhow!("invalid hex integer '{value}': {e}"))
    } else {
        v.parse::<u64>()
            .map_err(|e| anyhow::anyhow!("invalid integer '{value}': {e}"))
    }
}

fn default_rpc_url_for_network(network: &str) -> String {
    match network.to_ascii_lowercase().as_str() {
        "testnet" => "http://127.0.0.1:18545".to_string(),
        "devnet" => "http://127.0.0.1:28545".to_string(),
        "mainnet" | "local" => "http://127.0.0.1:8545".to_string(),
        _ => "http://127.0.0.1:8545".to_string(),
    }
}
