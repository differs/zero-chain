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
        serde_json::from_slice::<ObjectOutput>(&bytes).ok()
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
            .map_err(|e| zerocore::compute::ComputeError::InvalidTransaction(e.to_string()))?
        {
            return Err(zerocore::compute::ComputeError::DuplicateOutputId);
        }

        let serialized = serde_json::to_vec(&output)
            .map_err(|e| zerocore::compute::ComputeError::InvalidTransaction(e.to_string()))?;

        self.db
            .put(&key, &serialized)
            .map_err(|e| zerocore::compute::ComputeError::InvalidTransaction(e.to_string()))?;

        let latest_key = latest_key(output.object_id);
        self.db
            .put(&latest_key, output.output_id.0.as_bytes())
            .map_err(|e| zerocore::compute::ComputeError::InvalidTransaction(e.to_string()))?;
        Ok(())
    }

    fn mark_spent(&self, output_id: OutputId) -> zerocore::compute::error::ComputeResult<()> {
        let Some(mut output) = self.get_output(output_id) else {
            return Err(zerocore::compute::ComputeError::ObjectNotFound(output_id.0));
        };
        if output.spent {
            return Err(zerocore::compute::ComputeError::InvalidTransaction(
                "double spend detected".to_string(),
            ));
        }
        output.spent = true;
        let serialized = serde_json::to_vec(&output)
            .map_err(|e| zerocore::compute::ComputeError::InvalidTransaction(e.to_string()))?;
        self.db
            .put(&output_key(output_id), &serialized)
            .map_err(|e| zerocore::compute::ComputeError::InvalidTransaction(e.to_string()))?;
        Ok(())
    }
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use zerocore::compute::{
        execution::ObjectStore,
        object::{ObjectKind, Ownership, Script},
        primitives::{DomainId, ObjectId, OutputId, TxId, Version},
    };
    use zerocore::crypto::Hash;

    use crate::db::MemDatabase;

    use super::ComputeStore;

    #[test]
    fn compute_store_roundtrip() {
        let db = Arc::new(MemDatabase::new());
        let store = ComputeStore::new(db);

        let output = zerocore::compute::ObjectOutput {
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
        };

        store.insert_output(output.clone()).unwrap();
        let got = store.get_output(output.output_id).unwrap();
        assert_eq!(got.object_id, output.object_id);

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
}
