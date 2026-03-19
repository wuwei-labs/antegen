use anchor_lang::prelude::*;

#[error_code]
pub enum AntegenFiberError {
    #[msg("Invalid compiled instruction data")]
    InvalidCompiledInstruction,

    #[msg("Fiber account is already initialized")]
    AlreadyInitialized,

    #[msg("Derived PDA does not match the provided fiber account")]
    InvalidFiberPDA,

    #[msg("Fiber account has insufficient lamports for rent")]
    InsufficientRent,
}
