//! 完整交易执行基准测试
//!
//! 包含：签名验证 + 交易池添加 + 状态更新

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::sync::Arc;
use std::time::Duration;
use zerocore::account::{InMemoryAccountManager, U256};
use zerocore::crypto::{Address, PrivateKey};
use zerocore::transaction::pool::TxPoolConfig;
use zerocore::transaction::{SignedTransaction, TransactionPool, UnsignedTransaction};

/// 创建测试交易
fn create_test_transactions(tx_count: usize) -> Vec<SignedTransaction> {
    let mut txs = Vec::new();
    for i in 0..tx_count {
        let private_key = PrivateKey::random();
        let tx = UnsignedTransaction::new_transfer(
            i as u64,
            U256::from(1_000_000_000),
            U256::from(21000),
            Some(Address::from_bytes([i as u8; 20])),
            U256::from(1000),
            vec![],
            10086,
        );

        let signed_tx = tx.sign(&private_key);
        txs.push(signed_tx);
    }
    txs
}

/// 完整交易执行流程
fn execute_transactions_full(pool: &TransactionPool, txs: &[SignedTransaction]) -> usize {
    let mut executed = 0;

    for tx in txs {
        // 1. 验证签名
        if tx.verify_signature().is_err() {
            continue;
        }

        // 2. 添加到交易池 (包含 nonce 检查、余额检查等)
        if pool.add_transaction(tx.clone()).is_err() {
            continue;
        }

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
        let txs = create_test_transactions(*tx_count);

        let account_manager = Arc::new(InMemoryAccountManager::new());
        let pool = TransactionPool::new(TxPoolConfig::default(), account_manager);

        group.throughput(Throughput::Elements(*tx_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_txs_full", tx_count)),
            &txs,
            |b, txs| {
                b.iter(|| {
                    black_box(execute_transactions_full(&pool, txs));
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
        let txs = create_test_transactions(*tx_count);

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
