//! 真实磁盘 I/O 基准测试
//! 
//! 模拟生产环境： RocksDB/Redb 存储 + 真实余额检查

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use zerocore::crypto::{PrivateKey, Address, Hash};
use zerocore::account::{U256, InMemoryAccountManager, Account};
use zerocore::transaction::{UnsignedTransaction, SignedTransaction, TransactionPool};
use zerocore::transaction::pool::TxPoolConfig;
use zerocore::state::StateDb;
use std::sync::Arc;
use std::time::Duration;

/// 创建测试环境 (带真实状态数据库)
fn setup_realistic_environment(tx_count: usize) -> (StateDb, TransactionPool, Vec<SignedTransaction>) {
    // 创建真实的状态数据库 (会使用 RocksDB/Redb)
    let state_db = StateDb::new(Hash::zero());
    
    // 创建账户管理器
    let account_manager = Arc::new(InMemoryAccountManager::new());
    
    // 创建交易池
    let pool = TransactionPool::new(
        TxPoolConfig::default(),
        account_manager.clone(),
    );
    
    // 预创建账户和余额 (模拟真实场景)
    for i in 0..tx_count {
        let address = Address::from_bytes([i as u8; 20]);
        let account = Account {
            address,
            balance: U256::from(1_000_000_000_000u64), // 足够余额
            nonce: i as u64,
            ..Default::default()
        };
        // 写入数据库 (真实 I/O)
        let _ = state_db.insert_account(address, account);
    }
    
    // 创建测试交易
    let mut txs = Vec::new();
    for i in 0..tx_count {
        let private_key = PrivateKey::random();
        let tx = UnsignedTransaction::new_legacy(
            i as u64,
            U256::from(1_000_000_000),
            U256::from(21000),
            Some(Address::from_bytes([(i + 1) as u8; 20])),
            U256::from(1000),
            vec![],
            10086,
        );
        
        let signed_tx = tx.sign(&private_key);
        txs.push(signed_tx);
    }
    
    (state_db, pool, txs)
}

/// 完整交易执行 (真实 I/O)
fn execute_transactions_realistic(
    state_db: &StateDb,
    pool: &TransactionPool,
    txs: &[SignedTransaction],
) -> usize {
    let mut executed = 0;
    
    for tx in txs {
        // 1. 验证签名 (CPU 密集)
        if tx.verify_signature().is_err() {
            continue;
        }
        
        // 2. 检查余额 (需要数据库查询 - 真实 I/O)
        let sender = tx.sender();
        if let Some(account) = state_db.get_account(&sender) {
            if account.balance < tx.value() {
                continue; // 余额不足
            }
        } else {
            continue; // 账户不存在
        }
        
        // 3. 检查 nonce (内存 + 数据库)
        if let Some(account) = state_db.get_account(&sender) {
            if tx.nonce() < account.nonce {
                continue; // nonce 过小
            }
        }
        
        // 4. 添加到交易池 (内存操作)
        if pool.add_transaction(tx.clone()).is_err() {
            continue;
        }
        
        // 5. 更新状态 (需要数据库写入 - 真实 I/O)
        // 这里简化，实际会更复杂
        
        executed += 1;
    }
    
    executed
}

/// 基准测试：真实 I/O 环境
fn bench_realistic_io(c: &mut Criterion) {
    let mut group = c.benchmark_group("realistic_io");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(120)); // 增加测试时间
    group.warm_up_time(Duration::from_secs(10));
    
    for tx_count in [100, 500, 1000].iter() {
        println!("\n📊 准备 {} 笔交易 (真实 I/O)...", tx_count);
        let (state_db, pool, txs) = setup_realistic_environment(*tx_count);
        println!("✅ 准备完成");
        
        group.throughput(Throughput::Elements(*tx_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_txs_realistic", tx_count)),
            &txs,
            |b, txs| {
                b.iter(|| {
                    black_box(execute_transactions_realistic(&state_db, &pool, txs));
                });
            },
        );
    }
    
    group.finish();
}

/// 基准测试：纯内存对比
fn bench_memory_only(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_only");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));
    
    for tx_count in [100, 500, 1000].iter() {
        let (_, pool, txs) = setup_realistic_environment(*tx_count);
        
        group.throughput(Throughput::Elements(*tx_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_txs_memory", tx_count)),
            &txs,
            |b, txs| {
                b.iter(|| {
                    for tx in txs {
                        black_box(tx.verify_signature().is_ok());
                        black_box(pool.add_transaction(tx.clone()).is_ok());
                    }
                });
            },
        );
    }
    
    group.finish();
}

criterion_group!(
    name = benches;
    config = Criterion::default().noise_threshold(0.2);
    targets = bench_realistic_io, bench_memory_only
);
criterion_main!(benches);
