//! This program orchestrates a Antegen worker network deployed across a Solana cluster.
//! It implements a PoW protocol, if the pool has space and the worker provides work
//! they're accepted into the pool

pub mod errors;
pub mod state;

mod instructions;

use anchor_lang::prelude::*;
use instructions::*;
use state::*;

declare_id!("AgNet6qmh75bjFULcS9RQijUoWwkCtSiSwXM1K3Ujn6Z");
pub const ANTEGEN_SQUADS: Pubkey = pubkey!("14b1BKm2md7GgP6ccZd2u4cAvBsqsmFjxokzQUXoqGzC");
pub const TOTAL_BASIS_POINTS: u64 = 10_000;

pub const CURRENT_BUILDER_VERSION: u64 = 1;
pub const CURRENT_CONFIG_VERSION: u64 = 1;
pub const CURRENT_POOL_VERSION: u64 = 1;
pub const CURRENT_REGISTRY_VERSION: u64 = 1;

#[program]
pub mod network_program {
    pub use super::*;

    pub fn builder_add(ctx: Context<BuilderAdd>) -> Result<()> {
        builder_add::handler(ctx)
    }

    pub fn builder_create(ctx: Context<BuilderCreate>) -> Result<()> {
        builder_create::handler(ctx)
    }

    pub fn builder_remove(ctx: Context<BuilderRemove>) -> Result<()> {
        builder_remove::handler(ctx)
    }

    pub fn builder_update(ctx: Context<BuilderUpdate>, settings: BuilderSettings) -> Result<()> {
        builder_update::handler(ctx, settings)
    }

    pub fn config_update(ctx: Context<ConfigUpdate>, settings: ConfigSettings) -> Result<()> {
        config_update::handler(ctx, settings)
    }

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        initialize::handler(ctx)
    }

    pub fn pool_create(ctx: Context<PoolCreate>) -> Result<()> {
        pool_create::handler(ctx)
    }

    pub fn registry_reset(ctx: Context<RegistryReset>) -> Result<()> {
        registry_reset::handler(ctx)
    }

    pub fn registry_unlock(ctx: Context<RegistryUnlock>) -> Result<()> {
        registry_unlock::handler(ctx)
    }
}
