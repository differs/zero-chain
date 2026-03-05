//! EVM Gas calculation and management

use crate::evm::EvmError;

/// Zero gas cost
pub const GAS_ZERO: u64 = 0;
/// Base gas cost for simple operations
pub const GAS_BASE: u64 = 2;
/// Fastest gas cost (base)
pub const GAS_FASTEST: u64 = 0;
/// Fast gas cost
pub const GAS_FAST: u64 = 5;
/// Mid gas cost
pub const GAS_MID: u64 = 8;
/// Slow gas cost
pub const GAS_SLOW: u64 = 10;
/// Extra slow gas cost
pub const GAS_EXTRA_SLOW: u64 = 20;

/// Gas costs for specific operations
pub const GAS_STOP: u64 = 0;
pub const GAS_ADD: u64 = 3;
pub const GAS_MUL: u64 = 5;
pub const GAS_SUB: u64 = 3;
pub const GAS_DIV: u64 = 5;
pub const GAS_SDIV: u64 = 5;
pub const GAS_MOD: u64 = 5;
pub const GAS_SMOD: u64 = 5;
pub const GAS_ADDMOD: u64 = 8;
pub const GAS_MULMOD: u64 = 8;
pub const GAS_EXP: u64 = 10;
pub const GAS_EXP_PER_BYTE: u64 = 50;
pub const GAS_SIGNEXTEND: u64 = 5;

pub const GAS_LT: u64 = 3;
pub const GAS_GT: u64 = 3;
pub const GAS_SLT: u64 = 3;
pub const GAS_SGT: u64 = 3;
pub const GAS_EQ: u64 = 3;
pub const GAS_ISZERO: u64 = 3;
pub const GAS_AND: u64 = 3;
pub const GAS_OR: u64 = 3;
pub const GAS_XOR: u64 = 3;
pub const GAS_NOT: u64 = 3;
pub const GAS_BYTE: u64 = 3;
pub const GAS_SHL: u64 = 3;
pub const GAS_SHR: u64 = 3;
pub const GAS_SAR: u64 = 3;

pub const GAS_SHA3: u64 = 30;
pub const GAS_SHA3_PER_WORD: u64 = 6;

pub const GAS_ADDRESS: u64 = 2;
pub const GAS_BALANCE: u64 = 2600;
pub const GAS_BALANCE_WARM: u64 = 100;
pub const GAS_ORIGIN: u64 = 2;
pub const GAS_CALLER: u64 = 2;
pub const GAS_CALLVALUE: u64 = 2;
pub const GAS_CALLDATALOAD: u64 = 3;
pub const GAS_CALLDATASIZE: u64 = 2;
pub const GAS_CALLDATACOPY: u64 = 3;
pub const GAS_CALLDATACOPY_PER_WORD: u64 = 3;
pub const GAS_CODESIZE: u64 = 2;
pub const GAS_CODECOPY: u64 = 3;
pub const GAS_CODECOPY_PER_WORD: u64 = 3;
pub const GAS_GASPRICE: u64 = 2;
pub const GAS_EXTCODESIZE: u64 = 2600;
pub const GAS_EXTCODESIZE_WARM: u64 = 100;
pub const GAS_EXTCODECOPY: u64 = 2600;
pub const GAS_EXTCODECOPY_WARM: u64 = 100;
pub const GAS_EXTCODECOPY_PER_WORD: u64 = 3;
pub const GAS_RETURNDATASIZE: u64 = 2;
pub const GAS_RETURNDATACOPY: u64 = 3;
pub const GAS_RETURNDATACOPY_PER_WORD: u64 = 3;
pub const GAS_EXTCODEHASH: u64 = 2600;
pub const GAS_EXTCODEHASH_WARM: u64 = 100;

pub const GAS_BLOCKHASH: u64 = 20;
pub const GAS_COINBASE: u64 = 2;
pub const GAS_TIMESTAMP: u64 = 2;
pub const GAS_NUMBER: u64 = 2;
pub const GAS_PREVRANDAO: u64 = 2;
pub const GAS_GASLIMIT: u64 = 2;
pub const GAS_CHAINID: u64 = 2;
pub const GAS_SELFBALANCE: u64 = 5;
pub const GAS_BASEFEE: u64 = 2;

pub const GAS_POP: u64 = 2;
pub const GAS_MLOAD: u64 = 3;
pub const GAS_MSTORE: u64 = 3;
pub const GAS_MSTORE8: u64 = 3;
pub const GAS_SLOAD: u64 = 2100;
pub const GAS_SLOAD_WARM: u64 = 100;
pub const GAS_SSTORE: u64 = 20000;
pub const GAS_SSTORE_RESET: u64 = 5000;
pub const GAS_SSTORE_CLEARS_SCHEDULE: u64 = 4800;
pub const GAS_JUMP: u64 = 8;
pub const GAS_JUMPI: u64 = 10;
pub const GAS_JUMPDEST: u64 = 1;
pub const GAS_PC: u64 = 2;
pub const GAS_MSIZE: u64 = 2;
pub const GAS_GAS: u64 = 2;

pub const GAS_CREATE: u64 = 32000;
pub const GAS_CREATE2: u64 = 32000;
pub const GAS_CALL: u64 = 21000;
pub const GAS_CALL_VALUE: u64 = 9000;
pub const GAS_CALL_STIPEND: u64 = 2300;
pub const GAS_CALLCODE: u64 = 21000;
pub const GAS_DELEGATECALL: u64 = 21000;
pub const GAS_STATICCALL: u64 = 21000;
pub const GAS_RETURN: u64 = 0;
pub const GAS_REVERT: u64 = 0;
pub const GAS_INVALID: u64 = 0;
pub const GAS_SELFDESTRUCT: u64 = 5000;
pub const GAS_SELFDESTRUCT_NEW: u64 = 25000;

pub const GAS_LOG: u64 = 375;
pub const GAS_LOG_PER_TOPIC: u64 = 375;
pub const GAS_LOG_PER_BYTE: u64 = 8;

pub const GAS_COPY_PER_WORD: u64 = 3;

pub const GAS_TX_CREATE: u64 = 53000;
pub const GAS_TX: u64 = 21000;
pub const GAS_TX_DATA_ZERO: u64 = 4;
pub const GAS_TX_DATA_NON_ZERO: u64 = 16;
pub const GAS_TX_DATA_NON_ZERO_ISTANBUL: u64 = 16;

pub const MEMORY_EXPANSION_QUOTIENT: u64 = 512;

/// Consume gas from the gas left
pub fn consume_gas(gas_left: &mut u64, amount: u64) -> Result<(), EvmError> {
    if amount > *gas_left {
        return Err(EvmError::OutOfGas);
    }
    *gas_left -= amount;
    Ok(())
}

/// Calculate memory cost for expansion
pub fn memory_cost(offset: usize, size: usize) -> Result<u64, EvmError> {
    if size == 0 {
        return Ok(0);
    }

    // Calculate the new memory size needed
    let end = offset
        .checked_add(size)
        .ok_or(EvmError::InvalidMemoryAccess)?;

    // Round up to nearest 32-byte word
    let words = (end + 31) / 32;

    // Calculate memory cost using the formula:
    // cost = (words^2 / 512) + (3 * words)
    let word_count = words as u64;
    let memory_cost = (word_count * word_count) / MEMORY_EXPANSION_QUOTIENT + (3 * word_count);

    Ok(memory_cost)
}

/// Calculate gas cost for memory expansion
pub fn memory_expansion_cost(
    current_memory_size: usize,
    offset: usize,
    size: usize,
) -> Result<u64, EvmError> {
    if size == 0 {
        return Ok(0);
    }

    let end = offset
        .checked_add(size)
        .ok_or(EvmError::InvalidMemoryAccess)?;

    // Calculate new memory size in words (32 bytes)
    let new_memory_size = ((end + 31) / 32) * 32;

    if new_memory_size <= current_memory_size {
        return Ok(0);
    }

    // Calculate cost for current and new memory size
    let current_words = (current_memory_size / 32) as u64;
    let new_words = (new_memory_size / 32) as u64;

    let current_cost =
        (current_words * current_words) / MEMORY_EXPANSION_QUOTIENT + (3 * current_words);
    let new_cost = (new_words * new_words) / MEMORY_EXPANSION_QUOTIENT + (3 * new_words);

    Ok(new_cost - current_cost)
}

/// Calculate gas cost for EXP operation
pub fn exp_gas_cost(exponent: u64) -> u64 {
    if exponent == 0 {
        return GAS_EXP;
    }

    let bytes = ((64 - exponent.leading_zeros()) + 7) / 8;
    GAS_EXP + (GAS_EXP_PER_BYTE * bytes as u64)
}

/// Calculate gas cost for SHA3 operation
pub fn sha3_gas_cost(size: usize) -> u64 {
    let words = ((size + 31) / 32) as u64;
    GAS_SHA3 + (GAS_SHA3_PER_WORD * words)
}

/// Calculate gas cost for LOG operation
pub fn log_gas_cost(size: usize, num_topics: usize) -> u64 {
    let words = ((size + 31) / 32) as u64;
    GAS_LOG + (GAS_LOG_PER_TOPIC * num_topics as u64) + (GAS_LOG_PER_BYTE * words)
}

/// Calculate gas cost for copy operations (CALLDATACOPY, CODECOPY, etc.)
pub fn copy_gas_cost(size: usize) -> u64 {
    let words = ((size + 31) / 32) as u64;
    GAS_COPY_PER_WORD * words
}

/// Calculate initial memory expansion cost
pub fn initial_memory_cost(size: usize) -> u64 {
    if size == 0 {
        return 0;
    }
    let words = ((size + 31) / 32) as u64;
    (words * words) / MEMORY_EXPANSION_QUOTIENT + (3 * words)
}
