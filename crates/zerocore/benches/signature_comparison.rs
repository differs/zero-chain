//! 签名算法性能对比基准测试
//! 
//! 对比 secp256k1 (ECDSA) vs ed25519

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use zerocore::crypto::PrivateKey;
use zerocore::account::U256;
use zerocore::transaction::UnsignedTransaction;
use ed25519_dalek::{SigningKey, VerifyingKey, Signer, Verifier};
use rand::rngs::OsRng;
use std::time::Duration;

/// 创建 secp256k1 交易并签名
fn create_secp256k1_tx(nonce: u64) -> (UnsignedTransaction, PrivateKey) {
    let private_key = PrivateKey::random();
    let tx = UnsignedTransaction::new_legacy(
        nonce,
        U256::from(1_000_000_000),
        U256::from(21000),
        None,
        U256::from(1000),
        vec![],
        10086,
    );
    (tx, private_key)
}

/// 创建 ed25519 交易并签名
fn create_ed25519_tx(nonce: u64) -> (Vec<u8>, SigningKey, VerifyingKey) {
    let csprng = OsRng {};
    let signing_key = SigningKey::generate(&csprng);
    let verifying_key = VerifyingKey::from(&signing_key);
    
    // 模拟交易数据
    let mut tx_data = Vec::new();
    tx_data.extend_from_slice(&nonce.to_be_bytes());
    tx_data.extend_from_slice(&[1u8; 32]);
    tx_data.extend_from_slice(&[2u8; 32]);
    
    (tx_data, signing_key, verifying_key)
}

/// secp256k1 签名性能测试
fn bench_secp256k1_sign(c: &mut Criterion) {
    let mut group = c.benchmark_group("secp256k1_sign");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));
    
    for count in [100, 500, 1000].iter() {
        let txs: Vec<_> = (0..*count).map(|i| create_secp256k1_tx(i)).collect();
        
        group.throughput(Throughput::Elements(*count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_txs", count)),
            &txs,
            |b, txs| {
                b.iter(|| {
                    for (tx, private_key) in txs {
                        black_box(tx.clone().sign(private_key));
                    }
                });
            },
        );
    }
    
    group.finish();
}

/// secp256k1 验证性能测试
fn bench_secp256k1_verify(c: &mut Criterion) {
    let mut group = c.benchmark_group("secp256k1_verify");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));
    
    let signed_txs: Vec<_> = (0..1000)
        .map(|i| {
            let (tx, private_key) = create_secp256k1_tx(i);
            tx.sign(&private_key)
        })
        .collect();
    
    for count in [100, 500, 1000].iter() {
        group.throughput(Throughput::Elements(*count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_txs", count)),
            &signed_txs[..*count],
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

/// ed25519 签名性能测试
fn bench_ed25519_sign(c: &mut Criterion) {
    let mut group = c.benchmark_group("ed25519_sign");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));
    
    for count in [100, 500, 1000].iter() {
        let txs: Vec<_> = (0..*count).map(|i| create_ed25519_tx(i)).collect();
        
        group.throughput(Throughput::Elements(*count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_txs", count)),
            &txs,
            |b, txs| {
                b.iter(|| {
                    for (tx_data, signing_key, _) in txs {
                        black_box(signing_key.sign(tx_data));
                    }
                });
            },
        );
    }
    
    group.finish();
}

/// ed25519 验证性能测试
fn bench_ed25519_verify(c: &mut Criterion) {
    let mut group = c.benchmark_group("ed25519_verify");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));
    
    let signed_txs: Vec<_> = (0..1000)
        .map(|i| {
            let (tx_data, signing_key, verifying_key) = create_ed25519_tx(i);
            let signature = signing_key.sign(&tx_data);
            (tx_data, signature, verifying_key)
        })
        .collect();
    
    for count in [100, 500, 1000].iter() {
        group.throughput(Throughput::Elements(*count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_txs", count)),
            &signed_txs[..*count],
            |b, txs| {
                b.iter(|| {
                    for (tx_data, signature, verifying_key) in txs {
                        black_box(verifying_key.verify(tx_data, signature).is_ok());
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
    targets = bench_secp256k1_sign, bench_secp256k1_verify, 
              bench_ed25519_sign, bench_ed25519_verify
);
criterion_main!(benches);
