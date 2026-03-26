//! Compute command implementation.

use crate::commands::rpc::rpc_call;
use crate::{ComputeAction, Result};
use anyhow::Context;
use serde_json::json;
use std::fs;

pub async fn handle_compute(action: ComputeAction, _data_dir: &str, rpc_url: &str) -> Result<()> {
    match action {
        ComputeAction::Get { tx_id } => {
            let result = rpc_call::<serde_json::Value>(
                rpc_url,
                "zero_getComputeTxResult",
                json!([tx_id.clone()]),
            )
            .await?;
            println!("rpc_url: {}", rpc_url);
            println!("tx_id: {}", tx_id);
            println!(
                "result: {}",
                serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
            );
        }
        ComputeAction::Send {
            tx_file,
            account_name,
            passphrase,
        } => {
            let raw = fs::read_to_string(&tx_file)
                .with_context(|| format!("failed to read tx file `{}`", tx_file))?;
            let tx_value: serde_json::Value = serde_json::from_str(&raw)
                .with_context(|| format!("failed to parse tx json from `{}`", tx_file))?;

            if !tx_value.is_object() {
                anyhow::bail!("compute payload must be a JSON object");
            }

            let result = rpc_call::<serde_json::Value>(
                rpc_url,
                "zero_submitComputeTx",
                json!([tx_value.clone()]),
            )
            .await?;

            println!("rpc_url: {}", rpc_url);
            println!("tx_file: {}", tx_file);
            println!("account_name: {}", account_name);
            if passphrase.is_some() {
                println!("passphrase: provided");
            } else {
                println!("passphrase: not provided");
            }
            println!(
                "submit_result: {}",
                serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
            );
        }
    }

    Ok(())
}
