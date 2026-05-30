//! Persistent storage for UTXO Compute objects and tx results.

use std::sync::Arc;

use zerocore::compute::{
    execution::ObjectStore,
    primitives::{ObjectId, OutputId, TxId},
    ObjectOutput,
};

use crate::{db::KeyValueDB, Result, StorageError};

const OUTPUT_PREFIX: &[u8] = b"compute:output:";
const LATEST_PREFIX: &[u8] = b"compute:latest:";
const TX_RESULT_PREFIX: &[u8] = b"compute:txresult:";
const OUTPUT_BINARY_MAGIC: &[u8; 4] = b"ZCO1";

/// Statistics returned by compute store rebuilds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ComputeRebuildStats {
    /// Total key-value entries visited from the source database.
    pub total_entries: u64,
    /// Entries under the compute output namespace.
    pub output_entries: u64,
    /// Legacy JSON output entries decoded and rewritten to binary.
    pub legacy_json_outputs: u64,
    /// Already-binary output entries encountered.
    pub binary_outputs: u64,
    /// Non-output entries copied without codec changes.
    pub copied_entries: u64,
    /// Entries whose value changed in the rebuilt target database.
    pub rewritten_entries: u64,
}

/// Durable backend for compute object outputs and tx results.
pub struct ComputeStore {
    db: Arc<dyn KeyValueDB>,
}

impl ComputeStore {
    /// Creates a new compute store over key-value DB.
    pub fn new(db: Arc<dyn KeyValueDB>) -> Self {
        Self { db }
    }

    /// Saves execution result JSON by tx id.
    pub fn put_tx_result(&self, tx_id: TxId, result_json: &str) -> Result<()> {
        self.db.put(&tx_result_key(tx_id), result_json.as_bytes())
    }

    /// Loads execution result JSON by tx id.
    pub fn get_tx_result(&self, tx_id: TxId) -> Result<Option<String>> {
        let Some(bytes) = self.db.get(&tx_result_key(tx_id))? else {
            return Ok(None);
        };
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|e| StorageError::Serialization(e.to_string()))
    }
}

impl ObjectStore for ComputeStore {
    fn get_output(&self, output_id: OutputId) -> Option<ObjectOutput> {
        let key = output_key(output_id);
        let bytes = self.db.get(&key).ok().flatten()?;
        decode_output(&bytes).ok()
    }

    fn get_latest_output_by_object(&self, object_id: ObjectId) -> Option<ObjectOutput> {
        let latest_key = latest_key(object_id);
        let latest_bytes = self.db.get(&latest_key).ok().flatten()?;
        if latest_bytes.len() != 32 {
            return None;
        }
        let mut h = [0u8; 32];
        h.copy_from_slice(&latest_bytes);
        let out_id = OutputId(zerocore::crypto::Hash::from_bytes(h));
        self.get_output(out_id)
    }

    fn insert_output(&self, output: ObjectOutput) -> zerocore::compute::error::ComputeResult<()> {
        let key = output_key(output.output_id);
        if self
            .db
            .has(&key)
            .map_err(|e| zerocore::compute::ComputeError::InvalidOperation(e.to_string()))?
        {
            return Err(zerocore::compute::ComputeError::DuplicateOutputId);
        }

        let serialized = encode_output(&output)
            .map_err(|e| zerocore::compute::ComputeError::InvalidOperation(e.to_string()))?;

        self.db
            .put(&key, &serialized)
            .map_err(|e| zerocore::compute::ComputeError::InvalidOperation(e.to_string()))?;

        let latest_key = latest_key(output.object_id);
        self.db
            .put(&latest_key, output.output_id.0.as_bytes())
            .map_err(|e| zerocore::compute::ComputeError::InvalidOperation(e.to_string()))?;
        Ok(())
    }

    fn mark_spent(&self, output_id: OutputId) -> zerocore::compute::error::ComputeResult<()> {
        let Some(mut output) = self.get_output(output_id) else {
            return Err(zerocore::compute::ComputeError::ObjectNotFound(output_id.0));
        };
        if output.spent {
            return Err(zerocore::compute::ComputeError::InvalidOperation(
                "double spend detected".to_string(),
            ));
        }
        output.spent = true;
        let serialized = encode_output(&output)
            .map_err(|e| zerocore::compute::ComputeError::InvalidOperation(e.to_string()))?;
        self.db
            .put(&output_key(output_id), &serialized)
            .map_err(|e| zerocore::compute::ComputeError::InvalidOperation(e.to_string()))?;
        Ok(())
    }
}

/// Rebuilds a compute key-value database into the current on-disk format.
///
/// The target database must be distinct from the source database. Output values are decoded through
/// the backward-compatible reader and encoded with the current binary codec; all other entries are
/// copied byte-for-byte.
pub fn rebuild_compute_store(
    source: &dyn KeyValueDB,
    target: &dyn KeyValueDB,
) -> Result<ComputeRebuildStats> {
    let mut stats = ComputeRebuildStats::default();

    source.for_each_prefix(b"", &mut |key, value| {
        stats.total_entries += 1;

        if key.starts_with(OUTPUT_PREFIX) {
            stats.output_entries += 1;
            let was_binary = value.starts_with(OUTPUT_BINARY_MAGIC);
            if was_binary {
                stats.binary_outputs += 1;
            } else {
                stats.legacy_json_outputs += 1;
            }

            let output = decode_output(value)?;
            let encoded = encode_output(&output)?;
            if encoded != value {
                stats.rewritten_entries += 1;
            }
            target.put(key, &encoded)?;
        } else {
            stats.copied_entries += 1;
            target.put(key, value)?;
        }

        Ok(())
    })?;

    Ok(stats)
}

fn output_key(output_id: OutputId) -> Vec<u8> {
    let mut key = OUTPUT_PREFIX.to_vec();
    key.extend_from_slice(output_id.0.as_bytes());
    key
}

fn latest_key(object_id: ObjectId) -> Vec<u8> {
    let mut key = LATEST_PREFIX.to_vec();
    key.extend_from_slice(object_id.0.as_bytes());
    key
}

fn tx_result_key(tx_id: TxId) -> Vec<u8> {
    let mut key = TX_RESULT_PREFIX.to_vec();
    key.extend_from_slice(tx_id.0.as_bytes());
    key
}

fn encode_output(output: &ObjectOutput) -> Result<Vec<u8>> {
    let encoded = bincode::serialize(output)
        .map_err(|e| StorageError::Serialization(format!("encode output failed: {e}")))?;
    let mut value = Vec::with_capacity(OUTPUT_BINARY_MAGIC.len() + encoded.len());
    value.extend_from_slice(OUTPUT_BINARY_MAGIC);
    value.extend_from_slice(&encoded);
    Ok(value)
}

fn decode_output(bytes: &[u8]) -> Result<ObjectOutput> {
    if let Some(payload) = bytes.strip_prefix(OUTPUT_BINARY_MAGIC) {
        return bincode::deserialize::<ObjectOutput>(payload)
            .map_err(|e| StorageError::Serialization(format!("decode binary output failed: {e}")));
    }

    serde_json::from_slice::<ObjectOutput>(bytes)
        .map_err(|e| StorageError::Serialization(format!("decode legacy json output failed: {e}")))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use zerocore::compute::{
        execution::ObjectStore,
        object::{ObjectKind, Ownership, Script},
        primitives::{DomainId, ObjectId, OutputId, TxId, Version},
    };
    use zerocore::crypto::Hash;

    use crate::db::{KeyValueDB, MemDatabase};

    use super::{
        decode_output, encode_output, latest_key, output_key, rebuild_compute_store, tx_result_key,
        ComputeStore, OUTPUT_BINARY_MAGIC,
    };

    fn sample_output() -> zerocore::compute::ObjectOutput {
        zerocore::compute::ObjectOutput {
            output_id: OutputId(Hash::from_bytes([1; 32])),
            object_id: ObjectId(Hash::from_bytes([2; 32])),
            version: Version(1),
            domain_id: DomainId(0),
            kind: ObjectKind::State,
            owner: Ownership::Shared,
            predecessor: None,
            state: vec![1, 2, 3],
            state_root: None,
            resources: vec![],
            lock: Script::default(),
            logic: None,
            created_at: 0,
            ttl: None,
            rent_reserve: None,
            flags: 0,
            extensions: vec![],
            spent: false,
        }
    }

    #[test]
    fn compute_store_roundtrip() {
        let db = Arc::new(MemDatabase::new());
        let store = ComputeStore::new(db.clone());

        let output = sample_output();

        store.insert_output(output.clone()).unwrap();
        let got = store.get_output(output.output_id).unwrap();
        assert_eq!(got.object_id, output.object_id);
        let raw = db.get(&output_key(output.output_id)).unwrap().unwrap();
        assert!(raw.starts_with(OUTPUT_BINARY_MAGIC));

        let latest = store.get_latest_output_by_object(output.object_id).unwrap();
        assert_eq!(latest.output_id, output.output_id);

        store.mark_spent(output.output_id).unwrap();
        assert!(store.get_output(output.output_id).unwrap().spent);

        let tx_id = TxId(Hash::from_bytes([3; 32]));
        store.put_tx_result(tx_id, "{\"ok\":true}").unwrap();
        assert_eq!(
            store.get_tx_result(tx_id).unwrap(),
            Some("{\"ok\":true}".to_string())
        );
    }

    #[test]
    fn compute_store_reads_legacy_json_output() {
        let db = Arc::new(MemDatabase::new());
        let store = ComputeStore::new(db.clone());
        let output = sample_output();
        let legacy = serde_json::to_vec(&output).unwrap();

        db.put(&output_key(output.output_id), &legacy).unwrap();

        let got = store.get_output(output.output_id).unwrap();
        assert_eq!(got, output);
    }

    #[test]
    fn output_binary_codec_roundtrip() {
        let output = sample_output();
        let encoded = encode_output(&output).unwrap();
        assert!(encoded.starts_with(OUTPUT_BINARY_MAGIC));
        assert_eq!(decode_output(&encoded).unwrap(), output);
    }

    #[test]
    fn rebuild_compute_store_rewrites_legacy_outputs_and_copies_other_entries() {
        let source = MemDatabase::new();
        let target = MemDatabase::new();
        let output = sample_output();
        let legacy = serde_json::to_vec(&output).unwrap();
        let tx_id = TxId(Hash::from_bytes([4; 32]));

        source.put(&output_key(output.output_id), &legacy).unwrap();
        source
            .put(&latest_key(output.object_id), output.output_id.0.as_bytes())
            .unwrap();
        source
            .put(&tx_result_key(tx_id), br#"{"ok":true}"#)
            .unwrap();

        let stats = rebuild_compute_store(&source, &target).unwrap();
        assert_eq!(stats.total_entries, 3);
        assert_eq!(stats.output_entries, 1);
        assert_eq!(stats.legacy_json_outputs, 1);
        assert_eq!(stats.binary_outputs, 0);
        assert_eq!(stats.copied_entries, 2);
        assert_eq!(stats.rewritten_entries, 1);

        let raw_output = target.get(&output_key(output.output_id)).unwrap().unwrap();
        assert!(raw_output.starts_with(OUTPUT_BINARY_MAGIC));
        assert_eq!(decode_output(&raw_output).unwrap(), output);
        assert_eq!(
            target.get(&latest_key(output.object_id)).unwrap(),
            Some(output.output_id.0.as_bytes().to_vec())
        );
        assert_eq!(
            target.get(&tx_result_key(tx_id)).unwrap(),
            Some(br#"{"ok":true}"#.to_vec())
        );
    }
}
