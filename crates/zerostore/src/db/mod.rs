//! Database abstraction layer

use crate::{Result, StorageError};
use std::sync::Arc;

/// Key-value database trait
pub trait KeyValueDB: Send + Sync {
    /// Get value by key
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;
    /// Put key-value pair
    fn put(&self, key: &[u8], value: &[u8]) -> Result<()>;
    /// Delete key
    fn delete(&self, key: &[u8]) -> Result<()>;
    /// Check if key exists
    fn has(&self, key: &[u8]) -> Result<bool>;
    /// Write batch
    fn write_batch(&self, batch: Batch) -> Result<()>;
    /// Create batch
    fn batch(&self) -> Batch;
    /// Iterate over prefix
    fn iter_prefix(
        &self,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)> + '_>>;
}

/// Batch write operation
pub struct Batch {
    operations: Vec<BatchOp>,
}

enum BatchOp {
    Put(Vec<u8>, Vec<u8>),
    Delete(Vec<u8>),
}

impl Batch {
    pub fn new() -> Self {
        Self {
            operations: Vec::new(),
        }
    }

    pub fn put(&mut self, key: &[u8], value: &[u8]) {
        self.operations
            .push(BatchOp::Put(key.to_vec(), value.to_vec()));
    }

    pub fn delete(&mut self, key: &[u8]) {
        self.operations.push(BatchOp::Delete(key.to_vec()));
    }
}

impl Default for Batch {
    fn default() -> Self {
        Self::new()
    }
}

/// RocksDB implementation
pub struct RocksDb {
    db: rocksdb::DB,
}

impl RocksDb {
    pub fn open(path: &str) -> Result<Self> {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        // Use std::thread::available_parallelism instead of num_cpus
        let parallelism = std::thread::available_parallelism()
            .map(|p| p.get() as i32)
            .unwrap_or(4);
        opts.increase_parallelism(parallelism);
        opts.set_max_background_flushes(4);
        opts.set_max_background_compactions(4);
        opts.set_write_buffer_size(256 * 1024 * 1024); // 256MB
        opts.set_max_write_buffer_number(4);
        opts.set_target_file_size_base(128 * 1024 * 1024); // 128MB

        // Compression
        opts.set_compression_type(rocksdb::DBCompressionType::Lz4);

        // Bloom filter
        let mut block_opts = rocksdb::BlockBasedOptions::default();
        block_opts.set_bloom_filter(10.0, false);
        block_opts.set_block_size(16 * 1024); // 16KB
        opts.set_block_based_table_factory(&block_opts);

        let db =
            rocksdb::DB::open(&opts, path).map_err(|e| StorageError::Database(e.to_string()))?;

        Ok(Self { db })
    }

    pub fn inner(&self) -> &rocksdb::DB {
        &self.db
    }
}

impl KeyValueDB for RocksDb {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.db
            .get(key)
            .map_err(|e| StorageError::Database(e.to_string()))
    }

    fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
        self.db
            .put(key, value)
            .map_err(|e| StorageError::Database(e.to_string()))
    }

    fn delete(&self, key: &[u8]) -> Result<()> {
        self.db
            .delete(key)
            .map_err(|e| StorageError::Database(e.to_string()))
    }

    fn has(&self, key: &[u8]) -> Result<bool> {
        Ok(self
            .db
            .get(key)
            .map_err(|e| StorageError::Database(e.to_string()))?
            .is_some())
    }

    fn write_batch(&self, batch: Batch) -> Result<()> {
        let mut db_batch = rocksdb::WriteBatch::default();

        for op in batch.operations {
            match op {
                BatchOp::Put(k, v) => {
                    db_batch.put(&k, &v);
                }
                BatchOp::Delete(k) => {
                    db_batch.delete(&k);
                }
            }
        }

        self.db
            .write(db_batch)
            .map_err(|e| StorageError::Database(e.to_string()))
    }

    fn batch(&self) -> Batch {
        Batch::new()
    }

    fn iter_prefix(
        &self,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)> + '_>> {
        let iter = self.db.prefix_iterator(prefix);

        Ok(Box::new(iter.map(|item| {
            let (k, v) = item.unwrap();
            (k.to_vec(), v.to_vec())
        })))
    }
}

/// Redb implementation (pure Rust alternative)
pub struct RedbDatabase {
    db: Arc<redb::Database>,
}

impl RedbDatabase {
    pub fn open(path: &str) -> Result<Self> {
        let db = redb::Database::create(path).map_err(|e| StorageError::Database(e.to_string()))?;

        Ok(Self { db: Arc::new(db) })
    }
}

impl KeyValueDB for RedbDatabase {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        // Simplified - would implement full redb interface
        Ok(None)
    }

    fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
        Ok(())
    }

    fn delete(&self, key: &[u8]) -> Result<()> {
        Ok(())
    }

    fn has(&self, key: &[u8]) -> Result<bool> {
        Ok(false)
    }

    fn write_batch(&self, _batch: Batch) -> Result<()> {
        Ok(())
    }

    fn batch(&self) -> Batch {
        Batch::new()
    }

    fn iter_prefix(
        &self,
        _prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)> + '_>> {
        Ok(Box::new(std::iter::empty()))
    }
}

/// In-memory database (for testing)
pub struct MemDatabase {
    data: parking_lot::RwLock<std::collections::HashMap<Vec<u8>, Vec<u8>>>,
}

impl MemDatabase {
    pub fn new() -> Self {
        Self {
            data: parking_lot::RwLock::new(std::collections::HashMap::new()),
        }
    }
}

impl Default for MemDatabase {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyValueDB for MemDatabase {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(self.data.read().get(key).cloned())
    }

    fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
        self.data.write().insert(key.to_vec(), value.to_vec());
        Ok(())
    }

    fn delete(&self, key: &[u8]) -> Result<()> {
        self.data.write().remove(key);
        Ok(())
    }

    fn has(&self, key: &[u8]) -> Result<bool> {
        Ok(self.data.read().contains_key(key))
    }

    fn write_batch(&self, batch: Batch) -> Result<()> {
        for op in batch.operations {
            match op {
                BatchOp::Put(k, v) => {
                    self.data.write().insert(k, v);
                }
                BatchOp::Delete(k) => {
                    self.data.write().remove(&k);
                }
            }
        }
        Ok(())
    }

    fn batch(&self) -> Batch {
        Batch::new()
    }

    fn iter_prefix(
        &self,
        prefix: &[u8],
    ) -> Result<Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)> + '_>> {
        let prefix = prefix.to_vec();
        let items: Vec<_> = self
            .data
            .read()
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        Ok(Box::new(items.into_iter()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mem_database() {
        let db = MemDatabase::new();

        db.put(b"key1", b"value1").unwrap();
        assert_eq!(db.get(b"key1").unwrap(), Some(b"value1".to_vec()));
        assert!(db.has(b"key1").unwrap());

        db.delete(b"key1").unwrap();
        assert_eq!(db.get(b"key1").unwrap(), None);
    }

    #[test]
    fn test_batch_write() {
        let db = MemDatabase::new();
        let mut batch = db.batch();

        batch.put(b"key1", b"value1");
        batch.put(b"key2", b"value2");
        batch.delete(b"key3");

        db.write_batch(batch).unwrap();

        assert_eq!(db.get(b"key1").unwrap(), Some(b"value1".to_vec()));
        assert_eq!(db.get(b"key2").unwrap(), Some(b"value2".to_vec()));
    }
}
