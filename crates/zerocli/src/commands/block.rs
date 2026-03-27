//! Block query commands.

use crate::commands::rpc::rpc_call;
use crate::{BlockAction, Result};
use serde_json::json;

pub async fn handle_block(action: BlockAction, rpc_url: &str) -> Result<()> {
    match action {
        BlockAction::Latest => {
            let block =
                rpc_call::<serde_json::Value>(rpc_url, "zero_getLatestBlock", json!([])).await?;

            println!("rpc_url: {}", rpc_url);
            println!(
                "block: {}",
                serde_json::to_string_pretty(&block).unwrap_or_else(|_| block.to_string())
            );
        }
        BlockAction::Get { number } => {
            let number_hex = format!("0x{number:x}");
            let block = rpc_call::<serde_json::Value>(
                rpc_url,
                "zero_getBlockByNumber",
                json!([number_hex]),
            )
            .await?;

            println!("rpc_url: {}", rpc_url);
            println!("number: {}", number);
            println!(
                "block: {}",
                serde_json::to_string_pretty(&block).unwrap_or_else(|_| block.to_string())
            );
        }
    }

    Ok(())
}
