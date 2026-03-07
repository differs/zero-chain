//! Shared JSON-RPC client helpers for zerocli commands.

use crate::Result;
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

pub async fn rpc_call<T>(rpc_url: &str, method: &str, params: serde_json::Value) -> Result<T>
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
            "{}",
            format_rpc_error(method, error.code, &error.message)
        ));
    }

    body.result
        .ok_or_else(|| anyhow!("rpc `{}` missing result field", method))
}

fn format_rpc_error(method: &str, code: i64, message: &str) -> String {
    if code == -32010 && method == "eth_sendRawTransaction" {
        return "当前节点默认关闭 eth_sendRawTransaction。请在开发环境使用 --rpc-enable-eth-write-rpcs 启动节点。".to_string();
    }

    format!("rpc `{}` error {}: {}", method, code, message)
}
