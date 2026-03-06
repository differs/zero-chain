//! Wallet command implementation (native ed25519 + EVM secp256k1).

use crate::Result;
use chrono::Utc;
use ed25519_dalek::{Signer as _, Verifier as _};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use zerocore::crypto::{keccak256, Address, PrivateKey as SecpPrivateKey, Signature as SecpSignature};

#[derive(Debug, Clone)]
pub enum WalletCommand {
    New {
        name: Option<String>,
        scheme: WalletScheme,
    },
    List,
    Show {
        name: String,
    },
    Sign {
        name: String,
        message: String,
    },
    Verify {
        name: String,
        message: String,
        signature_hex: String,
    },
    Delete {
        name: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WalletScheme {
    Ed25519,
    Secp256k1,
}

#[derive(Debug, Serialize, Deserialize)]
struct WalletFile {
    version: u32,
    created_at: String,
    default: Option<String>,
    accounts: Vec<WalletAccount>,
}

impl Default for WalletFile {
    fn default() -> Self {
        Self {
            version: 1,
            created_at: Utc::now().to_rfc3339(),
            default: None,
            accounts: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct WalletAccount {
    id: String,
    name: String,
    scheme: WalletScheme,
    created_at: String,
    public_key_hex: String,
    private_key_hex: String,
    address: Option<String>,
}

pub async fn handle_wallet(data_dir: &str, cmd: WalletCommand) -> Result<()> {
    let path = wallet_file_path(data_dir);
    let mut wallet = load_wallet_file(&path)?;

    match cmd {
        WalletCommand::New { name, scheme } => {
            let account_name = name.unwrap_or_else(|| default_name(scheme, wallet.accounts.len()));
            if wallet.accounts.iter().any(|a| a.name == account_name) {
                anyhow::bail!("wallet account already exists: {}", account_name);
            }

            let account = match scheme {
                WalletScheme::Ed25519 => new_ed25519_account(account_name),
                WalletScheme::Secp256k1 => new_secp256k1_account(account_name)?,
            };

            if wallet.default.is_none() {
                wallet.default = Some(account.name.clone());
            }
            print_account(&account);
            wallet.accounts.push(account);
            save_wallet_file(&path, &wallet)?;
            println!("✅ wallet saved: {}", path.display());
        }
        WalletCommand::List => {
            if wallet.accounts.is_empty() {
                println!("No wallet accounts found.");
                return Ok(());
            }
            println!("Wallet file: {}", path.display());
            for a in &wallet.accounts {
                let default_mark = if wallet.default.as_deref() == Some(a.name.as_str()) {
                    " (default)"
                } else {
                    ""
                };
                println!(
                    "- {} [{}]{}\n  pubkey: {}\n  address: {}",
                    a.name,
                    scheme_name(a.scheme),
                    default_mark,
                    a.public_key_hex,
                    a.address.clone().unwrap_or_else(|| "n/a".to_string())
                );
            }
        }
        WalletCommand::Show { name } => {
            let account = wallet
                .accounts
                .iter()
                .find(|a| a.name == name)
                .ok_or_else(|| anyhow::anyhow!("wallet account not found: {}", name))?;
            print_account(account);
        }
        WalletCommand::Sign { name, message } => {
            let account = find_account(&wallet, &name)?;
            let msg_bytes = message.as_bytes();
            match account.scheme {
                WalletScheme::Ed25519 => {
                    let priv_bytes = parse_fixed_32_hex(&account.private_key_hex)?;
                    let signing = ed25519_dalek::SigningKey::from_bytes(&priv_bytes);
                    let sig = signing.sign(msg_bytes);
                    println!("scheme: ed25519");
                    println!("message: {}", message);
                    println!("signature_hex: 0x{}", hex::encode(sig.to_bytes()));
                }
                WalletScheme::Secp256k1 => {
                    let priv_bytes = parse_fixed_32_hex(&account.private_key_hex)?;
                    let secp = SecpPrivateKey::from_bytes(priv_bytes)
                        .map_err(|_| anyhow::anyhow!("invalid secp256k1 private key"))?;
                    let sig = secp.sign(msg_bytes);
                    println!("scheme: secp256k1");
                    println!("message: {}", message);
                    println!("message_keccak256: 0x{}", hex::encode(keccak256(msg_bytes)));
                    println!("signature_hex: 0x{}", hex::encode(sig.as_bytes()));
                }
            }
        }
        WalletCommand::Verify {
            name,
            message,
            signature_hex,
        } => {
            let account = find_account(&wallet, &name)?;
            let msg_bytes = message.as_bytes();
            let sig_bytes = parse_hex(signature_hex)?;

            match account.scheme {
                WalletScheme::Ed25519 => {
                    if sig_bytes.len() != 64 {
                        anyhow::bail!("ed25519 signature must be 64 bytes");
                    }
                    let pub_bytes = parse_fixed_32_hex(&account.public_key_hex)?;
                    let pubkey = ed25519_dalek::VerifyingKey::from_bytes(&pub_bytes)
                        .map_err(|e| anyhow::anyhow!("invalid ed25519 public key: {e}"))?;
                    let sig = ed25519_dalek::Signature::from_slice(&sig_bytes)
                        .map_err(|e| anyhow::anyhow!("invalid ed25519 signature: {e}"))?;
                    match pubkey.verify(msg_bytes, &sig) {
                        Ok(_) => println!("✅ verify ok (ed25519)"),
                        Err(_) => println!("❌ verify failed (ed25519)"),
                    }
                }
                WalletScheme::Secp256k1 => {
                    let sig = SecpSignature::from_bytes(&sig_bytes)
                        .map_err(|_| anyhow::anyhow!("invalid secp256k1 signature bytes"))?;
                    let recovered = sig.recover(msg_bytes);
                    match recovered {
                        Ok(pubk) => {
                            let expected_pub = parse_hex(account.public_key_hex.clone())?;
                            if expected_pub == pubk.as_bytes() {
                                println!("✅ verify ok (secp256k1)");
                            } else {
                                println!("❌ verify failed (secp256k1, pubkey mismatch)");
                            }
                        }
                        Err(_) => println!("❌ verify failed (secp256k1)"),
                    }
                }
            }
        }
        WalletCommand::Delete { name } => {
            let before = wallet.accounts.len();
            wallet.accounts.retain(|a| a.name != name);
            if wallet.accounts.len() == before {
                anyhow::bail!("wallet account not found: {}", name);
            }
            if wallet.default.as_deref() == Some(name.as_str()) {
                wallet.default = wallet.accounts.first().map(|a| a.name.clone());
            }
            save_wallet_file(&path, &wallet)?;
            println!("✅ deleted wallet account: {}", name);
        }
    }

    Ok(())
}

fn find_account<'a>(wallet: &'a WalletFile, name: &str) -> Result<&'a WalletAccount> {
    wallet
        .accounts
        .iter()
        .find(|a| a.name == name)
        .ok_or_else(|| anyhow::anyhow!("wallet account not found: {}", name).into())
}

fn new_ed25519_account(name: String) -> WalletAccount {
    let signing = ed25519_dalek::SigningKey::generate(&mut OsRng);
    let verify = signing.verifying_key();

    let public = verify.to_bytes();
    let address = native_address_from_public_key(&public);

    WalletAccount {
        id: Uuid::new_v4().to_string(),
        name,
        scheme: WalletScheme::Ed25519,
        created_at: Utc::now().to_rfc3339(),
        public_key_hex: hex::encode(public),
        private_key_hex: hex::encode(signing.to_bytes()),
        address: Some(format!("native1{}", hex::encode(address))),
    }
}

fn new_secp256k1_account(name: String) -> Result<WalletAccount> {
    let private_key = SecpPrivateKey::random();
    let public_key = private_key.public_key();
    let address = Address::from_public_key(&public_key);

    Ok(WalletAccount {
        id: Uuid::new_v4().to_string(),
        name,
        scheme: WalletScheme::Secp256k1,
        created_at: Utc::now().to_rfc3339(),
        public_key_hex: hex::encode(public_key.as_bytes()),
        private_key_hex: hex::encode(private_key.as_bytes()),
        address: Some(address.to_checksum_hex()),
    })
}

fn native_address_from_public_key(pubkey: &[u8; 32]) -> [u8; 20] {
    let mut hasher = Keccak256::new();
    hasher.update(pubkey);
    let out: [u8; 32] = hasher.finalize().into();
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&out[12..]);
    addr
}

fn wallet_file_path(data_dir: &str) -> PathBuf {
    Path::new(data_dir).join("wallet.json")
}

fn load_wallet_file(path: &Path) -> Result<WalletFile> {
    if !path.exists() {
        return Ok(WalletFile::default());
    }
    let content = fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read wallet file {}: {}", path.display(), e))?;
    let wallet: WalletFile = serde_json::from_str(&content)
        .map_err(|e| anyhow::anyhow!("failed to parse wallet file {}: {}", path.display(), e))?;
    Ok(wallet)
}

fn save_wallet_file(path: &Path, wallet: &WalletFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("failed to create wallet dir {}: {}", parent.display(), e))?;
    }
    let json = serde_json::to_string_pretty(wallet)
        .map_err(|e| anyhow::anyhow!("failed to encode wallet file: {}", e))?;
    fs::write(path, json)
        .map_err(|e| anyhow::anyhow!("failed to write wallet file {}: {}", path.display(), e))?;
    Ok(())
}

fn default_name(scheme: WalletScheme, idx: usize) -> String {
    match scheme {
        WalletScheme::Ed25519 => format!("native-{}", idx + 1),
        WalletScheme::Secp256k1 => format!("evm-{}", idx + 1),
    }
}

fn parse_hex(s: String) -> Result<Vec<u8>> {
    let raw = s.trim().strip_prefix("0x").unwrap_or(s.trim());
    hex::decode(raw).map_err(|e| anyhow::anyhow!("invalid hex: {}", e).into())
}

fn parse_fixed_32_hex(s: &str) -> Result<[u8; 32]> {
    let raw = s.trim().strip_prefix("0x").unwrap_or(s.trim());
    let bytes = hex::decode(raw).map_err(|e| anyhow::anyhow!("invalid hex: {}", e))?;
    if bytes.len() != 32 {
        anyhow::bail!("expected 32-byte key, got {} bytes", bytes.len());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn print_account(a: &WalletAccount) {
    println!("name: {}", a.name);
    println!("scheme: {}", scheme_name(a.scheme));
    println!("public_key: 0x{}", a.public_key_hex);
    if let Some(addr) = &a.address {
        println!("address: {}", addr);
    }
}

fn scheme_name(s: WalletScheme) -> &'static str {
    match s {
        WalletScheme::Ed25519 => "ed25519",
        WalletScheme::Secp256k1 => "secp256k1",
    }
}
