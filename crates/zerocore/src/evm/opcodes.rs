//! EVM Opcode definitions and constants

/// Stop opcode
pub const OP_STOP: u8 = 0x00;
/// Addition opcode
pub const OP_ADD: u8 = 0x01;
/// Multiplication opcode
pub const OP_MUL: u8 = 0x02;
/// Subtraction opcode
pub const OP_SUB: u8 = 0x03;
/// Integer division opcode
pub const OP_DIV: u8 = 0x04;
/// Signed integer division opcode
pub const OP_SDIV: u8 = 0x05;
/// Modulo operation opcode
pub const OP_MOD: u8 = 0x06;
/// Signed modulo operation opcode
pub const OP_SMOD: u8 = 0x07;
/// Addition modulo opcode
pub const OP_ADDMOD: u8 = 0x08;
/// Multiplication modulo opcode
pub const OP_MULMOD: u8 = 0x09;
/// Exponential operation opcode
pub const OP_EXP: u8 = 0x0a;
/// Extend length of two's complement signed integer opcode
pub const OP_SIGNEXTEND: u8 = 0x0b;

/// Less-than comparison opcode
pub const OP_LT: u8 = 0x10;
/// Greater-than comparison opcode
pub const OP_GT: u8 = 0x11;
/// Signed less-than comparison opcode
pub const OP_SLT: u8 = 0x12;
/// Signed greater-than comparison opcode
pub const OP_SGT: u8 = 0x13;
/// Equality comparison opcode
pub const OP_EQ: u8 = 0x14;
/// Simple not opcode
pub const OP_ISZERO: u8 = 0x15;

/// Bitwise AND opcode
pub const OP_AND: u8 = 0x16;
/// Bitwise OR opcode
pub const OP_OR: u8 = 0x17;
/// Bitwise XOR opcode
pub const OP_XOR: u8 = 0x18;
/// Bitwise NOT opcode
pub const OP_NOT: u8 = 0x19;
/// Retrieve single byte from word opcode
pub const OP_BYTE: u8 = 0x1a;
/// Left shift opcode
pub const OP_SHL: u8 = 0x1b;
/// Logical right shift opcode
pub const OP_SHR: u8 = 0x1c;
/// Arithmetic (signed) right shift opcode
pub const OP_SAR: u8 = 0x1d;

/// Keccak-256 hash opcode
pub const OP_SHA3: u8 = 0x20;

/// Get address of currently executing account opcode
pub const OP_ADDRESS: u8 = 0x30;
/// Get balance of the given account opcode
pub const OP_BALANCE: u8 = 0x31;
/// Get address of execution originator opcode
pub const OP_ORIGIN: u8 = 0x32;
/// Get address of caller opcode
pub const OP_CALLER: u8 = 0x33;
/// Get deposited value by the instruction/transaction opcode
pub const OP_CALLVALUE: u8 = 0x34;
/// Get input data of current environment opcode
pub const OP_CALLDATALOAD: u8 = 0x35;
/// Get size of input data in current environment opcode
pub const OP_CALLDATASIZE: u8 = 0x36;
/// Copy input data in current environment to memory opcode
pub const OP_CALLDATACOPY: u8 = 0x37;
/// Get size of code running in current environment opcode
pub const OP_CODESIZE: u8 = 0x38;
/// Copy code running in current environment to memory opcode
pub const OP_CODECOPY: u8 = 0x39;
/// Get price of gas in current environment opcode
pub const OP_GASPRICE: u8 = 0x3a;
/// Get size of code running in current environment with given offset opcode
pub const OP_EXTCODESIZE: u8 = 0x3b;
/// Copy code running in current environment to memory with given offset opcode
pub const OP_EXTCODECOPY: u8 = 0x3c;
/// Get size of return data buffer opcode
pub const OP_RETURNDATASIZE: u8 = 0x3d;
/// Copy return data buffer to memory opcode
pub const OP_RETURNDATACOPY: u8 = 0x3e;
/// Get hash of account running in current environment opcode
pub const OP_EXTCODEHASH: u8 = 0x3f;

/// Get hash of most recent complete block opcode
pub const OP_BLOCKHASH: u8 = 0x40;
/// Get the block's coinbase address opcode
pub const OP_COINBASE: u8 = 0x41;
/// Get the block's timestamp opcode
pub const OP_TIMESTAMP: u8 = 0x42;
/// Get the block's number opcode
pub const OP_NUMBER: u8 = 0x43;
/// Get the block's difficulty opcode
pub const OP_PREVRANDAO: u8 = 0x44;
/// Get the block's gas limit opcode
pub const OP_GASLIMIT: u8 = 0x45;
/// Get chain ID opcode
pub const OP_CHAINID: u8 = 0x46;
/// Get balance of currently executing account opcode
pub const OP_SELFBALANCE: u8 = 0x47;
/// Get base fee opcode
pub const OP_BASEFEE: u8 = 0x48;

/// Pop value from stack opcode
pub const OP_POP: u8 = 0x50;
/// Load word from memory opcode
pub const OP_MLOAD: u8 = 0x51;
/// Save word to memory opcode
pub const OP_MSTORE: u8 = 0x52;
/// Save byte to memory opcode
pub const OP_MSTORE8: u8 = 0x53;
/// Load word from storage opcode
pub const OP_SLOAD: u8 = 0x54;
/// Save word to storage opcode
pub const OP_SSTORE: u8 = 0x55;
/// Alter the program counter opcode
pub const OP_JUMP: u8 = 0x56;
/// Conditionally alter the program counter opcode
pub const OP_JUMPI: u8 = 0x57;
/// Get the program counter opcode
pub const OP_PC: u8 = 0x58;
/// Get the size of active memory opcode
pub const OP_MSIZE: u8 = 0x59;
/// Get the amount of gas available opcode
pub const OP_GAS: u8 = 0x5a;
/// Alter the program counter to a marked location opcode
pub const OP_JUMPDEST: u8 = 0x5b;

/// Push N bytes onto stack (OP_PUSH1 = push 1 byte, OP_PUSH32 = push 32 bytes)
pub const OP_PUSH1: u8 = 0x60;
pub const OP_PUSH2: u8 = 0x61;
pub const OP_PUSH3: u8 = 0x62;
pub const OP_PUSH4: u8 = 0x63;
pub const OP_PUSH5: u8 = 0x64;
pub const OP_PUSH6: u8 = 0x65;
pub const OP_PUSH7: u8 = 0x66;
pub const OP_PUSH8: u8 = 0x67;
pub const OP_PUSH9: u8 = 0x68;
pub const OP_PUSH10: u8 = 0x69;
pub const OP_PUSH11: u8 = 0x6a;
pub const OP_PUSH12: u8 = 0x6b;
pub const OP_PUSH13: u8 = 0x6c;
pub const OP_PUSH14: u8 = 0x6d;
pub const OP_PUSH15: u8 = 0x6e;
pub const OP_PUSH16: u8 = 0x6f;
pub const OP_PUSH17: u8 = 0x70;
pub const OP_PUSH18: u8 = 0x71;
pub const OP_PUSH19: u8 = 0x72;
pub const OP_PUSH20: u8 = 0x73;
pub const OP_PUSH21: u8 = 0x74;
pub const OP_PUSH22: u8 = 0x75;
pub const OP_PUSH23: u8 = 0x76;
pub const OP_PUSH24: u8 = 0x77;
pub const OP_PUSH25: u8 = 0x78;
pub const OP_PUSH26: u8 = 0x79;
pub const OP_PUSH27: u8 = 0x7a;
pub const OP_PUSH28: u8 = 0x7b;
pub const OP_PUSH29: u8 = 0x7c;
pub const OP_PUSH30: u8 = 0x7d;
pub const OP_PUSH31: u8 = 0x7e;
pub const OP_PUSH32: u8 = 0x7f;

/// Duplicate Nth stack item (OP_DUP1 = duplicate top, OP_DUP16 = duplicate 16th)
pub const OP_DUP1: u8 = 0x80;
pub const OP_DUP2: u8 = 0x81;
pub const OP_DUP3: u8 = 0x82;
pub const OP_DUP4: u8 = 0x83;
pub const OP_DUP5: u8 = 0x84;
pub const OP_DUP6: u8 = 0x85;
pub const OP_DUP7: u8 = 0x86;
pub const OP_DUP8: u8 = 0x87;
pub const OP_DUP9: u8 = 0x88;
pub const OP_DUP10: u8 = 0x89;
pub const OP_DUP11: u8 = 0x8a;
pub const OP_DUP12: u8 = 0x8b;
pub const OP_DUP13: u8 = 0x8c;
pub const OP_DUP14: u8 = 0x8d;
pub const OP_DUP15: u8 = 0x8e;
pub const OP_DUP16: u8 = 0x8f;

/// Exchange Nth stack item with top (OP_SWAP1 = exchange top and 2nd, OP_SWAP16 = exchange top and 17th)
pub const OP_SWAP1: u8 = 0x90;
pub const OP_SWAP2: u8 = 0x91;
pub const OP_SWAP3: u8 = 0x92;
pub const OP_SWAP4: u8 = 0x93;
pub const OP_SWAP5: u8 = 0x94;
pub const OP_SWAP6: u8 = 0x95;
pub const OP_SWAP7: u8 = 0x96;
pub const OP_SWAP8: u8 = 0x97;
pub const OP_SWAP9: u8 = 0x98;
pub const OP_SWAP10: u8 = 0x99;
pub const OP_SWAP11: u8 = 0x9a;
pub const OP_SWAP12: u8 = 0x9b;
pub const OP_SWAP13: u8 = 0x9c;
pub const OP_SWAP14: u8 = 0x9d;
pub const OP_SWAP15: u8 = 0x9e;
pub const OP_SWAP16: u8 = 0x9f;

/// Append log entry with N topics (OP_LOG0 = no topics, OP_LOG4 = 4 topics)
pub const OP_LOG0: u8 = 0xa0;
pub const OP_LOG1: u8 = 0xa1;
pub const OP_LOG2: u8 = 0xa2;
pub const OP_LOG3: u8 = 0xa3;
pub const OP_LOG4: u8 = 0xa4;

/// Create new account opcode
pub const OP_CREATE: u8 = 0xf0;
/// Message-call into account opcode
pub const OP_CALL: u8 = 0xf1;
/// Message-call with alternative code opcode
pub const OP_CALLCODE: u8 = 0xf2;
/// Return from opcode
pub const OP_RETURN: u8 = 0xf3;
/// Delegate-call opcode
pub const OP_DELEGATECALL: u8 = 0xf4;
/// Create new account with salt opcode
pub const OP_CREATE2: u8 = 0xf5;
/// Static-call opcode
pub const OP_STATICCALL: u8 = 0xfa;
/// Revert opcode
pub const OP_REVERT: u8 = 0xfd;
/// Invalid opcode
pub const OP_INVALID: u8 = 0xfe;
/// Self-destruct opcode
pub const OP_SELFDESTRUCT: u8 = 0xff;

/// Get the number of bytes needed to push for a PUSH opcode
pub fn push_bytes(opcode: u8) -> usize {
    if (OP_PUSH1..=OP_PUSH32).contains(&opcode) {
        (opcode - OP_PUSH1 + 1) as usize
    } else {
        0
    }
}

/// Check if opcode is a PUSH
pub fn is_push(opcode: u8) -> bool {
    (OP_PUSH1..=OP_PUSH32).contains(&opcode)
}

/// Check if opcode is a DUP
pub fn is_dup(opcode: u8) -> bool {
    (OP_DUP1..=OP_DUP16).contains(&opcode)
}

/// Check if opcode is a SWAP
pub fn is_swap(opcode: u8) -> bool {
    (OP_SWAP1..=OP_SWAP16).contains(&opcode)
}

/// Check if opcode is a LOG
pub fn is_log(opcode: u8) -> bool {
    (OP_LOG0..=OP_LOG4).contains(&opcode)
}

/// Get the number of topics for a LOG opcode
pub fn log_topics(opcode: u8) -> usize {
    if (OP_LOG0..=OP_LOG4).contains(&opcode) {
        (opcode - OP_LOG0) as usize
    } else {
        0
    }
}

/// Get opcode name
pub fn opcode_name(opcode: u8) -> &'static str {
    match opcode {
        OP_STOP => "STOP",
        OP_ADD => "ADD",
        OP_MUL => "MUL",
        OP_SUB => "SUB",
        OP_DIV => "DIV",
        OP_SDIV => "SDIV",
        OP_MOD => "MOD",
        OP_SMOD => "SMOD",
        OP_ADDMOD => "ADDMOD",
        OP_MULMOD => "MULMOD",
        OP_EXP => "EXP",
        OP_SIGNEXTEND => "SIGNEXTEND",
        OP_LT => "LT",
        OP_GT => "GT",
        OP_SLT => "SLT",
        OP_SGT => "SGT",
        OP_EQ => "EQ",
        OP_ISZERO => "ISZERO",
        OP_AND => "AND",
        OP_OR => "OR",
        OP_XOR => "XOR",
        OP_NOT => "NOT",
        OP_BYTE => "BYTE",
        OP_SHL => "SHL",
        OP_SHR => "SHR",
        OP_SAR => "SAR",
        OP_SHA3 => "SHA3",
        OP_ADDRESS => "ADDRESS",
        OP_BALANCE => "BALANCE",
        OP_ORIGIN => "ORIGIN",
        OP_CALLER => "CALLER",
        OP_CALLVALUE => "CALLVALUE",
        OP_CALLDATALOAD => "CALLDATALOAD",
        OP_CALLDATASIZE => "CALLDATASIZE",
        OP_CALLDATACOPY => "CALLDATACOPY",
        OP_CODESIZE => "CODESIZE",
        OP_CODECOPY => "CODECOPY",
        OP_GASPRICE => "GASPRICE",
        OP_EXTCODESIZE => "EXTCODESIZE",
        OP_EXTCODECOPY => "EXTCODECOPY",
        OP_RETURNDATASIZE => "RETURNDATASIZE",
        OP_RETURNDATACOPY => "RETURNDATACOPY",
        OP_EXTCODEHASH => "EXTCODEHASH",
        OP_BLOCKHASH => "BLOCKHASH",
        OP_COINBASE => "COINBASE",
        OP_TIMESTAMP => "TIMESTAMP",
        OP_NUMBER => "NUMBER",
        OP_PREVRANDAO => "PREVRANDAO",
        OP_GASLIMIT => "GASLIMIT",
        OP_CHAINID => "CHAINID",
        OP_SELFBALANCE => "SELFBALANCE",
        OP_BASEFEE => "BASEFEE",
        OP_POP => "POP",
        OP_MLOAD => "MLOAD",
        OP_MSTORE => "MSTORE",
        OP_MSTORE8 => "MSTORE8",
        OP_SLOAD => "SLOAD",
        OP_SSTORE => "SSTORE",
        OP_JUMP => "JUMP",
        OP_JUMPI => "JUMPI",
        OP_PC => "PC",
        OP_MSIZE => "MSIZE",
        OP_GAS => "GAS",
        OP_JUMPDEST => "JUMPDEST",
        OP_PUSH1..=OP_PUSH32 => "PUSH",
        OP_DUP1..=OP_DUP16 => "DUP",
        OP_SWAP1..=OP_SWAP16 => "SWAP",
        OP_LOG0..=OP_LOG4 => "LOG",
        OP_CREATE => "CREATE",
        OP_CALL => "CALL",
        OP_CALLCODE => "CALLCODE",
        OP_RETURN => "RETURN",
        OP_DELEGATECALL => "DELEGATECALL",
        OP_CREATE2 => "CREATE2",
        OP_STATICCALL => "STATICCALL",
        OP_REVERT => "REVERT",
        OP_INVALID => "INVALID",
        OP_SELFDESTRUCT => "SELFDESTRUCT",
        _ => "UNKNOWN",
    }
}
