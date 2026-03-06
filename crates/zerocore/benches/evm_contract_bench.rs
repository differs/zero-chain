//! EVM 合约部署和执行基准测试
//! 
//! 测试真实的智能合约部署和调用

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use zerocore::crypto::{Address, PrivateKey, Hash};
use zerocore::account::{U256, Account, InMemoryAccountManager};
use zerocore::evm::{EvmEngine, EvmConfig, StateDb};
use zerocore::transaction::{UnsignedTransaction, SignedTransaction};
use std::sync::Arc;
use std::time::Duration;

/// ERC20 合约字节码 (简化版)
const ERC20_BYTECODE: &[u8] = include_bytes!("../../test_contracts/erc20_bytecode.bin");

/// 创建测试环境
fn setup_evm_environment() -> (EvmEngine, StateDb, Address) {
    let evm = EvmEngine::new(EvmConfig::default());
    let state_db = StateDb::new(Hash::zero());
    
    // 创建测试账户
    let deployer = Address::from_bytes([1u8; 20]);
    let account = Account {
        address: deployer,
        balance: U256::from(1_000_000_000_000_000u64), // 足够 Gas 费
        nonce: 0,
        ..Default::default()
    };
    state_db.insert_account(deployer, account).unwrap();
    
    (evm, state_db, deployer)
}

/// 基准测试：合约部署
fn bench_contract_deployment(c: &mut Criterion) {
    let mut group = c.benchmark_group("evm_deployment");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));
    
    group.bench_function(BenchmarkId::from_parameter("erc20_deploy"), |b| {
        b.iter(|| {
            let (evm, state_db, deployer) = setup_evm_environment();
            
            // 部署合约
            let result = evm.deploy(
                &state_db,
                ERC20_BYTECODE.to_vec(),
                vec![], // 构造函数参数
                deployer,
                U256::from(1_000_000), // Gas limit
                U256::from(1_000_000_000), // Gas price
            );
            
            black_box(result.is_ok());
        });
    });
    
    group.finish();
}

/// 基准测试：合约调用 (转账)
fn bench_contract_call(c: &mut Criterion) {
    let mut group = c.benchmark_group("evm_call");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));
    
    // 先部署合约
    let (evm, state_db, deployer) = setup_evm_environment();
    let contract_result = evm.deploy(
        &state_db,
        ERC20_BYTECODE.to_vec(),
        vec![],
        deployer,
        U256::from(1_000_000),
        U256::from(1_000_000_000),
    );
    
    if let Ok(contract_address) = contract_result {
        // 准备转账调用数据
        let transfer_selector = [0xa9, 0x05, 0x9c, 0xbb]; // transfer(address,uint256)
        let mut call_data = Vec::new();
        call_data.extend_from_slice(&transfer_selector);
        call_data.extend_from_slice(&[2u8; 32]); // to address
        call_data.extend_from_slice(&U256::from(100).to_big_endian()); // amount
        
        group.bench_function(BenchmarkId::from_parameter("erc20_transfer"), |b| {
            b.iter(|| {
                let result = evm.call(
                    &state_db,
                    contract_address,
                    call_data.clone(),
                    deployer,
                    U256::zero(), // value
                    U256::from(100_000), // gas limit
                    U256::from(1_000_000_000), // gas price
                );
                
                black_box(result.is_ok());
            });
        });
    }
    
    group.finish();
}

/// 基准测试：简单计算合约
fn bench_simple_computation(c: &mut Criterion) {
    let mut group = c.benchmark_group("evm_computation");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));
    
    // 简单计算合约字节码 (加法)
    let simple_bytecode = vec![
        0x60, 0x01, // PUSH1 1
        0x60, 0x02, // PUSH1 2
        0x01,       // ADD
        0x00,       // STOP
    ];
    
    group.bench_function(BenchmarkId::from_parameter("simple_add"), |b| {
        b.iter(|| {
            let (evm, state_db, deployer) = setup_evm_environment();
            
            let result = evm.execute(
                &state_db,
                deployer,
                simple_bytecode.clone(),
                U256::from(100_000),
                U256::from(1_000_000_000),
            );
            
            black_box(result.is_ok());
        });
    });
    
    group.finish();
}

criterion_group!(
    name = benches;
    config = Criterion::default().noise_threshold(0.1);
    targets = bench_contract_deployment, bench_contract_call, bench_simple_computation
);
criterion_main!(benches);
