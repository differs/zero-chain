//! Init command - Placeholder

use crate::Result;
use std::fs;
use std::path::Path;
use zeroapi::ApiConfig;

pub fn init_data_dir(_data_dir: &str) -> Result<()> {
    println!("Init command not fully implemented");
    Ok(())
}

/// Load API config from JSON file.
pub fn load_api_config(path: &str) -> Result<ApiConfig> {
    let content = fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read config file {}: {}", path, e))?;
    let cfg: ApiConfig = serde_json::from_str(&content)
        .map_err(|e| anyhow::anyhow!("failed to parse config JSON {}: {}", path, e))?;
    Ok(cfg)
}

/// Write default API config to a JSON file.
pub fn write_default_api_config(path: &str) -> Result<()> {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("failed to create config directory: {}", e))?;
        }
    }

    let default_cfg = ApiConfig::default();
    let json = serde_json::to_string_pretty(&default_cfg)
        .map_err(|e| anyhow::anyhow!("failed to serialize default API config: {}", e))?;
    fs::write(path, json)
        .map_err(|e| anyhow::anyhow!("failed to write config file {}: {}", path, e))?;
    Ok(())
}
