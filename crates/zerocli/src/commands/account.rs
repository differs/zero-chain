//! Account command implementation.

use crate::{AccountAction, Result};
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

pub async fn handle_account(action: AccountAction, rpc_url: &str) -> Result<()> {
    match action {
        AccountAction::New => {
            anyhow::bail!(
                "account new is not implemented yet; use `zerochain wallet new --scheme secp256k1|ed25519`"
            );
        }
        AccountAction::List => {
            anyhow::bail!(
                "account list is not implemented yet; use `zerochain wallet list` for local wallet accounts"
            );
        }
        AccountAction::Balance { address } => {
            let balance_hex: String =
                rpc_call(rpc_url, "eth_getBalance", json!([address, "latest"])).await?;
            let account_info =
                rpc_call::<serde_json::Value>(rpc_url, "zero_getAccount", json!([address])).await;

            println!("rpc_url: {}", rpc_url);
            println!("address: {}", address);
            println!("balance: {}", balance_hex);
            if let Ok(value) = account_info {
                if !value.is_null() {
                    println!(
                        "account: {}",
                        serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
                    );
                }
            }
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
