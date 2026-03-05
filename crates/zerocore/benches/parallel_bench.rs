//! 并行执行性能基准测试

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::time::Duration;
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

/// 模拟交易验证
fn validate_transaction(tx: &UnsignedTransaction) {
    black_box(tx.nonce);
    black_box(tx.gas_limit);
    black_box(tx.value);
}

/// 串行执行基准测试
fn bench_serial_execution(c: &mut Criterion) {
    let mut group = c.benchmark_group("serial_execution");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));

    for tx_count in [1000, 2000, 3000].iter() {
        let txs = create_test_transactions(*tx_count);

        group.throughput(Throughput::Elements(*tx_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_txs_serial", tx_count)),
            &txs,
            |b, txs| {
                b.iter(|| {
                    for tx in txs {
                        validate_transaction(tx);
                    }
                });
            },
        );
    }

    group.finish();
}

/// 并行执行基准测试 (模拟多核)
fn bench_parallel_execution(c: &mut Criterion) {
    let mut group = c.benchmark_group("parallel_execution");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));

    for tx_count in [1000, 2000, 3000].iter() {
        let txs = create_test_transactions(*tx_count);

        group.throughput(Throughput::Elements(*tx_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_txs_parallel", tx_count)),
            &txs,
            |b, txs| {
                b.iter(|| {
                    // 模拟 8 核并行
                    let chunk_size = (txs.len() / 8).max(1);
                    let chunks: Vec<_> = txs.chunks(chunk_size).collect();

                    chunks.par_iter().for_each(|chunk| {
                        for tx in chunk {
                            validate_transaction(tx);
                        }
                    });
                });
            },
        );
    }

    group.finish();
}

/// 并行加速比测试
fn bench_speedup(c: &mut Criterion) {
    let mut group = c.benchmark_group("speedup_comparison");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));

    let tx_count = 2000;
    let txs = create_test_transactions(tx_count);

    // 串行
    group.bench_function("serial_2000txs", |b| {
        b.iter(|| {
            for tx in &txs {
                validate_transaction(tx);
            }
        });
    });

    // 2 核并行
    group.bench_function("parallel_2core_2000txs", |b| {
        b.iter(|| {
            let chunks: Vec<_> = txs.chunks(tx_count / 2).collect();
            chunks.par_iter().for_each(|chunk| {
                for tx in chunk {
                    validate_transaction(tx);
                }
            });
        });
    });

    // 4 核并行
    group.bench_function("parallel_4core_2000txs", |b| {
        b.iter(|| {
            let chunks: Vec<_> = txs.chunks(tx_count / 4).collect();
            chunks.par_iter().for_each(|chunk| {
                for tx in chunk {
                    validate_transaction(tx);
                }
            });
        });
    });

    // 8 核并行
    group.bench_function("parallel_8core_2000txs", |b| {
        b.iter(|| {
            let chunks: Vec<_> = txs.chunks(tx_count / 8).collect();
            chunks.par_iter().for_each(|chunk| {
                for tx in chunk {
                    validate_transaction(tx);
                }
            });
        });
    });

    group.finish();
}

criterion_group!(
    name = benches;
    config = Criterion::default().noise_threshold(0.05);
    targets = bench_serial_execution, bench_parallel_execution, bench_speedup
);
criterion_main!(benches);
