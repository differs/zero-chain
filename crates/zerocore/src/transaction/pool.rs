//! Transaction Pool Implementation
//!
//! Manages pending transactions with:
//! - Priority queue based on gas price
//! - Nonce management per account
//! - Pool size limits
//! - Transaction validation

use super::{SignedTransaction, TransactionError};
use crate::account::{Account, AccountManager, U256};
use crate::block::BlockHeader;
use crate::crypto::{Address, Hash};
use parking_lot::RwLock;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

/// Transaction pool configuration
#[derive(Clone, Debug)]
pub struct TxPoolConfig {
    /// Maximum number of transactions in pool
    pub max_transactions: usize,
    /// Maximum number of transactions per account
    pub max_per_account: usize,
    /// Minimum gas price to accept transaction
    pub min_gas_price: U256,
    /// Transaction lifetime in seconds
    pub tx_lifetime: u64,
    /// Maximum transaction size in bytes
    pub max_tx_size: usize,
}

impl Default for TxPoolConfig {
    fn default() -> Self {
        Self {
            max_transactions: 10_000,
            max_per_account: 100,
            min_gas_price: U256::from(1_000_000_000), // 1 Gwei
            tx_lifetime: 3600,                        // 1 hour
            max_tx_size: 128 * 1024,                  // 128 KB
        }
    }
}

/// Transaction with metadata
#[derive(Clone, Debug)]
pub struct PoolTransaction {
    /// Signed transaction
    pub tx: SignedTransaction,
    /// Time added to pool (timestamp)
    pub added_at: u64,
    /// Gas price (for priority)
    pub gas_price: U256,
    /// Priority score
    pub priority: u64,
}

impl PoolTransaction {
    pub fn new(tx: SignedTransaction, added_at: u64) -> Self {
        let gas_price = tx.tx.effective_gas_price(None);

        Self {
            tx,
            added_at,
            gas_price,
            priority: 0,
        }
    }

    /// Calculate priority score
    pub fn calculate_priority(&mut self, base_fee: U256) {
        // Priority = gas_price - base_fee + time_bonus
        let gas_bonus = self.gas_price.saturating_sub(base_fee).as_u64();
        let time_bonus = 0; // Could add time-based priority

        self.priority = gas_bonus + time_bonus;
    }

    pub fn gas_limit(&self) -> U256 {
        self.tx.gas_limit()
    }
}

impl PartialEq for PoolTransaction {
    fn eq(&self, other: &Self) -> bool {
        self.tx.hash == other.tx.hash
    }
}

impl Eq for PoolTransaction {}

impl PartialOrd for PoolTransaction {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PoolTransaction {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Higher priority first
        // Then higher gas price
        // Then earlier addition time
        self.priority
            .cmp(&other.priority)
            .then_with(|| self.gas_price.cmp(&other.gas_price))
            .then_with(|| other.added_at.cmp(&self.added_at))
    }
}

/// Per-account transaction queue
#[derive(Clone, Debug)]
pub struct AccountQueue {
    /// Transactions ordered by nonce
    transactions: BTreeSet<PoolTransaction>,
    /// Current nonce
    current_nonce: u64,
    /// Total gas used by pending transactions
    total_gas: U256,
}

impl AccountQueue {
    pub fn new(current_nonce: u64) -> Self {
        Self {
            transactions: BTreeSet::new(),
            current_nonce,
            total_gas: U256::zero(),
        }
    }

    /// Add transaction to queue
    pub fn add(&mut self, tx: PoolTransaction) -> Result<(), TransactionError> {
        // Check nonce is sequential
        let expected_nonce = self.current_nonce + self.transactions.len() as u64;

        if tx.tx.nonce() < expected_nonce {
            return Err(TransactionError::InvalidNonce {
                expected: expected_nonce,
                got: tx.tx.nonce(),
            });
        }

        self.total_gas = self.total_gas + tx.gas_limit();
        self.transactions.insert(tx);

        Ok(())
    }

    /// Get executable transactions (nonce matches)
    pub fn get_executable(&self) -> Vec<&PoolTransaction> {
        let mut executables = Vec::new();

        for tx in &self.transactions {
            if tx.tx.nonce() == self.current_nonce {
                executables.push(tx);
            } else {
                break;
            }
        }

        executables
    }

    /// Remove transaction by nonce
    pub fn remove(&mut self, nonce: u64) -> Option<PoolTransaction> {
        if let Some(tx) = self.transactions.iter().find(|t| t.tx.nonce() == nonce) {
            let tx = tx.clone();
            self.transactions.remove(&tx);
            self.total_gas = self.total_gas - tx.gas_limit();

            if nonce == self.current_nonce {
                self.current_nonce += 1;
            }

            Some(tx)
        } else {
            None
        }
    }

    /// Clear all transactions
    pub fn clear(&mut self) {
        self.transactions.clear();
        self.total_gas = U256::zero();
    }

    /// Get transaction count
    pub fn len(&self) -> usize {
        self.transactions.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.transactions.is_empty()
    }
}

/// Transaction pool statistics
#[derive(Clone, Debug, Default)]
pub struct TxPoolStats {
    /// Total transactions in pool
    pub total_transactions: usize,
    /// Total accounts with pending transactions
    pub total_accounts: usize,
    /// Total size in bytes
    pub total_size: usize,
    /// Average gas price
    pub avg_gas_price: U256,
    /// Maximum gas price
    pub max_gas_price: U256,
    /// Minimum gas price
    pub min_gas_price: U256,
}

/// Transaction Pool
pub struct TransactionPool {
    /// Pool configuration
    config: TxPoolConfig,
    /// All pending transactions by hash
    all_transactions: RwLock<HashMap<Hash, PoolTransaction>>,
    /// Per-account queues
    account_queues: RwLock<HashMap<Address, AccountQueue>>,
    /// Transaction hashes by account
    account_txs: RwLock<HashMap<Address, HashSet<Hash>>>,
    /// Known transactions (to prevent duplicates)
    known_transactions: RwLock<HashSet<Hash>>,
    /// Account manager reference
    account_manager: Arc<dyn AccountManager>,
    /// Current base fee
    base_fee: RwLock<U256>,
    /// Pool statistics
    stats: RwLock<TxPoolStats>,
}

impl TransactionPool {
    /// Create new transaction pool
    pub fn new(config: TxPoolConfig, account_manager: Arc<dyn AccountManager>) -> Self {
        Self {
            config,
            all_transactions: RwLock::new(HashMap::new()),
            account_queues: RwLock::new(HashMap::new()),
            account_txs: RwLock::new(HashMap::new()),
            known_transactions: RwLock::new(HashSet::new()),
            account_manager,
            base_fee: RwLock::new(U256::from(1_000_000_000)),
            stats: RwLock::new(TxPoolStats::default()),
        }
    }

    /// Add transaction to pool
    pub fn add_transaction(&self, tx: SignedTransaction) -> Result<Hash, TransactionError> {
        let tx_hash = tx.hash();

        // Check if already known
        if self.known_transactions.read().contains(&tx_hash) {
            return Err(TransactionError::InvalidNonce {
                expected: 0, // Would get actual nonce
                got: tx.nonce(),
            });
        }

        // Validate transaction
        self.validate_transaction(&tx)?;

        // Check pool size
        if self.all_transactions.read().len() >= self.config.max_transactions {
            self.remove_worst_transactions(1);
        }

        // Create pool transaction
        let now = current_timestamp();
        let mut pool_tx = PoolTransaction::new(tx, now);
        pool_tx.calculate_priority(*self.base_fee.read());

        let sender = pool_tx.tx.sender();

        // Add to account queue
        {
            let mut queues = self.account_queues.write();
            let queue = queues.entry(sender).or_insert_with(|| {
                AccountQueue::new(0) // Would get actual nonce
            });

            if queue.len() >= self.config.max_per_account {
                return Err(TransactionError::TooLarge { size: queue.len() });
            }

            queue.add(pool_tx.clone())?;
        }

        // Add to all transactions
        self.all_transactions
            .write()
            .insert(tx_hash, pool_tx.clone());

        // Add to account transactions
        self.account_txs
            .write()
            .entry(sender)
            .or_default()
            .insert(tx_hash);

        // Add to known transactions
        self.known_transactions.write().insert(tx_hash);

        // Update statistics
        self.update_stats();

        Ok(tx_hash)
    }

    /// Add multiple transactions
    pub fn add_transactions(
        &self,
        txs: Vec<SignedTransaction>,
    ) -> Result<Vec<Hash>, TransactionError> {
        let mut hashes = Vec::with_capacity(txs.len());

        for tx in txs {
            match self.add_transaction(tx) {
                Ok(hash) => hashes.push(hash),
                Err(e) => {
                    // Continue adding other transactions
                    tracing::warn!("Failed to add transaction: {}", e);
                }
            }
        }

        Ok(hashes)
    }

    /// Validate transaction
    fn validate_transaction(&self, tx: &SignedTransaction) -> Result<(), TransactionError> {
        // Check transaction size
        let encoded_size = tx.encode_rlp().len();
        if encoded_size > self.config.max_tx_size {
            return Err(TransactionError::TooLarge { size: encoded_size });
        }

        // Check gas price
        let gas_price = tx.tx.effective_gas_price(None);
        if gas_price < self.config.min_gas_price {
            return Err(TransactionError::GasPriceTooLow);
        }

        // Check chain ID
        if tx.tx.chain_id != 10086 {
            // Would use actual chain ID
            return Err(TransactionError::InvalidChainId);
        }

        // Verify signature
        if !tx.verify_signature()? {
            return Err(TransactionError::InvalidSignature);
        }

        // Check account balance and nonce (simplified, no async call)
        // In production, this would check against actual state
        let _ = self.account_manager;

        Ok(())
    }

    /// Remove transaction from pool
    pub fn remove_transaction(&self, tx_hash: &Hash) -> Option<PoolTransaction> {
        if let Some(tx) = self.all_transactions.write().remove(tx_hash) {
            let sender = tx.tx.sender();

            // Remove from account queue
            if let Some(queue) = self.account_queues.write().get_mut(&sender) {
                queue.remove(tx.tx.nonce());
            }

            // Remove from account transactions
            if let Some(tx_set) = self.account_txs.write().get_mut(&sender) {
                tx_set.remove(tx_hash);
            }

            // Update statistics
            self.update_stats();

            Some(tx)
        } else {
            None
        }
    }

    /// Remove transactions by account
    pub fn remove_account_transactions(&self, address: &Address) -> Vec<PoolTransaction> {
        let mut removed = Vec::new();

        if let Some(tx_hashes) = self.account_txs.write().remove(address) {
            for hash in tx_hashes {
                if let Some(tx) = self.all_transactions.write().remove(&hash) {
                    removed.push(tx);
                }
            }
        }

        self.account_queues.write().remove(address);

        self.update_stats();

        removed
    }

    /// Get transaction by hash
    pub fn get_transaction(&self, tx_hash: &Hash) -> Option<SignedTransaction> {
        self.all_transactions
            .read()
            .get(tx_hash)
            .map(|tx| tx.tx.clone())
    }

    /// Check if transaction exists
    pub fn contains(&self, tx_hash: &Hash) -> bool {
        self.all_transactions.read().contains_key(tx_hash)
    }

    /// Get executable transactions for block
    pub fn select_transactions(&self, gas_limit: u64) -> Vec<SignedTransaction> {
        let mut selected = Vec::new();
        let mut total_gas = 0u64;

        // Get all executable transactions
        let mut executables = Vec::new();

        for queue in self.account_queues.read().values() {
            for tx in queue.get_executable() {
                executables.push(tx.clone());
            }
        }

        // Sort by priority
        executables.sort_by(|a, b| b.cmp(a));

        // Select transactions up to gas limit
        for tx in executables {
            let tx_gas = tx.gas_limit().as_u64();

            if total_gas + tx_gas <= gas_limit {
                selected.push(tx.tx);
                total_gas += tx_gas;
            }
        }

        selected
    }

    /// Get pending transaction count
    pub fn pending_count(&self) -> usize {
        self.all_transactions.read().len()
    }

    /// Get pool statistics
    pub fn get_stats(&self) -> TxPoolStats {
        self.stats.read().clone()
    }

    /// Update base fee
    pub fn set_base_fee(&self, base_fee: U256) {
        *self.base_fee.write() = base_fee;

        // Recalculate priorities
        for tx in self.all_transactions.write().values_mut() {
            tx.calculate_priority(base_fee);
        }

        self.update_stats();
    }

    /// Remove worst transactions (for pool size management)
    fn remove_worst_transactions(&self, count: usize) {
        let mut all_txs: Vec<_> = self.all_transactions.read().values().cloned().collect();

        // Sort by priority (lowest first)
        all_txs.sort();

        // Remove lowest priority transactions
        for tx in all_txs.into_iter().take(count) {
            self.remove_transaction(&tx.tx.hash);
        }
    }

    /// Update pool statistics
    fn update_stats(&self) {
        let transactions = self.all_transactions.read();

        if transactions.is_empty() {
            *self.stats.write() = TxPoolStats::default();
            return;
        }

        let total = transactions.len();
        let accounts = self.account_queues.read().len();

        let total_size: usize = transactions
            .values()
            .map(|tx| tx.tx.encode_rlp().len())
            .sum();

        let mut total_gas_price = U256::zero();
        let mut max_gas_price = U256::zero();
        let mut min_gas_price = U256::from_u128(u128::MAX);

        for tx in transactions.values() {
            total_gas_price = total_gas_price + tx.gas_price;
            if tx.gas_price > max_gas_price {
                max_gas_price = tx.gas_price;
            }
            if tx.gas_price < min_gas_price {
                min_gas_price = tx.gas_price;
            }
        }

        let avg_gas_price = {
            let mut sum: u128 = 0;
            for tx in transactions.values() {
                sum = sum.saturating_add(tx.gas_price.as_u64() as u128);
            }
            let avg = if total == 0 { 0 } else { sum / total as u128 };
            U256::from(avg as u64)
        };

        *self.stats.write() = TxPoolStats {
            total_transactions: total,
            total_accounts: accounts,
            total_size,
            avg_gas_price,
            max_gas_price,
            min_gas_price,
        };
    }

    /// Clear old transactions
    pub fn clear_old_transactions(&self) {
        let now = current_timestamp();
        let lifetime = self.config.tx_lifetime;

        let to_remove: Vec<_> = self
            .all_transactions
            .read()
            .iter()
            .filter(|(_, tx)| now - tx.added_at > lifetime)
            .map(|(hash, _)| *hash)
            .collect();

        for hash in to_remove {
            self.remove_transaction(&hash);
        }
    }

    /// Clear pool
    pub fn clear(&self) {
        self.all_transactions.write().clear();
        self.account_queues.write().clear();
        self.account_txs.write().clear();
        self.known_transactions.write().clear();
        self.update_stats();
    }
}

fn current_timestamp() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::InMemoryAccountManager;
    use crate::crypto::PrivateKey;
    use crate::transaction::UnsignedTransaction;

    #[tokio::test]
    async fn test_add_transaction() {
        let config = TxPoolConfig::default();
        let manager = Arc::new(InMemoryAccountManager::new());
        let pool = TransactionPool::new(config, manager);

        // Create test transaction
        let private_key = PrivateKey::random();
        let tx = UnsignedTransaction::new_transfer(
            0,
            U256::from(1_000_000_000),
            U256::from(21000),
            Some(Address::from_bytes([1u8; 20])),
            U256::from(1000),
            vec![],
            10086,
        );

        let signed_tx = tx.sign(&private_key);

        // Add to pool
        let result = pool.add_transaction(signed_tx);
        assert!(result.is_ok());

        // Check pool size
        assert_eq!(pool.pending_count(), 1);

        // Get transaction
        let hash = result.unwrap();
        let retrieved = pool.get_transaction(&hash);
        assert!(retrieved.is_some());
    }

    #[tokio::test]
    async fn test_select_transactions() {
        let config = TxPoolConfig::default();
        let manager = Arc::new(InMemoryAccountManager::new());
        let pool = TransactionPool::new(config, manager);

        // Add multiple transactions
        for i in 0..5 {
            let private_key = PrivateKey::random();
            let tx = UnsignedTransaction::new_transfer(
                0,
                U256::from(1_000_000_000 + i * 100_000_000),
                U256::from(21000),
                Some(Address::from_bytes([1u8; 20])),
                U256::from(1000),
                vec![],
                10086,
            );

            let signed_tx = tx.sign(&private_key);
            pool.add_transaction(signed_tx).unwrap();
        }

        // Select transactions
        let selected = pool.select_transactions(30_000_000);
        assert!(!selected.is_empty());
    }

    #[tokio::test]
    async fn test_pool_stats() {
        let config = TxPoolConfig::default();
        let manager = Arc::new(InMemoryAccountManager::new());
        let pool = TransactionPool::new(config, manager);

        // Add transaction
        let private_key = PrivateKey::random();
        let tx = UnsignedTransaction::new_transfer(
            0,
            U256::from(1_000_000_000),
            U256::from(21000),
            Some(Address::from_bytes([1u8; 20])),
            U256::from(1000),
            vec![],
            10086,
        );

        let signed_tx = tx.sign(&private_key);
        pool.add_transaction(signed_tx).unwrap();

        // Get stats
        let stats = pool.get_stats();
        assert_eq!(stats.total_transactions, 1);
        assert_eq!(stats.total_accounts, 1);
    }
}
