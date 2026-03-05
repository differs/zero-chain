//! 5 秒极限压力测试
//! 
//! 测试 5 秒内最多能验证多少交易

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use zerocore::crypto::PrivateKey;
use zerocore::account::U256;
use zerocore::transaction::UnsignedTransaction;
use std::time::Duration;

/// 创建已签名交易
fn create_signed_transactions(count: usize) -> Vec<UnsignedTransaction> {
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

/// 验证交易
fn validate_transactions(txs: &[UnsignedTransaction]) -> usize {
    let mut valid_count = 0;
    for tx in txs {
        if tx.verify_signature().is_ok() {
            valid_count += 1;
        }
    }
    valid_count
}

/// 5 秒极限测试 - 固定时间窗口
fn bench_5sec_limit(c: &mut Criterion) {
    let mut group = c.benchmark_group("5sec_limit");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));
    group.warm_up_time(Duration::from_secs(1));
    group.noise_threshold(0.1);
    
    // 测试不同规模
    for tx_count in [10000, 50000, 100000, 200000, 500000].iter() {
        println!("\n📊 准备 {} 笔交易...", tx_count);
        let txs = create_signed_transactions(*tx_count);
        println!("✅ 交易准备完成");
        
        group.throughput(Throughput::Elements(*tx_count as u64));
        group.bench_with_input(
            format!("{}_transactions", tx_count),
            &txs,
            |b, txs| {
                b.iter_custom(|iters| {
                    let start = std::time::Instant::now();
                    for _ in 0..iters {
                        black_box(validate_transactions(txs));
                    }
                    start.elapsed()
                });
            },
        );
    }
    
    group.finish();
}

/// 固定 5 秒窗口测试
fn bench_fixed_5sec_window(c: &mut Criterion) {
    let mut group = c.benchmark_group("fixed_5sec");
    group.sample_size(5);
    group.measurement_time(Duration::from_secs(5));
    
    // 预创建大量交易
    let tx_count = 100000;
    let txs = create_signed_transactions(tx_count);
    
    group.throughput(Throughput::Elements(tx_count as u64));
    group.bench_function("max_transactions_in_5sec", |b| {
        b.iter_custom(|iters| {
            let start = std::time::Instant::now();
            let mut total_validated = 0;
            
            for _ in 0..iters {
                total_validated += validate_transactions(&txs);
            }
            
            let elapsed = start.elapsed();
            println!("⏱️  {} 次迭代验证了 {} 笔交易，耗时 {:?}", 
                     iters, total_validated, elapsed);
            elapsed
        });
    });
    
    group.finish();
}

/// 推算 5 秒极限
fn bench_extrapolate_5sec(c: &mut Criterion) {
    let mut group = c.benchmark_group("extrapolate_5sec");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));
    
    // 小规模测试，然后推算
    for tx_count in [1000, 5000, 10000].iter() {
        let txs = create_signed_transactions(*tx_count);
        
        group.throughput(Throughput::Elements(*tx_count as u64));
        group.bench_with_input(
            format!("{}_base", tx_count),
            &txs,
            |b, txs| {
                b.iter(|| {
                    black_box(validate_transactions(txs));
                });
            },
        );
    }
    
    group.finish();
}

criterion_group!(
    name = benches;
    config = Criterion::default()
        .noise_threshold(0.1)
        .sample_size(10);
    targets = bench_5sec_limit, bench_fixed_5sec_window, bench_extrapolate_5sec
);
criterion_main!(benches);
