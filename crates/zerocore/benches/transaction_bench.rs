//! 交易处理性能基准测试

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::sync::Arc;
use std::time::Duration;
use zerocore::account::InMemoryAccountManager;
use zerocore::account::U256;
use zerocore::crypto::PrivateKey;
use zerocore::transaction::{
    SignedTransaction, TransactionPool, TxPoolConfig, UnsignedTransaction,
};

/// 创建已签名的测试交易
fn create_signed_transactions(count: usize) -> Vec<SignedTransaction> {
    let mut txs = Vec::new();
    for i in 0..count {
        let private_key = PrivateKey::random();
        let tx = UnsignedTransaction::new_legacy(
            i as u64,
            U256::from(1_000_000_000),
            U256::from(21000),
            None,
            U256::from(1000),
            vec![],
            10086,
        );

        // 签名交易
        if let Ok(signed_tx) = tx.sign(&private_key) {
            txs.push(signed_tx);
        }
    }
    txs
}

/// 测试交易池添加性能
fn bench_transaction_pool(c: &mut Criterion) {
    let mut group = c.benchmark_group("transaction_pool");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));

    let account_manager = Arc::new(InMemoryAccountManager::new());

    for tx_count in [100, 500, 1000, 2000].iter() {
        let txs = create_signed_transactions(*tx_count);

        group.throughput(Throughput::Elements(*tx_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_txs", tx_count)),
            &txs,
            |b, txs| {
                b.iter(|| {
                    let pool =
                        TransactionPool::new(TxPoolConfig::default(), account_manager.clone());

                    // 添加交易到池
                    for tx in txs {
                        black_box(pool.add_transaction(tx.clone()));
                    }
                });
            },
        );
    }

    group.finish();
}

/// 测试交易验证性能
fn bench_transaction_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("transaction_validation");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));

    for tx_count in [100, 500, 1000].iter() {
        let txs = create_signed_transactions(*tx_count);

        group.throughput(Throughput::Elements(*tx_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_validations", tx_count)),
            &txs,
            |b, txs| {
                b.iter(|| {
                    for tx in txs {
                        // 验证签名
                        black_box(tx.verify_signature());
                    }
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    name = benches;
    config = Criterion::default().noise_threshold(0.05);
    targets = bench_transaction_pool, bench_transaction_validation
);
criterion_main!(benches);
