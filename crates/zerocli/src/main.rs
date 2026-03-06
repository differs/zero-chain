//! ZeroChain CLI - Blockchain node and client

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use zeroapi::rpc::{ComputeBackend, RpcConfig};
use zeroapi::ApiConfig;


mod commands;

/// ZeroChain CLI
#[derive(Parser)]
#[command(name = "zerocchain")]
#[command(author = "ZeroChain Team")]
#[command(version = "0.1.0")]
#[command(about = "ZeroChain blockchain node and client", long_about = None)]
struct Cli {
    /// Log level
    #[arg(short, long, default_value = "info")]
    log_level: String,
    
    /// Data directory
    #[arg(short, long, default_value = "~/.zerocchain")]
    data_dir: String,
    
    /// Network ID
    #[arg(short, long, default_value = "10086")]
    network_id: u64,

    /// Optional node config file (JSON)
    #[arg(long)]
    config: Option<String>,
    
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
}

#[derive(Subcommand)]
#[derive(Debug)]
enum AccountAction {
    /// Create new account
    New,
    /// List accounts
    List,
    /// Get account balance
    Balance {
        #[arg(short, long)]
        address: String,
    },
}

#[derive(Subcommand)]
#[derive(Debug)]
enum TransactionAction {
    /// Send transaction
    Send {
        #[arg(short, long)]
        from: String,
        #[arg(short, long)]
        to: String,
        #[arg(short, long)]
        amount: String,
    },
    /// Get transaction by hash
    Get {
        #[arg(short, long)]
        hash: String,
    },
}

#[derive(Subcommand)]
#[derive(Debug)]
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
    
    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("info,zeroccore={},zeronet={},zeroapi={}", 
                    cli.log_level, cli.log_level, cli.log_level).into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
    
    match cli.command {
        Some(Commands::Run { mine, coinbase, http_port, ws_port, compute_backend, compute_db_path }) => {
            let mut api_config = if let Some(path) = &cli.config {
                commands::init::load_api_config(path)?
            } else {
                ApiConfig::default()
            };

            // CLI flags override config file.
            let backend = parse_compute_backend(&compute_backend)?;
            api_config.http_rpc = RpcConfig {
                address: "127.0.0.1".to_string(),
                port: http_port,
                compute_backend: backend,
                compute_db_path,
                ..api_config.http_rpc
            };
            api_config.ws.port = ws_port;

            commands::run::run_node(mine, coinbase, http_port, ws_port, &cli.data_dir, Some(api_config.http_rpc.clone())).await?;
        }
        Some(Commands::Init) => {
            commands::init::init_data_dir(&cli.data_dir)?;
            let cfg_path = format!("{}/api-config.json", &cli.data_dir);
            commands::init::write_default_api_config(&cfg_path)?;
            println!("Default API config written to {}", cfg_path);
        }
        Some(Commands::Account { action }) => {
            commands::account::handle_account(format!("{:?}", action)).await?;
        }
        Some(Commands::Transaction { action }) => {
            commands::transaction::handle_transaction(format!("{:?}", action)).await?;
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
        None => {
            // Default: show help
            println!("ZeroChain v{}", env!("CARGO_PKG_VERSION"));
            println!("Use --help for usage information");
        }
    }
    
    Ok(())
}

fn parse_compute_backend(value: &str) -> Result<ComputeBackend> {
    match value.to_ascii_lowercase().as_str() {
        "mem" => Ok(ComputeBackend::Mem),
        "rocksdb" => Ok(ComputeBackend::RocksDb),
        "redb" => Ok(ComputeBackend::Redb),
        other => anyhow::bail!("unsupported compute backend: {other}"),
    }
}
