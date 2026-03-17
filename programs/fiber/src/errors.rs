use anchor_lang::prelude::*;

#[error_code]
pub enum AntegenFiberError {
    #[msg("Invalid compiled instruction data")]
    InvalidCompiledInstruction,
}
