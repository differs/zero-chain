//! Transaction command implementation.

use crate::{Result, TransactionAction};
use anyhow::{anyhow, Context};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse<T> {
    result: Option<T>,
    error: Option<JsonRpcError>,
}

pub async fn handle_transaction(action: TransactionAction, rpc_url: &str) -> Result<()> {
    match action {
        TransactionAction::Get { hash } => {
            let tx = rpc_call::<serde_json::Value>(
                rpc_url,
                "eth_getTransactionByHash",
                json!([hash.clone()]),
            )
            .await?;
            println!("rpc_url: {}", rpc_url);
            println!("hash: {}", hash);
            println!(
                "transaction: {}",
                serde_json::to_string_pretty(&tx).unwrap_or_else(|_| tx.to_string())
            );
        }
        TransactionAction::Send { from, to, amount } => {
            println!("rpc_url: {}", rpc_url);
            println!("from: {}", from);
            println!("to: {}", to);
            println!("amount: {}", amount);
            anyhow::bail!(
                "transaction send is not implemented in zerocli yet; use `zerochain wallet sign` + `eth_sendRawTransaction` pipeline"
            );
        }
    }

    Ok(())
}

async fn rpc_call<T>(rpc_url: &str, method: &str, params: serde_json::Value) -> Result<T>
where
    T: DeserializeOwned,
{
    let client = reqwest::Client::new();
    let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });

    let response = client
        .post(rpc_url)
        .json(&payload)
        .send()
        .await
        .with_context(|| format!("failed to call rpc method `{method}`"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(anyhow!("rpc http status {} for `{}`", status, method));
    }

    let body: JsonRpcResponse<T> = response
        .json()
        .await
        .with_context(|| format!("failed to decode rpc response for `{method}`"))?;
    if let Some(error) = body.error {
        return Err(anyhow!(
            "rpc `{}` error {}: {}",
            method,
            error.code,
            error.message
        ));
    }

    body.result
        .ok_or_else(|| anyhow!("rpc `{}` missing result field", method))
}
