use anchor_lang::prelude::*;

#[error_code]
pub enum AntegenNetworkError {
    #[msg("The worker is already in the pool")]
    AlreadyInPool,

    #[msg("The commission rate must be an integer between 0 and 100")]
    InvalidCommissionRate,

    #[msg("The authority address cannot be used as the worker signatory")]
    InvalidSignatory,

    #[msg("The registry is locked and may not be updated right now")]
    RegistryLocked,

    #[msg("The worker cannot rotate into the pool right now")]
    PoolFull,
}
