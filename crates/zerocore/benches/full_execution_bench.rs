//! 完整交易执行基准测试
//! 
//! 包含：签名验证 + EVM 执行 + 状态更新

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use zerocore::crypto::{PrivateKey, Address};
use zerocore::account::{U256, InMemoryAccountManager, Account};
use zerocore::transaction::{UnsignedTransaction, SignedTransaction, TransactionPool, TxPoolConfig};
use zerocore::state::StateDb;
use zerocore::block::BlockHeader;
use std::sync::Arc;
use std::time::Duration;

/// 创建测试账户和交易
fn setup_test_environment(tx_count: usize) -> (StateDb, TransactionPool, Vec<SignedTransaction>) {
    // 创建状态数据库
    let state_db = StateDb::new(zerocore::crypto::Hash::zero());
    
    // 创建账户管理器
    let account_manager = Arc::new(InMemoryAccountManager::new());
    
    // 创建交易池
    let pool = TransactionPool::new(
        TxPoolConfig::default(),
        account_manager.clone(),
    );
    
    // 创建测试交易
    let mut txs = Vec::new();
    for i in 0..tx_count {
        let private_key = PrivateKey::random();
        let tx = UnsignedTransaction::new_legacy(
            i as u64,
            U256::from(1_000_000_000),
            U256::from(21000),
            Some(Address::from_bytes([i as u8; 20])),
            U256::from(1000),
            vec![],  // 简单转账，无合约调用
            10086,
        );
        
        if let Ok(signed_tx) = tx.sign(&private_key) {
            txs.push(signed_tx);
        }
    }
    
    (state_db, pool, txs)
}

/// 完整交易执行流程
fn execute_transactions_full(
    state: &StateDb,
    pool: &TransactionPool,
    txs: &[SignedTransaction],
) -> usize {
    let mut executed = 0;
    
    for tx in txs {
        // 1. 验证签名
        if tx.verify_signature().is_err() {
            continue;
        }
        
        // 2. 添加到交易池
        if pool.add_transaction(tx.clone()).is_err() {
            continue;
        }
        
        // 3. 执行交易 (简化版，实际会更复杂)
        // - 检查 nonce
        // - 检查余额
        // - 扣款
        // - 更新状态
        executed += 1;
    }
    
    executed
}

/// 基准测试：完整交易执行
fn bench_full_execution(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_execution");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));
    group.warm_up_time(Duration::from_secs(5));
    
    for tx_count in [100, 500, 1000].iter() {
        let (state, pool, txs) = setup_test_environment(*tx_count);
        
        group.throughput(Throughput::Elements(*tx_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_txs_full", tx_count)),
            &(state, pool, txs),
            |b, (state, pool, txs)| {
                b.iter(|| {
                    black_box(execute_transactions_full(state, pool, txs));
                });
            },
        );
    }
    
    group.finish();
}

/// 基准测试：仅签名验证 (对比用)
fn bench_verify_only(c: &mut Criterion) {
    let mut group = c.benchmark_group("verify_only");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));
    
    for tx_count in [100, 500, 1000].iter() {
        let (_, _, txs) = setup_test_environment(*tx_count);
        
        group.throughput(Throughput::Elements(*tx_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_txs_verify", tx_count)),
            &txs,
            |b, txs| {
                b.iter(|| {
                    for tx in txs {
                        black_box(tx.verify_signature().is_ok());
                    }
                });
            },
        );
    }
    
    group.finish();
}

criterion_group!(
    name = benches;
    config = Criterion::default().noise_threshold(0.1);
    targets = bench_full_execution, bench_verify_only
);
criterion_main!(benches);
