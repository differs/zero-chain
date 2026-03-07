//! Transaction command implementation.

use crate::commands::rpc::rpc_call;
use crate::commands::wallet::{self, WalletScheme};
use crate::{Result, TransactionAction};
use anyhow::{anyhow, Context};
use serde::Serialize;
use serde_json::json;
use zerocore::crypto::PrivateKey as SecpPrivateKey;

pub async fn handle_transaction(
    action: TransactionAction,
    data_dir: &str,
    rpc_url: &str,
) -> Result<()> {
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
        TransactionAction::Send {
            from,
            to,
            amount,
            account_name,
            passphrase,
            nonce,
            gas_limit,
            gas_price,
            data,
        } => {
            let signing = wallet::load_signing_context(
                data_dir,
                account_name.as_deref(),
                Some(from.as_str()),
                passphrase.as_deref(),
            )?;
            if signing.scheme != WalletScheme::Secp256k1 {
                anyhow::bail!(
                    "transaction send requires secp256k1 account, got {:?}",
                    signing.scheme
                );
            }
            if signing
                .address
                .as_deref()
                .map(|value| !value.eq_ignore_ascii_case(from.as_str()))
                .unwrap_or(true)
            {
                anyhow::bail!(
                    "wallet account `{}` does not match --from address {}",
                    signing.account_name,
                    from
                );
            }

            let chain_id_hex: String = rpc_call(rpc_url, "eth_chainId", json!([])).await?;
            let chain_id = parse_hex_u64(&chain_id_hex).context("invalid chain id from rpc")?;

            let nonce_value = match nonce {
                Some(value) => value,
                None => {
                    let nonce_hex: String =
                        rpc_call(rpc_url, "eth_getTransactionCount", json!([from, "pending"]))
                            .await?;
                    parse_hex_u64(&nonce_hex).context("invalid nonce from rpc")?
                }
            };

            let gas_price_wei = match gas_price {
                Some(value) => parse_u128_decimal_or_hex(&value)
                    .with_context(|| format!("invalid --gas-price value `{value}`"))?,
                None => {
                    let gas_price_hex: String =
                        rpc_call(rpc_url, "eth_gasPrice", json!([])).await?;
                    parse_u128_decimal_or_hex(&gas_price_hex)
                        .context("invalid gas price from rpc")?
                }
            };
            let value_wei = parse_u128_decimal_or_hex(&amount)
                .with_context(|| format!("invalid --amount value `{amount}`"))?;
            let normalized_data = data.as_deref().map(normalize_hex);

            let payload = LocalUnsignedTx {
                from: from.clone(),
                to: to.clone(),
                value_wei: format!("0x{:x}", value_wei),
                nonce: nonce_value,
                gas_limit,
                gas_price_wei: format!("0x{:x}", gas_price_wei),
                chain_id,
                data: normalized_data,
            };
            let payload_bytes =
                serde_json::to_vec(&payload).context("failed to encode tx signing payload")?;

            let signing_key = SecpPrivateKey::from_bytes(signing.secret)
                .map_err(|_| anyhow!("invalid secp256k1 private key"))?;
            let signature = signing_key.sign(&payload_bytes);
            let public_key = signing_key.public_key();

            let raw = LocalSignedRawTx {
                payload,
                signature: format!("0x{}", hex::encode(signature.as_bytes())),
                public_key: format!("0x{}", hex::encode(public_key.as_bytes())),
                signer: signing.account_name.clone(),
            };
            let raw_bytes = serde_json::to_vec(&raw).context("failed to encode raw tx")?;
            let raw_hex = format!("0x{}", hex::encode(raw_bytes));

            let tx_hash: String =
                rpc_call(rpc_url, "eth_sendRawTransaction", json!([raw_hex.clone()])).await?;

            println!("rpc_url: {}", rpc_url);
            println!("from: {}", from);
            println!("to: {}", to);
            println!("nonce: {}", nonce_value);
            println!("gas_limit: {}", gas_limit);
            println!("gas_price_wei: 0x{:x}", gas_price_wei);
            println!("value_wei: 0x{:x}", value_wei);
            if let Some(data_hex) = raw.payload.data.as_deref() {
                println!("data: {}", data_hex);
            }
            println!("signer_account: {}", signing.account_name);
            println!("raw_tx: {}", raw_hex);
            println!("tx_hash: {}", tx_hash);
        }
    }

    Ok(())
}

#[derive(Debug, Serialize)]
struct LocalUnsignedTx {
    from: String,
    to: String,
    value_wei: String,
    nonce: u64,
    gas_limit: u64,
    gas_price_wei: String,
    chain_id: u64,
    data: Option<String>,
}

#[derive(Debug, Serialize)]
struct LocalSignedRawTx {
    payload: LocalUnsignedTx,
    signature: String,
    public_key: String,
    signer: String,
}

fn normalize_hex(value: &str) -> String {
    if value.starts_with("0x") {
        value.to_string()
    } else {
        format!("0x{}", value)
    }
}

fn parse_hex_u64(value: &str) -> Result<u64> {
    let normalized = value.trim().strip_prefix("0x").unwrap_or(value.trim());
    u64::from_str_radix(normalized, 16).map_err(|e| anyhow!("invalid hex u64 `{value}`: {e}"))
}

fn parse_u128_decimal_or_hex(value: &str) -> Result<u128> {
    let normalized = value.trim();
    if let Some(hex) = normalized.strip_prefix("0x") {
        u128::from_str_radix(hex, 16).map_err(|e| anyhow!("invalid hex integer `{value}`: {e}"))
    } else {
        normalized
            .parse::<u128>()
            .map_err(|e| anyhow!("invalid decimal integer `{value}`: {e}"))
    }
}
