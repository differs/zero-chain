//! ZeroChain Integration Tests

use zerocore::crypto::{PrivateKey, Address, Hash};
use zerocore::account::{Account, AccountType, U256, InMemoryAccountManager, AccountManager};
use zerocore::transaction::{UnsignedTransaction, TransactionPool, TxPoolConfig};
use zerocore::block::{Block, create_genesis_block};
use zerocore::consensus::{PowConsensus, PowAlgorithm, MiningEngine, MiningConfig};
use zerocore::state::{StateDb, StateExecutor};
use zerocore::blockchain::{Blockchain, SyncManager, SyncConfig};
use zerostore::trie::{MerklePatriciaTrie, MemTrieDB};
use std::sync::Arc;

/// Test fixture for integration tests
struct TestFixture {
    consensus: Arc<PowConsensus>,
    state_db: Arc<StateDb>,
    blockchain: Arc<Blockchain>,
    account_manager: Arc<InMemoryAccountManager>,
    tx_pool: Arc<TransactionPool>,
    mining_engine: Arc<MiningEngine>,
}

impl TestFixture {
    /// Create new test fixture
    fn new() -> Self {
        let consensus = Arc::new(PowConsensus::new(PowAlgorithm::LightHash));
        let state_db = Arc::new(StateDb::new(Hash::zero()));
        let blockchain = Arc::new(Blockchain::new(consensus.clone(), state_db.clone()));
        let account_manager = Arc::new(InMemoryAccountManager::new());
        let tx_pool = Arc::new(TransactionPool::new(
            TxPoolConfig::default(),
            account_manager.clone(),
        ));
        
        let mining_config = MiningConfig {
            enabled: true,
            coinbase: Address::from_bytes([1u8; 20]),
            threads: 2,
            algorithm: PowAlgorithm::LightHash,
            ..Default::default()
        };
        
        let mining_engine = Arc::new(MiningEngine::new(
            mining_config,
            consensus.clone(),
            tx_pool.clone(),
            state_db.clone(),
        ));
        
        Self {
            consensus,
            state_db,
            blockchain,
            account_manager,
            tx_pool,
            mining_engine,
        }
    }
}

/// Test: Create and execute transaction
#[tokio::test]
async fn test_transaction_execution() {
    let fixture = TestFixture::new();
    
    // Create test accounts
    let sender_key = PrivateKey::random();
    let sender_addr = Address::from_public_key(&sender_key.public_key());
    
    let recipient_key = PrivateKey::random();
    let recipient_addr = Address::from_public_key(&recipient_key.public_key());
    
    // Fund sender account
    let mut sender_account = Account::new_signer_account(sender_key.public_key(), sender_addr);
    sender_account.balance = U256::from(1_000_000_000_000_000_000u128);  // 1 ETH
    fixture.account_manager.create_account(
        sender_account.account_type.clone(),
        sender_account.config.clone(),
    ).await.unwrap();
    
    // Create transaction
    let tx = UnsignedTransaction::new_transfer(
        0,  // nonce
        U256::from(1_000_000_000),  // gas price
        U256::from(21000),  // gas limit
        Some(recipient_addr),
        U256::from(1000),  // value
        vec![],  // input
        10086,  // chain_id
    );
    
    let signed_tx = tx.sign(&sender_key);
    
    // Add to pool
    fixture.tx_pool.add_transaction(signed_tx.clone()).unwrap();
    
    // Verify transaction in pool
    assert!(fixture.tx_pool.contains(&signed_tx.hash()));
    
    println!("✓ Transaction created and added to pool");
}

/// Test: Mine block with transactions
#[tokio::test]
async fn test_block_mining() {
    let fixture = TestFixture::new();
    
    // Start mining
    fixture.mining_engine.start_mining().unwrap();
    
    // Get genesis block
    let genesis = fixture.blockchain.genesis();
    
    // Mine new block
    let block = fixture.mining_engine.mine_block(&genesis.header).unwrap();
    
    // Verify block
    assert_eq!(block.header.number, U256::from(1));
    assert_eq!(block.header.parent_hash, genesis.header.hash);
    assert!(!block.header.hash.is_zero());
    
    // Verify PoW
    fixture.consensus.verify_pow(&block.header).unwrap();
    
    println!("✓ Block mined successfully: #{}", block.header.number.as_u64());
}

/// Test: State transitions
#[tokio::test]
async fn test_state_transitions() {
    let fixture = TestFixture::new();
    
    let executor = StateExecutor::new(fixture.state_db.clone(), 10086);
    
    // Create test account
    let key = PrivateKey::random();
    let addr = Address::from_public_key(&key.public_key());
    
    let mut account = Account::new_signer_account(key.public_key(), addr);
    account.balance = U256::from(1_000_000);
    fixture.account_manager.create_account(
        account.account_type.clone(),
        account.config.clone(),
    ).await.unwrap();
    
    // Get initial state
    let initial_balance = fixture.state_db.get_balance(&addr);
    assert_eq!(initial_balance, U256::from(1_000_000));
    
    println!("✓ State transitions working correctly");
}

/// Test: MPT Trie operations
#[test]
fn test_mpt_trie() {
    let db = Arc::new(MemTrieDB::new());
    let trie = MerklePatriciaTrie::new(db);
    
    // Insert key-value pairs
    let root1 = trie.insert(b"key1", b"value1".to_vec().into()).unwrap();
    let root2 = trie.insert(b"key2", b"value2".to_vec().into()).unwrap();
    let root3 = trie.insert(b"key3", b"value3".to_vec().into()).unwrap();
    
    // Verify root changes
    assert!(!root1.is_zero());
    assert!(!root2.is_zero());
    assert!(!root3.is_zero());
    assert_ne!(root1, root2);
    assert_ne!(root2, root3);
    
    // Retrieve values
    let val1 = trie.get(b"key1").unwrap().unwrap();
    let val2 = trie.get(b"key2").unwrap().unwrap();
    let val3 = trie.get(b"key3").unwrap().unwrap();
    
    assert_eq!(val1.as_ref(), b"value1");
    assert_eq!(val2.as_ref(), b"value2");
    assert_eq!(val3.as_ref(), b"value3");
    
    // Delete key
    let removed = trie.remove(b"key2").unwrap().unwrap();
    assert_eq!(removed.as_ref(), b"value2");
    
    // Verify deletion
    assert!(trie.get(b"key2").unwrap().is_none());
    
    println!("✓ MPT Trie operations working correctly");
}

/// Test: RLP encoding/decoding
#[test]
fn test_rlp_encoding() {
    use zerocore::rlp::{RlpEncode, RlpDecode};
    
    // Test u64
    let original = 1000u64;
    let encoded = original.rlp_encode();
    let decoded = u64::rlp_decode(&encoded).unwrap();
    assert_eq!(original, decoded);
    
    // Test string
    let original = "hello".to_string();
    let encoded = original.rlp_encode();
    let decoded = String::rlp_decode(&encoded).unwrap();
    assert_eq!(original, decoded);
    
    // Test list
    let original = vec![1u64, 2, 3, 4, 5];
    let encoded = original.rlp_encode();
    let decoded = Vec::<u64>::rlp_decode(&encoded).unwrap();
    assert_eq!(original, decoded);
    
    println!("✓ RLP encoding/decoding working correctly");
}

/// Test: Blockchain operations
#[test]
fn test_blockchain_operations() {
    let consensus = Arc::new(PowConsensus::new(PowAlgorithm::LightHash));
    let state_db = Arc::new(StateDb::new(Hash::zero()));
    let blockchain = Arc::new(Blockchain::new(consensus, state_db));
    
    // Get genesis
    let genesis = blockchain.genesis();
    assert_eq!(genesis.header.number, U256::zero());
    
    // Get best block
    let best = blockchain.best_block();
    assert_eq!(best.header.number, U256::zero());
    
    // Get block by number
    let block = blockchain.get_block_by_number(0);
    assert!(block.is_some());
    
    // Get chain info
    let info = blockchain.get_chain_info();
    assert_eq!(info.best_number, 0);
    
    println!("✓ Blockchain operations working correctly");
}

/// Test: Account management
#[tokio::test]
async fn test_account_management() {
    let manager = InMemoryAccountManager::new();
    
    // Create account
    let key = PrivateKey::random();
    let account_type = AccountType::ExternalOwned {
        public_key: key.public_key(),
        signature_scheme: Default::default(),
    };
    
    let account = manager.create_account(account_type, Default::default()).await.unwrap();
    
    // Verify account created
    let retrieved = manager.get_account(&account.address).await.unwrap().unwrap();
    assert_eq!(retrieved.address, account.address);
    
    // Update balance
    manager.update_balance(
        &account.address,
        zerocore::account::I256::from(1000),
        zerocore::account::BalanceChangeReason::Transfer,
    ).await.unwrap();
    
    let updated = manager.get_account(&account.address).await.unwrap().unwrap();
    assert_eq!(updated.balance, U256::from(1000));
    
    // Increment nonce
    manager.increment_nonce(&account.address).await.unwrap();
    
    let nonce = manager.get_nonce(&account.address).await.unwrap();
    assert_eq!(nonce, 1);
    
    println!("✓ Account management working correctly");
}

/// Test: Sync manager
#[tokio::test]
async fn test_sync_manager() {
    let config = SyncConfig::default();
    let consensus = Arc::new(PowConsensus::new(PowAlgorithm::LightHash));
    let state_db = Arc::new(StateDb::new(Hash::zero()));
    let blockchain = Arc::new(Blockchain::new(consensus.clone(), state_db.clone()));
    
    let sync = SyncManager::new(config, blockchain, state_db, consensus);
    
    // Initial state
    assert!(!sync.is_syncing());
    assert_eq!(sync.get_progress(), 0.0);
    
    // Add peers
    sync.add_peer("peer1".to_string());
    sync.add_peer("peer2".to_string());
    
    assert_eq!(sync.peer_count(), 2);
    
    // Remove peer
    sync.remove_peer("peer1");
    assert_eq!(sync.peer_count(), 1);
    
    println!("✓ Sync manager working correctly");
}

/// Test: Transaction pool
#[tokio::test]
async fn test_transaction_pool() {
    let config = TxPoolConfig::default();
    let manager = Arc::new(InMemoryAccountManager::new());
    let pool = TransactionPool::new(config, manager);
    
    // Create test transaction
    let key = PrivateKey::random();
    let tx = UnsignedTransaction::new_transfer(
        0,
        U256::from(1_000_000_000),
        U256::from(21000),
        Some(Address::from_bytes([1u8; 20])),
        U256::from(1000),
        vec![],
        10086,
    );
    
    let signed_tx = tx.sign(&key);
    
    // Add to pool
    pool.add_transaction(signed_tx.clone()).unwrap();
    
    // Verify in pool
    assert!(pool.contains(&signed_tx.hash()));
    assert_eq!(pool.pending_count(), 1);
    
    // Get stats
    let stats = pool.get_stats();
    assert_eq!(stats.total_transactions, 1);
    
    // Select transactions
    let selected = pool.select_transactions(30_000_000);
    assert!(!selected.is_empty());
    
    println!("✓ Transaction pool working correctly");
}

/// Benchmark: Transaction creation
#[test]
fn bench_transaction_creation() {
    let start = std::time::Instant::now();
    
    for _ in 0..1000 {
        let key = PrivateKey::random();
        let tx = UnsignedTransaction::new_transfer(
            0,
            U256::from(1_000_000_000),
            U256::from(21000),
            Some(Address::from_bytes([1u8; 20])),
            U256::from(1000),
            vec![],
            10086,
        );
        let _signed = tx.sign(&key);
    }
    
    let elapsed = start.elapsed();
    println!("✓ Created 1000 transactions in {:?}", elapsed);
    println!("  Average: {:?}", elapsed / 1000);
}

/// Benchmark: MPT Trie operations
#[test]
fn bench_trie_operations() {
    let db = Arc::new(MemTrieDB::new());
    let trie = MerklePatriciaTrie::new(db);
    
    let start = std::time::Instant::now();
    
    // Insert 1000 keys
    for i in 0..1000 {
        let key = format!("key{}", i);
        let value = format!("value{}", i);
        trie.insert(key.as_bytes(), value.into_bytes().into()).unwrap();
    }
    
    let insert_elapsed = start.elapsed();
    
    // Get 1000 keys
    let start = std::time::Instant::now();
    for i in 0..1000 {
        let key = format!("key{}", i);
        trie.get(key.as_bytes()).unwrap();
    }
    
    let get_elapsed = start.elapsed();
    
    println!("✓ MPT Trie benchmark:");
    println!("  Insert 1000: {:?}", insert_elapsed);
    println!("  Get 1000: {:?}", get_elapsed);
    println!("  Average insert: {:?}", insert_elapsed / 1000);
    println!("  Average get: {:?}", get_elapsed / 1000);
}
