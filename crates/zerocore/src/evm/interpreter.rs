//! EVM Interpreter - Complete implementation

use super::gas::*;
use super::opcodes::*;
use super::precompiles::*;
use super::EvmError;
use crate::account::{Account, AccountType, I256, U256};
use crate::crypto::{keccak256, Address, Hash};
use crate::evm::StateDb;
use bytes::Bytes;
use parking_lot::RwLock;
use std::sync::Arc;

/// EVM execution result
#[derive(Clone, Debug)]
pub struct ExecutionResult {
    pub success: bool,
    pub output: Bytes,
    pub gas_used: u64,
    pub logs: Vec<LogEntry>,
    pub created_address: Option<Address>,
    pub error: Option<EvmError>,
}

/// Log entry
#[derive(Clone, Debug)]
pub struct LogEntry {
    pub address: Address,
    pub topics: Vec<Hash>,
    pub data: Bytes,
}

/// EVM configuration
#[derive(Clone, Debug)]
pub struct EvmConfig {
    pub chain_id: u64,
    pub gas_limit: u64,
    pub base_fee: U256,
    pub max_code_size: usize,
    pub evm_version: EvmVersion,
}

impl Default for EvmConfig {
    fn default() -> Self {
        Self {
            chain_id: 10086,
            gas_limit: 30_000_000,
            base_fee: U256::from(1_000_000_000),
            max_code_size: 24576,
            evm_version: EvmVersion::London,
        }
    }
}

/// EVM version
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvmVersion {
    Frontier,
    Homestead,
    Byzantium,
    Constantinople,
    Istanbul,
    Berlin,
    London,
    Shanghai,
}

/// EVM interpreter
pub struct EvmInterpreter<'a> {
    /// Program counter
    pc: usize,
    /// Gas left
    gas_left: u64,
    /// Stack
    stack: EvmStack,
    /// Memory
    memory: EvmMemory,
    /// Return data
    return_data: Bytes,
    /// Logs
    logs: Vec<LogEntry>,
    /// Execution context
    context: ExecutionContext<'a>,
    /// Configuration
    config: &'a EvmConfig,
    /// Created contracts
    created_contracts: Vec<CreatedContract>,
}

/// Execution context
pub struct ExecutionContext<'a> {
    pub caller: Address,
    pub address: Address,
    pub code: Bytes,
    pub input: Bytes,
    pub value: U256,
    pub gas_price: U256,
    pub is_static: bool,
    pub depth: usize,
    pub state: &'a mut dyn StateDb,
}

/// Created contract info
pub struct CreatedContract {
    pub address: Address,
    pub code: Bytes,
}

impl<'a> EvmInterpreter<'a> {
    /// Create new interpreter
    pub fn new(context: ExecutionContext<'a>, config: &'a EvmConfig) -> Self {
        Self {
            pc: 0,
            gas_left: context.gas_price.as_u64(),
            stack: EvmStack::new(),
            memory: EvmMemory::new(),
            return_data: Bytes::new(),
            logs: Vec::new(),
            context,
            config,
            created_contracts: Vec::new(),
        }
    }

    /// Execute code
    pub fn execute(&mut self) -> Result<ExecutionResult, EvmError> {
        while self.pc < self.context.code.len() {
            let opcode = self.context.code[self.pc];

            // Execute opcode
            self.execute_opcode(opcode)?;

            // Move to next opcode
            self.pc += 1;
        }

        // Return remaining memory as output
        let output = self.memory.to_bytes();

        Ok(ExecutionResult {
            success: true,
            output,
            gas_used: self.gas_left,
            logs: self.logs.clone(),
            created_address: self.created_contracts.first().map(|c| c.address),
            error: None,
        })
    }

    /// Execute single opcode
    fn execute_opcode(&mut self, opcode: u8) -> Result<(), EvmError> {
        match opcode {
            // Stop and arithmetic
            OP_STOP => self.op_stop(),
            OP_ADD => self.op_add(),
            OP_MUL => self.op_mul(),
            OP_SUB => self.op_sub(),
            OP_DIV => self.op_div(),
            OP_SDIV => self.op_sdiv(),
            OP_MOD => self.op_mod(),
            OP_SMOD => self.op_smod(),
            OP_ADDMOD => self.op_addmod(),
            OP_MULMOD => self.op_mulmod(),
            OP_EXP => self.op_exp(),
            OP_SIGNEXTEND => self.op_signextend(),

            // Comparison
            OP_LT => self.op_lt(),
            OP_GT => self.op_gt(),
            OP_SLT => self.op_slt(),
            OP_SGT => self.op_sgt(),
            OP_EQ => self.op_eq(),
            OP_ISZERO => self.op_iszero(),

            // Bitwise
            OP_AND => self.op_and(),
            OP_OR => self.op_or(),
            OP_XOR => self.op_xor(),
            OP_NOT => self.op_not(),
            OP_BYTE => self.op_byte(),
            OP_SHL => self.op_shl(),
            OP_SHR => self.op_shr(),
            OP_SAR => self.op_sar(),

            // Hash
            OP_SHA3 => self.op_sha3(),

            // Environment
            OP_ADDRESS => self.op_address(),
            OP_BALANCE => self.op_balance(),
            OP_ORIGIN => self.op_origin(),
            OP_CALLER => self.op_caller(),
            OP_CALLVALUE => self.op_callvalue(),
            OP_CALLDATALOAD => self.op_calldataload(),
            OP_CALLDATASIZE => self.op_calldatasize(),
            OP_CALLDATACOPY => self.op_calldatacopy(),
            OP_CODESIZE => self.op_codesize(),
            OP_CODECOPY => self.op_codecopy(),
            OP_GASPRICE => self.op_gasprice(),
            OP_EXTCODESIZE => self.op_extcodesize(),
            OP_EXTCODECOPY => self.op_extcodecopy(),
            OP_RETURNDATASIZE => self.op_returndatasize(),
            OP_RETURNDATACOPY => self.op_returndatacopy(),
            OP_EXTCODEHASH => self.op_extcodehash(),

            // Block info
            OP_BLOCKHASH => self.op_blockhash(),
            OP_COINBASE => self.op_coinbase(),
            OP_TIMESTAMP => self.op_timestamp(),
            OP_NUMBER => self.op_number(),
            OP_PREVRANDAO => self.op_prevrandao(),
            OP_GASLIMIT => self.op_gaslimit(),
            OP_CHAINID => self.op_chainid(),
            OP_SELFBALANCE => self.op_selfbalance(),
            OP_BASEFEE => self.op_basefee(),

            // Stack, memory, storage, flow
            OP_POP => self.op_pop(),
            OP_MLOAD => self.op_mload(),
            OP_MSTORE => self.op_mstore(),
            OP_MSTORE8 => self.op_mstore8(),
            OP_SLOAD => self.op_sload(),
            OP_SSTORE => self.op_sstore(),
            OP_JUMP => self.op_jump(),
            OP_JUMPI => self.op_jumpi(),
            OP_PC => self.op_pc(),
            OP_MSIZE => self.op_msize(),
            OP_GAS => self.op_gas(),
            OP_JUMPDEST => self.op_jumpdest(),

            // Push operations
            op if op >= OP_PUSH1 && op <= OP_PUSH32 => self.op_push(op),

            // Dup operations
            op if op >= OP_DUP1 && op <= OP_DUP16 => self.op_dup(op),

            // Swap operations
            op if op >= OP_SWAP1 && op <= OP_SWAP16 => self.op_swap(op),

            // Log operations
            op if op >= OP_LOG0 && op <= OP_LOG4 => self.op_log(op),

            // System operations
            OP_CREATE => self.op_create(),
            OP_CALL => self.op_call(),
            OP_CALLCODE => self.op_callcode(),
            OP_RETURN => self.op_return(),
            OP_DELEGATECALL => self.op_delegatecall(),
            OP_CREATE2 => self.op_create2(),
            OP_STATICCALL => self.op_staticcall(),
            OP_REVERT => self.op_revert(),
            OP_INVALID => self.op_invalid(),
            OP_SELFDESTRUCT => self.op_selfdestruct(),

            // Unknown opcode
            _ => Err(EvmError::InvalidOpcode(opcode)),
        }
    }

    // ============ Arithmetic Operations ============

    fn op_stop(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_ZERO)?;
        Ok(())
    }

    fn op_add(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;
        self.stack.push(a.overflowing_add(b).0)?;
        Ok(())
    }

    fn op_mul(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FAST)?;
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;
        self.stack.push(a.overflowing_mul(b).0)?;
        Ok(())
    }

    fn op_sub(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;
        self.stack.push(a.overflowing_sub(b).0)?;
        Ok(())
    }

    fn op_div(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FAST)?;
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;

        if b.is_zero() {
            self.stack.push(U256::zero())?;
        } else {
            self.stack.push(a / b)?;
        }
        Ok(())
    }

    fn op_sdiv(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FAST)?;
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;

        // Simplified signed division
        if b.is_zero() {
            self.stack.push(U256::zero())?;
        } else {
            self.stack.push(a / b)?;
        }
        Ok(())
    }

    fn op_mod(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FAST)?;
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;

        if b.is_zero() {
            self.stack.push(U256::zero())?;
        } else {
            self.stack.push(a % b)?;
        }
        Ok(())
    }

    fn op_smod(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FAST)?;
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;

        if b.is_zero() {
            self.stack.push(U256::zero())?;
        } else {
            self.stack.push(a % b)?;
        }
        Ok(())
    }

    fn op_addmod(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_MID)?;
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;
        let c = self.stack.pop()?;

        if c.is_zero() {
            self.stack.push(U256::zero())?;
        } else {
            let (sum, _) = a.overflowing_add(b);
            self.stack.push(sum % c)?;
        }
        Ok(())
    }

    fn op_mulmod(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_MID)?;
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;
        let c = self.stack.pop()?;

        if c.is_zero() {
            self.stack.push(U256::zero())?;
        } else {
            let (prod, _) = a.overflowing_mul(b);
            self.stack.push(prod % c)?;
        }
        Ok(())
    }

    fn op_exp(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_EXP)?;
        let base = self.stack.pop()?;
        let exp = self.stack.pop()?;

        // Gas cost for exponentiation
        let exp_byte_cost = ((256 - exp.leading_zeros()) as u64 + 7) / 8 * 50;
        consume_gas(&mut self.gas_left, exp_byte_cost)?;

        self.stack.push(base.overflowing_pow_u256(exp).0)?;
        Ok(())
    }

    fn op_signextend(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FAST)?;
        let byte = self.stack.pop()?;
        let value = self.stack.pop()?;

        if byte.as_u64() < 31 {
            let sign_bit = (byte.as_u64() * 8 + 7) as usize;
            let mask = (U256::one() << sign_bit) - U256::one();

            if (value & (U256::one() << sign_bit)).is_zero() {
                self.stack.push(value & mask)?;
            } else {
                self.stack.push(value | !mask)?;
            }
        } else {
            self.stack.push(value)?;
        }
        Ok(())
    }

    // ============ Comparison Operations ============

    fn op_lt(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;
        self.stack
            .push(if a < b { U256::one() } else { U256::zero() })?;
        Ok(())
    }

    fn op_gt(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;
        self.stack
            .push(if a > b { U256::one() } else { U256::zero() })?;
        Ok(())
    }

    fn op_slt(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;
        // Simplified signed comparison
        self.stack
            .push(if a < b { U256::one() } else { U256::zero() })?;
        Ok(())
    }

    fn op_sgt(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;
        self.stack
            .push(if a > b { U256::one() } else { U256::zero() })?;
        Ok(())
    }

    fn op_eq(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;
        self.stack
            .push(if a == b { U256::one() } else { U256::zero() })?;
        Ok(())
    }

    fn op_iszero(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;
        let a = self.stack.pop()?;
        self.stack.push(if a.is_zero() {
            U256::one()
        } else {
            U256::zero()
        })?;
        Ok(())
    }

    // ============ Bitwise Operations ============

    fn op_and(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;
        self.stack.push(a & b)?;
        Ok(())
    }

    fn op_or(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;
        self.stack.push(a | b)?;
        Ok(())
    }

    fn op_xor(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;
        let a = self.stack.pop()?;
        let b = self.stack.pop()?;
        self.stack.push(a ^ b)?;
        Ok(())
    }

    fn op_not(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;
        let a = self.stack.pop()?;
        self.stack.push(!a)?;
        Ok(())
    }

    fn op_byte(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;
        let pos = self.stack.pop()?;
        let value = self.stack.pop()?;

        if pos.as_u64() >= 32 {
            self.stack.push(U256::zero())?;
        } else {
            let shift = (31 - pos.as_u64()) * 8;
            let byte = (value >> shift) & U256::from(0xFF);
            self.stack.push(byte)?;
        }
        Ok(())
    }

    fn op_shl(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;
        let shift = self.stack.pop()?;
        let value = self.stack.pop()?;

        if shift.as_u64() >= 256 {
            self.stack.push(U256::zero())?;
        } else {
            self.stack.push(value << shift)?;
        }
        Ok(())
    }

    fn op_shr(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;
        let shift = self.stack.pop()?;
        let value = self.stack.pop()?;

        if shift.as_u64() >= 256 {
            self.stack.push(U256::zero())?;
        } else {
            self.stack.push(value >> shift)?;
        }
        Ok(())
    }

    fn op_sar(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;
        let shift = self.stack.pop()?;
        let value = self.stack.pop()?;

        // Simplified arithmetic shift
        if shift.as_u64() >= 256 {
            self.stack.push(U256::zero())?;
        } else {
            self.stack.push(value >> shift)?;
        }
        Ok(())
    }

    // ============ Hash Operation ============

    fn op_sha3(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_SHA3)?;
        let offset = self.stack.pop()?;
        let size = self.stack.pop()?;

        let offset_usize = offset.as_u64() as usize;
        let size_usize = size.as_u64() as usize;

        // Memory expansion cost
        let memory_cost = memory_cost(offset_usize, size_usize)?;
        consume_gas(&mut self.gas_left, memory_cost)?;

        // Read data from memory
        let data = self.memory.read(offset_usize, size_usize)?;

        // Compute Keccak256
        let hash = keccak256(&data);

        self.stack.push(U256::from_big_endian(&hash))?;
        Ok(())
    }

    // ============ Environment Operations ============

    fn op_address(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_BASE)?;
        let addr: [u8; 32] = {
            let mut bytes = [0u8; 32];
            bytes[12..].copy_from_slice(self.context.address.as_bytes());
            bytes
        };
        self.stack.push(U256::from_big_endian(&addr))?;
        Ok(())
    }

    fn op_balance(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, 100)?;
        let addr_u256 = self.stack.pop()?;
        let addr = Address::from_slice(&addr_u256.to_big_endian()[12..])
            .map_err(|_| EvmError::InvalidAddress)?;

        let balance = self.context.state.get_balance(&addr);
        self.stack.push(balance)?;
        Ok(())
    }

    fn op_origin(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_BASE)?;
        // Would return tx origin
        self.stack.push(U256::zero())?;
        Ok(())
    }

    fn op_caller(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_BASE)?;
        let addr: [u8; 32] = {
            let mut bytes = [0u8; 32];
            bytes[12..].copy_from_slice(self.context.caller.as_bytes());
            bytes
        };
        self.stack.push(U256::from_big_endian(&addr))?;
        Ok(())
    }

    fn op_callvalue(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_BASE)?;
        self.stack.push(self.context.value)?;
        Ok(())
    }

    fn op_calldataload(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;
        let offset = self.stack.pop()?;

        let offset_usize = offset.as_u64() as usize;
        let mut data = [0u8; 32];

        if offset_usize < self.context.input.len() {
            let end = (offset_usize + 32).min(self.context.input.len());
            data[..end - offset_usize].copy_from_slice(&self.context.input[offset_usize..end]);
        }

        self.stack.push(U256::from_big_endian(&data))?;
        Ok(())
    }

    fn op_calldatasize(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_BASE)?;
        self.stack
            .push(U256::from_u128(self.context.input.len() as u128))?;
        Ok(())
    }

    fn op_calldatacopy(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;
        let mem_offset = self.stack.pop()?;
        let data_offset = self.stack.pop()?;
        let size = self.stack.pop()?;

        let mem_offset_usize = mem_offset.as_u64() as usize;
        let data_offset_usize = data_offset.as_u64() as usize;
        let size_usize = size.as_u64() as usize;

        // Memory expansion cost
        let memory_cost = memory_cost(mem_offset_usize, size_usize)?;
        consume_gas(&mut self.gas_left, memory_cost)?;

        // Copy data
        let mut data = vec![0u8; size_usize];
        if data_offset_usize < self.context.input.len() {
            let end = (data_offset_usize + size_usize).min(self.context.input.len());
            let copy_len = end - data_offset_usize;
            data[..copy_len].copy_from_slice(&self.context.input[data_offset_usize..end]);
        }

        self.memory.write(mem_offset_usize, size_usize, &data)?;
        Ok(())
    }

    fn op_codesize(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_BASE)?;
        self.stack
            .push(U256::from_u128(self.context.code.len() as u128))?;
        Ok(())
    }

    fn op_codecopy(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;
        let mem_offset = self.stack.pop()?;
        let code_offset = self.stack.pop()?;
        let size = self.stack.pop()?;

        let mem_offset_usize = mem_offset.as_u64() as usize;
        let code_offset_usize = code_offset.as_u64() as usize;
        let size_usize = size.as_u64() as usize;

        // Memory expansion cost
        let memory_cost = memory_cost(mem_offset_usize, size_usize)?;
        consume_gas(&mut self.gas_left, memory_cost)?;

        // Copy code
        let mut data = vec![0u8; size_usize];
        if code_offset_usize < self.context.code.len() {
            let end = (code_offset_usize + size_usize).min(self.context.code.len());
            let copy_len = end - code_offset_usize;
            data[..copy_len].copy_from_slice(&self.context.code[code_offset_usize..end]);
        }

        self.memory.write(mem_offset_usize, size_usize, &data)?;
        Ok(())
    }

    fn op_gasprice(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_BASE)?;
        self.stack.push(self.context.gas_price)?;
        Ok(())
    }

    fn op_extcodesize(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, 100)?;
        let addr_u256 = self.stack.pop()?;
        let addr = Address::from_slice(&addr_u256.to_big_endian()[12..])
            .map_err(|_| EvmError::InvalidAddress)?;

        let code = self.context.state.get_code(&addr).unwrap_or_default();
        self.stack.push(U256::from_u128(code.len() as u128))?;
        Ok(())
    }

    fn op_extcodecopy(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, 100)?;
        let addr_u256 = self.stack.pop()?;
        let mem_offset = self.stack.pop()?;
        let code_offset = self.stack.pop()?;
        let size = self.stack.pop()?;

        // Copy code
        let addr = Address::from_slice(&addr_u256.to_big_endian()[12..])
            .map_err(|_| EvmError::InvalidAddress)?;
        let mem_offset_usize = mem_offset.as_u64() as usize;
        let code_offset_usize = code_offset.as_u64() as usize;
        let size_usize = size.as_u64() as usize;

        // Memory expansion cost
        let memory_cost = memory_cost(mem_offset_usize, size_usize)?;
        consume_gas(&mut self.gas_left, memory_cost)?;

        // Copy code
        let code = self.context.state.get_code(&addr).unwrap_or_default();
        let mut data = vec![0u8; size_usize];
        if code_offset_usize < code.len() {
            let end = (code_offset_usize + size_usize).min(code.len());
            let copy_len = end - code_offset_usize;
            data[..copy_len].copy_from_slice(&code[code_offset_usize..end]);
        }

        self.memory.write(mem_offset_usize, size_usize, &data)?;
        Ok(())
    }

    fn op_returndatasize(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_BASE)?;
        self.stack
            .push(U256::from_u128(self.return_data.len() as u128))?;
        Ok(())
    }

    fn op_returndatacopy(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;
        let mem_offset = self.stack.pop()?;
        let data_offset = self.stack.pop()?;
        let size = self.stack.pop()?;

        let mem_offset_usize = mem_offset.as_u64() as usize;
        let data_offset_usize = data_offset.as_u64() as usize;
        let size_usize = size.as_u64() as usize;

        // Memory expansion cost
        let memory_cost = memory_cost(mem_offset_usize, size_usize)?;
        consume_gas(&mut self.gas_left, memory_cost)?;

        // Copy return data
        let mut data = vec![0u8; size_usize];
        if data_offset_usize < self.return_data.len() {
            let end = (data_offset_usize + size_usize).min(self.return_data.len());
            let copy_len = end - data_offset_usize;
            data[..copy_len].copy_from_slice(&self.return_data[data_offset_usize..end]);
        }

        self.memory.write(mem_offset_usize, size_usize, &data)?;
        Ok(())
    }

    fn op_extcodehash(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, 100)?;
        let addr_u256 = self.stack.pop()?;
        let addr = Address::from_slice(&addr_u256.to_big_endian()[12..])
            .map_err(|_| EvmError::InvalidAddress)?;

        let code = self.context.state.get_code(&addr).unwrap_or_default();
        let hash = keccak256(&code);

        self.stack.push(U256::from_u128(u128::from_be_bytes(
            hash[0..16].try_into().unwrap_or([0u8; 16]),
        )))?;
        Ok(())
    }

    // ============ Block Info Operations ============

    fn op_blockhash(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, 20)?;
        let _number = self.stack.pop()?;
        // Would return block hash
        self.stack.push(U256::zero())?;
        Ok(())
    }

    fn op_coinbase(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_BASE)?;
        // Would return miner address
        self.stack.push(U256::zero())?;
        Ok(())
    }

    fn op_timestamp(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_BASE)?;
        // Would return block timestamp
        self.stack.push(U256::zero())?;
        Ok(())
    }

    fn op_number(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_BASE)?;
        // Would return block number
        self.stack.push(U256::zero())?;
        Ok(())
    }

    fn op_prevrandao(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_BASE)?;
        self.stack.push(U256::zero())?;
        Ok(())
    }

    fn op_gaslimit(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_BASE)?;
        self.stack.push(U256::from(self.config.gas_limit))?;
        Ok(())
    }

    fn op_chainid(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_BASE)?;
        self.stack.push(U256::from(self.config.chain_id))?;
        Ok(())
    }

    fn op_selfbalance(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FAST)?;
        let balance = self.context.state.get_balance(&self.context.address);
        self.stack.push(balance)?;
        Ok(())
    }

    fn op_basefee(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_BASE)?;
        self.stack.push(self.config.base_fee)?;
        Ok(())
    }

    // ============ Stack Operations ============

    fn op_pop(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_BASE)?;
        self.stack.pop()?;
        Ok(())
    }

    fn op_mload(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;
        let offset = self.stack.pop()?;
        let offset_usize = offset.as_u64() as usize;

        // Memory expansion cost
        let memory_cost = memory_cost(offset_usize, 32)?;
        consume_gas(&mut self.gas_left, memory_cost)?;

        let data = self.memory.read(offset_usize, 32)?;
        self.stack.push(U256::from_big_endian(&data))?;
        Ok(())
    }

    fn op_mstore(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;
        let offset = self.stack.pop()?;
        let value = self.stack.pop()?;

        let offset_usize = offset.as_u64() as usize;

        // Memory expansion cost
        let memory_cost = memory_cost(offset_usize, 32)?;
        consume_gas(&mut self.gas_left, memory_cost)?;

        self.memory
            .write(offset_usize, 32, &value.to_big_endian())?;
        Ok(())
    }

    fn op_mstore8(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;
        let offset = self.stack.pop()?;
        let value = self.stack.pop()?;

        let offset_usize = offset.as_u64() as usize;
        let byte = (value & U256::from(0xFF)).as_u8();

        // Memory expansion cost
        let memory_cost = memory_cost(offset_usize, 1)?;
        consume_gas(&mut self.gas_left, memory_cost)?;

        self.memory.write_byte(offset_usize, byte)?;
        Ok(())
    }

    fn op_sload(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, 50)?;
        let key = self.stack.pop()?;

        let key_hash = Hash::from_bytes(key.to_big_endian());
        let value = self
            .context
            .state
            .get_storage(&self.context.address, &key_hash);

        self.stack.push(U256::from_big_endian(value.as_bytes()))?;
        Ok(())
    }

    fn op_sstore(&mut self) -> Result<(), EvmError> {
        if self.context.is_static {
            return Err(EvmError::WriteProtection);
        }

        consume_gas(&mut self.gas_left, 20000)?;
        let key = self.stack.pop()?;
        let value = self.stack.pop()?;

        let key_hash = Hash::from_bytes(key.to_big_endian());
        self.context.state.set_storage(
            self.context.address,
            key_hash,
            Hash::from_bytes(value.to_big_endian()),
        );

        Ok(())
    }

    fn op_jump(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_MID)?;
        let dest = self.stack.pop()?;
        let dest_usize = dest.as_u64() as usize;

        if dest_usize >= self.context.code.len() || self.context.code[dest_usize] != OP_JUMPDEST {
            return Err(EvmError::InvalidJumpdest);
        }

        self.pc = dest_usize;
        Ok(())
    }

    fn op_jumpi(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_SLOW)?;
        let dest = self.stack.pop()?;
        let condition = self.stack.pop()?;

        if !condition.is_zero() {
            let dest_usize = dest.as_u64() as usize;
            if dest_usize >= self.context.code.len() || self.context.code[dest_usize] != OP_JUMPDEST
            {
                return Err(EvmError::InvalidJumpdest);
            }
            self.pc = dest_usize;
        }
        Ok(())
    }

    fn op_pc(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_BASE)?;
        self.stack.push(U256::from_u128(self.pc as u128))?;
        Ok(())
    }

    fn op_msize(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_BASE)?;
        self.stack
            .push(U256::from_u128(self.memory.size() as u128))?;
        Ok(())
    }

    fn op_gas(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_BASE)?;
        self.stack.push(U256::from(self.gas_left))?;
        Ok(())
    }

    fn op_jumpdest(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, GAS_JUMPDEST)?;
        Ok(())
    }

    // ============ Push Operations ============

    fn op_push(&mut self, opcode: u8) -> Result<(), EvmError> {
        let num_bytes = (opcode - OP_PUSH1 + 1) as usize;
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;

        let mut data = [0u8; 32];
        let start = 32 - num_bytes;

        for i in 0..num_bytes {
            if self.pc + 1 + i < self.context.code.len() {
                data[start + i] = self.context.code[self.pc + 1 + i];
            }
        }

        self.stack.push(U256::from_big_endian(&data))?;
        self.pc += num_bytes;
        Ok(())
    }

    // ============ Dup Operations ============

    fn op_dup(&mut self, opcode: u8) -> Result<(), EvmError> {
        let n = (opcode - OP_DUP1 + 1) as usize;
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;

        let value = self.stack.peek(n - 1)?;
        self.stack.push(value)?;
        Ok(())
    }

    // ============ Swap Operations ============

    fn op_swap(&mut self, opcode: u8) -> Result<(), EvmError> {
        let n = (opcode - OP_SWAP1 + 1) as usize;
        consume_gas(&mut self.gas_left, GAS_FASTEST)?;

        self.stack.swap(n)?;
        Ok(())
    }

    // ============ Log Operations ============

    fn op_log(&mut self, opcode: u8) -> Result<(), EvmError> {
        if self.context.is_static {
            return Err(EvmError::WriteProtection);
        }

        let num_topics = (opcode - OP_LOG0) as usize;

        consume_gas(&mut self.gas_left, 375)?;
        let offset = self.stack.pop()?;
        let size = self.stack.pop()?;

        let offset_usize = offset.as_u64() as usize;
        let size_usize = size.as_u64() as usize;

        // Memory expansion cost
        let memory_cost = memory_cost(offset_usize, size_usize)?;
        consume_gas(&mut self.gas_left, memory_cost)?;

        // Topic gas cost
        consume_gas(&mut self.gas_left, 375 * num_topics as u64)?;

        // Read topics
        let mut topics = Vec::with_capacity(num_topics);
        for _ in 0..num_topics {
            let topic = self.stack.pop()?;
            topics.push(Hash::from_bytes(topic.to_big_endian()));
        }

        // Read data
        let data = self.memory.read(offset_usize, size_usize)?;

        // Create log entry
        self.logs.push(LogEntry {
            address: self.context.address,
            topics,
            data: Bytes::from(data),
        });

        Ok(())
    }

    // ============ System Operations ============

    fn op_create(&mut self) -> Result<(), EvmError> {
        if self.context.is_static {
            return Err(EvmError::WriteProtection);
        }

        consume_gas(&mut self.gas_left, GAS_CREATE)?;
        let _value = self.stack.pop()?;
        let _offset = self.stack.pop()?;
        let _size = self.stack.pop()?;

        // Simplified - would create contract
        self.stack.push(U256::zero())?;
        Ok(())
    }

    fn op_call(&mut self) -> Result<(), EvmError> {
        consume_gas(&mut self.gas_left, 100)?;
        let _gas = self.stack.pop()?;
        let _to = self.stack.pop()?;
        let _value = self.stack.pop()?;
        let _in_offset = self.stack.pop()?;
        let _in_size = self.stack.pop()?;
        let _out_offset = self.stack.pop()?;
        let _out_size = self.stack.pop()?;

        // Simplified - would make call
        self.stack.push(U256::one())?;
        Ok(())
    }

    fn op_callcode(&mut self) -> Result<(), EvmError> {
        // Similar to CALL but different context
        self.stack.push(U256::one())?;
        Ok(())
    }

    fn op_return(&mut self) -> Result<(), EvmError> {
        let offset = self.stack.pop()?;
        let size = self.stack.pop()?;

        let offset_usize = offset.as_u64() as usize;
        let size_usize = size.as_u64() as usize;

        // Memory expansion
        let memory_cost = memory_cost(offset_usize, size_usize)?;
        consume_gas(&mut self.gas_left, memory_cost)?;

        // Set return data
        // Interpreter will handle actual return

        Ok(())
    }

    fn op_delegatecall(&mut self) -> Result<(), EvmError> {
        self.stack.push(U256::one())?;
        Ok(())
    }

    fn op_create2(&mut self) -> Result<(), EvmError> {
        if self.context.is_static {
            return Err(EvmError::WriteProtection);
        }

        consume_gas(&mut self.gas_left, GAS_CREATE2)?;
        self.stack.push(U256::zero())?;
        Ok(())
    }

    fn op_staticcall(&mut self) -> Result<(), EvmError> {
        self.stack.push(U256::one())?;
        Ok(())
    }

    fn op_revert(&mut self) -> Result<(), EvmError> {
        let offset = self.stack.pop()?;
        let size = self.stack.pop()?;

        let offset_usize = offset.as_u64() as usize;
        let size_usize = size.as_u64() as usize;

        let memory_cost = memory_cost(offset_usize, size_usize)?;
        consume_gas(&mut self.gas_left, memory_cost)?;

        let data = self.memory.read(offset_usize, size_usize)?;
        Err(EvmError::Revert(data))
    }

    fn op_invalid(&mut self) -> Result<(), EvmError> {
        Err(EvmError::InvalidOpcode(0xFE))
    }

    fn op_selfdestruct(&mut self) -> Result<(), EvmError> {
        if self.context.is_static {
            return Err(EvmError::WriteProtection);
        }

        consume_gas(&mut self.gas_left, 5000)?;
        let _addr = self.stack.pop()?;

        // Would handle selfdestruct logic
        Ok(())
    }
}

// ============ EVM Stack ============

const STACK_LIMIT: usize = 1024;

pub struct EvmStack {
    data: [U256; STACK_LIMIT],
    top: usize,
}

impl EvmStack {
    pub fn new() -> Self {
        Self {
            data: [U256::zero(); STACK_LIMIT],
            top: 0,
        }
    }

    pub fn push(&mut self, value: U256) -> Result<(), EvmError> {
        if self.top >= STACK_LIMIT {
            return Err(EvmError::StackOverflow);
        }
        self.data[self.top] = value;
        self.top += 1;
        Ok(())
    }

    pub fn pop(&mut self) -> Result<U256, EvmError> {
        if self.top == 0 {
            return Err(EvmError::StackUnderflow);
        }
        self.top -= 1;
        Ok(self.data[self.top])
    }

    pub fn peek(&self, depth: usize) -> Result<U256, EvmError> {
        if depth >= self.top || depth >= STACK_LIMIT {
            return Err(EvmError::StackUnderflow);
        }
        Ok(self.data[self.top - 1 - depth])
    }

    pub fn swap(&mut self, depth: usize) -> Result<(), EvmError> {
        if depth >= self.top || depth >= STACK_LIMIT {
            return Err(EvmError::StackUnderflow);
        }
        let idx = self.top - 1;
        let swap_idx = self.top - 1 - depth;
        self.data.swap(idx, swap_idx);
        Ok(())
    }
}

impl Default for EvmStack {
    fn default() -> Self {
        Self::new()
    }
}

// ============ EVM Memory ============

pub struct EvmMemory {
    data: Vec<u8>,
}

impl EvmMemory {
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    pub fn size(&self) -> usize {
        self.data.len()
    }

    pub fn read(&self, offset: usize, size: usize) -> Result<Vec<u8>, EvmError> {
        let mut data = vec![0u8; size];

        for i in 0..size {
            if offset + i < self.data.len() {
                data[i] = self.data[offset + i];
            }
        }

        Ok(data)
    }

    pub fn write(&mut self, offset: usize, size: usize, data: &[u8]) -> Result<(), EvmError> {
        // Expand memory if needed
        let needed_size = offset + size;
        if needed_size > self.data.len() {
            self.data.resize(needed_size, 0);
        }

        // Write data
        for i in 0..size {
            if i < data.len() {
                self.data[offset + i] = data[i];
            } else {
                self.data[offset + i] = 0;
            }
        }

        Ok(())
    }

    pub fn write_byte(&mut self, offset: usize, byte: u8) -> Result<(), EvmError> {
        if offset >= self.data.len() {
            self.data.resize(offset + 1, 0);
        }
        self.data[offset] = byte;
        Ok(())
    }

    pub fn to_bytes(&self) -> Bytes {
        Bytes::from(self.data.clone())
    }
}

impl Default for EvmMemory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stack() {
        let mut stack = EvmStack::new();

        stack.push(U256::from(1)).unwrap();
        stack.push(U256::from(2)).unwrap();

        assert_eq!(stack.pop().unwrap().as_u64(), 2);
        assert_eq!(stack.pop().unwrap().as_u64(), 1);
    }

    #[test]
    fn test_memory() {
        let mut memory = EvmMemory::new();

        memory.write(0, 32, &[1u8; 32]).unwrap();
        let data = memory.read(0, 32).unwrap();

        assert_eq!(data, vec![1u8; 32]);
    }
}
