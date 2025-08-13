use anchor_lang::prelude::*;

#[error_code]
pub enum AntegenNetworkError {
    #[msg("The commission must be between 0 and the max commission in basis points")]
    InvalidCommissionRate,

    #[msg("The authority address cannot be used as the builder signatory")]
    InvalidSignatory,
}
