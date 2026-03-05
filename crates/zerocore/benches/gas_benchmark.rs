//! Gas 处理性能基准测试

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use zerocore::account::U256;
use zerocore::transaction::UnsignedTransaction;

/// 创建测试交易
fn create_test_transactions(count: usize) -> Vec<UnsignedTransaction> {
    let mut txs = Vec::new();
    for i in 0..count {
        let tx = UnsignedTransaction::new_legacy(
            i as u64,
            U256::from(1_000_000_000),
            U256::from(21000),
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

/// 测试不同 Gas 上限的处理时间
fn bench_gas_processing(c: &mut Criterion) {
    let mut group = c.benchmark_group("gas_processing");
    group.sample_size(10);  // 减少样本数加快测试
    group.measurement_time(std::time::Duration::from_secs(30));
    
    for gas_limit in [30_000_000u64, 50_000_000, 60_000_000, 100_000_000].iter() {
        let tx_count = gas_limit / 21000;
        
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}M", gas_limit / 1_000_000)),
            gas_limit,
            |b, &gas_limit| {
                b.iter(|| {
                    let txs = create_test_transactions((gas_limit / 21000) as usize);
                    for tx in &txs {
                        black_box(tx.nonce);
                    }
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_gas_processing);
criterion_main!(benches);
