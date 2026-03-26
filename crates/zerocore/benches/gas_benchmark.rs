//! Gas 处理性能基准测试
//!
//! 测试不同 Gas 上限下的交易处理时间

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::time::Duration;
use zerocore::account::U256;
use zerocore::transaction::UnsignedTransaction;

/// 创建测试交易
fn create_test_transactions(count: usize) -> Vec<UnsignedTransaction> {
    let mut txs = Vec::new();
    for i in 0..count {
        let tx = UnsignedTransaction::new_transfer(
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

/// 模拟交易验证开销
fn validate_transactions(txs: &[UnsignedTransaction]) {
    for tx in txs {
        black_box(tx.nonce);
        black_box(tx.gas_limit);
        black_box(tx.value);
    }
}

/// 测试不同 Gas 上限的处理时间
fn bench_gas_processing(c: &mut Criterion) {
    let mut group = c.benchmark_group("gas_processing");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));
    group.warm_up_time(Duration::from_secs(3));

    for gas_limit in [30_000_000u64, 50_000_000, 60_000_000, 100_000_000].iter() {
        let tx_count = gas_limit / 21000;

        group.throughput(Throughput::Elements(tx_count));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}M_Gas", gas_limit / 1_000_000)),
            gas_limit,
            |b, &_gas_limit| {
                b.iter(|| {
                    let txs = create_test_transactions(tx_count as usize);
                    validate_transactions(&txs);
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    name = benches;
    config = Criterion::default().noise_threshold(0.05);
    targets = bench_gas_processing
);
criterion_main!(benches);
