//! Storage maintenance commands.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use zeroapi::rpc::ComputeBackend;
use zerostore::compute::{rebuild_compute_store, ComputeRebuildStats};
use zerostore::db::{KeyValueDB, RedbDatabase, RocksDb};

/// Rebuilds an existing compute database into the current storage format.
pub fn rebuild_compute_db(
    backend: ComputeBackend,
    path: &str,
    discard_backup: bool,
) -> Result<ComputeRebuildStats> {
    match backend {
        ComputeBackend::Mem => bail!("cannot rebuild in-memory compute backend"),
        ComputeBackend::RocksDb | ComputeBackend::Redb => {}
    }

    let source_path = Path::new(path);
    if !source_path.exists() {
        bail!("compute database path does not exist: {path}");
    }

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?
        .as_millis();
    let target_path = format!("{path}.rebuild-{suffix}");
    let backup_path = format!("{path}.backup-{suffix}");

    ensure_absent(&target_path)?;
    ensure_absent(&backup_path)?;

    let stats = {
        let source = open_compute_backend(backend, path)?;
        let target = open_compute_backend(backend, &target_path)?;
        rebuild_compute_store(source.as_ref(), target.as_ref())
            .with_context(|| format!("failed to rebuild compute database from {path}"))?
    };

    fs::rename(path, &backup_path)
        .with_context(|| format!("failed to move old compute database to {backup_path}"))?;
    if let Err(err) = fs::rename(&target_path, path) {
        if let Err(rollback_err) = fs::rename(&backup_path, path) {
            bail!(
                "failed to install rebuilt compute database ({err}); rollback also failed ({rollback_err}); backup remains at {backup_path}, rebuilt database remains at {target_path}"
            );
        }
        bail!("failed to install rebuilt compute database: {err}; restored original database");
    }

    if discard_backup {
        remove_path(&backup_path)
            .with_context(|| format!("failed to remove backup at {backup_path}"))?;
    }

    println!("Compute DB rebuild complete");
    println!("   backend: {}", backend.as_str());
    println!("   path: {path}");
    println!(
        "   backup: {}",
        if discard_backup {
            "discarded".to_string()
        } else {
            backup_path.clone()
        }
    );
    println!("   entries: {}", stats.total_entries);
    println!("   output entries: {}", stats.output_entries);
    println!("   legacy json outputs: {}", stats.legacy_json_outputs);
    println!("   binary outputs: {}", stats.binary_outputs);
    println!("   copied entries: {}", stats.copied_entries);
    println!("   rewritten entries: {}", stats.rewritten_entries);

    Ok(stats)
}

fn open_compute_backend(backend: ComputeBackend, path: &str) -> Result<Arc<dyn KeyValueDB>> {
    match backend {
        ComputeBackend::Mem => bail!("cannot open in-memory compute backend for rebuild"),
        ComputeBackend::RocksDb => {
            let db =
                RocksDb::open(path).with_context(|| format!("failed to open rocksdb at {path}"))?;
            Ok(Arc::new(db))
        }
        ComputeBackend::Redb => {
            let db = RedbDatabase::open(path)
                .with_context(|| format!("failed to open redb at {path}"))?;
            Ok(Arc::new(db))
        }
    }
}

fn ensure_absent(path: &str) -> Result<()> {
    if Path::new(path).exists() {
        bail!("refusing to overwrite existing path: {path}");
    }
    Ok(())
}

fn remove_path(path: &str) -> Result<()> {
    let metadata = fs::metadata(path)?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}
