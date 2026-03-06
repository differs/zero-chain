//! 真实交易验证基准测试
//!
//! 包含完整的签名验证、nonce 检查、余额验证

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::sync::Arc;
use std::time::Duration;
use zerocore::account::{InMemoryAccountManager, U256};
use zerocore::crypto::PrivateKey;
use zerocore::transaction::pool::TxPoolConfig;
use zerocore::transaction::{SignedTransaction, TransactionPool, UnsignedTransaction};

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

        let signed_tx = tx.sign(&private_key);
        txs.push(signed_tx);
    }
    txs
}

/// 真实交易验证（包含签名验证）
fn validate_transaction_real(tx: &SignedTransaction) -> Result<(), &'static str> {
    // 1. 验证签名（这是最耗时的部分）
    tx.verify_signature().map_err(|_| "Invalid signature")?;

    // 2. 验证 nonce
    if tx.tx.nonce > 1000000 {
        return Err("Invalid nonce");
    }

    // 3. 验证 Gas
    if tx.tx.gas_limit < U256::from(21000) {
        return Err("Gas limit too low");
    }

    Ok(())
}

/// 基准测试：真实交易验证（含签名）
fn bench_real_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("real_validation");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));
    group.warm_up_time(Duration::from_secs(5));

    for tx_count in [100, 500, 1000].iter() {
        let txs = create_signed_transactions(*tx_count);

        group.throughput(Throughput::Elements(*tx_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_txs_real", tx_count)),
            &txs,
            |b, txs| {
                b.iter(|| {
                    for tx in txs {
                        black_box(validate_transaction_real(tx).is_ok());
                    }
                });
            },
        );
    }

    group.finish();
}

/// 基准测试：交易池完整流程
fn bench_transaction_pool_full(c: &mut Criterion) {
    let mut group = c.benchmark_group("transaction_pool_full");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));

    let account_manager = Arc::new(InMemoryAccountManager::new());

    for tx_count in [100, 500, 1000].iter() {
        let txs = create_signed_transactions(*tx_count);

        group.throughput(Throughput::Elements(*tx_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_txs_pool", tx_count)),
            &txs,
            |b, txs| {
                b.iter(|| {
                    let pool =
                        TransactionPool::new(TxPoolConfig::default(), account_manager.clone());

                    for tx in txs {
                        let _ = black_box(pool.add_transaction(tx.clone()));
                    }
                });
            },
        );
    }

    group.finish();
}

/// 基准测试：极限压力测试
fn bench_stress_test(c: &mut Criterion) {
    let mut group = c.benchmark_group("stress_test");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(120));

    for tx_count in [2000, 5000, 10000].iter() {
        let txs = create_signed_transactions(*tx_count);

        group.throughput(Throughput::Elements(*tx_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_txs_stress", tx_count)),
            &txs,
            |b, txs| {
                b.iter(|| {
                    for tx in txs {
                        black_box(validate_transaction_real(tx).is_ok());
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
    targets = bench_real_validation, bench_transaction_pool_full, bench_stress_test
);
criterion_main!(benches);
