use anchor_lang::prelude::*;

#[error_code]
pub enum AntegenNetworkError {
    #[msg("The builder is already in the pool")]
    BuilderInPool,

    #[msg("The pool is locked - cannot add builder right now")]
    PoolLocked,

    #[msg("The pool is full - cannot add builder right now")]
    PoolFull,

    #[msg("Can't create another pool")]
    PoolOverflow,

    #[msg("The builder is not in the pool")]
    BuilderNotInPool,

    #[msg("The commission rate must be an integer between 0 and 100")]
    InvalidCommissionRate,

    #[msg("The authority address cannot be used as the builder signatory")]
    InvalidSignatory,

    #[msg("The registry is locked and may not be updated right now")]
    RegistryLocked,
}
