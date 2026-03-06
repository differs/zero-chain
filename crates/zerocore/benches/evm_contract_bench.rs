//! EVM 合约执行基准测试

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use zerocore::crypto::{Address, PrivateKey, Hash};
use zerocore::account::{U256, Account};
use zerocore::evm::{EvmEngine, EvmConfig};
use zerocore::state::StateDb;
use zerocore::transaction::UnsignedTransaction;
use std::time::Duration;

/// 创建测试环境
fn setup_test_account(state_db: &mut StateDb) -> (PrivateKey, Address) {
    let private_key = PrivateKey::random();
    let address = private_key.public_key().address();
    
    let account = Account {
        address,
        balance: U256::from(1_000_000_000_000_000u64),
        nonce: 0,
        ..Default::default()
    };
    state_db.insert_account(address, account).unwrap();
    
    (private_key, address)
}

/// 基准测试：简单 EVM 执行
fn bench_evm_simple_execution(c: &mut Criterion) {
    let mut group = c.benchmark_group("evm_simple");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));
    
    group.bench_function("simple_stop", |b| {
        b.iter(|| {
            let mut state_db = StateDb::new(Hash::zero());
            let (private_key, address) = setup_test_account(&mut state_db);
            
            // STOP 操作码
            let bytecode = vec![0x00];
            
            let tx = UnsignedTransaction::new_legacy(
                0,
                U256::from(1_000_000_000),
                U256::from(100_000),
                None,
                U256::zero(),
                bytecode,
                10086,
            ).sign(&private_key);
            
            let mut evm = EvmEngine::new(EvmConfig {
                chain_id: 10086,
                gas_limit: 100_000,
                base_fee: U256::from(1_000_000_000),
            });
            
            let result = evm.execute(&tx, &mut state_db);
            black_box(result.is_ok());
        });
    });
    
    group.finish();
}

/// 基准测试：EVM 算术运算
fn bench_evm_arithmetic(c: &mut Criterion) {
    let mut group = c.benchmark_group("evm_arithmetic");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));
    
    group.bench_function("add_numbers", |b| {
        b.iter(|| {
            let mut state_db = StateDb::new(Hash::zero());
            let (private_key, address) = setup_test_account(&mut state_db);
            
            // PUSH1 1, PUSH1 2, ADD, STOP
            let bytecode = vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00];
            
            let tx = UnsignedTransaction::new_legacy(
                0,
                U256::from(1_000_000_000),
                U256::from(100_000),
                None,
                U256::zero(),
                bytecode,
                10086,
            ).sign(&private_key);
            
            let mut evm = EvmEngine::new(EvmConfig {
                chain_id: 10086,
                gas_limit: 100_000,
                base_fee: U256::from(1_000_000_000),
            });
            
            let result = evm.execute(&tx, &mut state_db);
            black_box(result.is_ok());
        });
    });
    
    group.finish();
}

criterion_group!(
    name = benches;
    config = Criterion::default().noise_threshold(0.1);
    targets = bench_evm_simple_execution, bench_evm_arithmetic
);
criterion_main!(benches);
