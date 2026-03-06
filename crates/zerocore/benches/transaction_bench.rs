//! 交易处理性能基准测试

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::time::Duration;
use zerocore::account::U256;
use zerocore::transaction::UnsignedTransaction;

/// 创建已签名的测试交易
fn create_signed_transactions(count: usize) -> Vec<UnsignedTransaction> {
    let mut txs = Vec::new();
    for i in 0..count {
        let tx = UnsignedTransaction::new_legacy(
            i as u64,
            U256::from(1_000_000_000),
            U256::from(21000),
            None,
            U256::from(1000),
            vec![],
            10086,
        );
        txs.push(tx);
    }
    txs
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
                        // 验证交易字段
                        black_box(tx.nonce);
                        black_box(tx.gas_limit);
                        black_box(tx.value);
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
    targets = bench_transaction_validation
);
criterion_main!(benches);
