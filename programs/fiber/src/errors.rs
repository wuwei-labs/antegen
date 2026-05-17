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

    #[msg("Lookup tables list exceeds maximum of 4 entries")]
    LookupTablesExceedMax,

    #[msg("Lookup tables are not supported on legacy fibers — close and recreate")]
    LegacyFiberLookupTablesUnsupported,

    #[msg("Fiber account data is malformed or has unknown discriminator")]
    InvalidFiberData,
}
