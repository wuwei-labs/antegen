//! This program orchestrates a Antegen worker network deployed across a Solana cluster.
//! It implements a PoW protocol, if the pool has space and the worker provides work
//! they're accepted into the pool

pub mod errors;
pub mod state;

mod instructions;
mod jobs;

use anchor_lang::prelude::*;
use antegen_utils::thread::*;
use instructions::*;
use jobs::*;
use state::*;

declare_id!("AgNet6qmh75bjFULcS9RQijUoWwkCtSiSwXM1K3Ujn6Z");
pub const ANTEGEN_SQUADS: Pubkey = pubkey!("14b1BKm2md7GgP6ccZd2u4cAvBsqsmFjxokzQUXoqGzC");

#[program]
pub mod network_program {
    pub use super::*;

    pub fn config_update(ctx: Context<ConfigUpdate>, settings: ConfigSettings) -> Result<()> {
        config_update::handler(ctx, settings)
    }

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        initialize::handler(ctx)
    }

    pub fn pool_create(ctx: Context<PoolCreate>) -> Result<()> {
        pool_create::handler(ctx)
    }

    pub fn pool_rotate(ctx: Context<PoolRotate>) -> Result<()> {
        pool_rotate::handler(ctx)
    }

    pub fn pool_update(ctx: Context<PoolUpdate>, settings: PoolSettings) -> Result<()> {
        pool_update::handler(ctx, settings)
    }

    pub fn registry_claim(ctx: Context<RegistryClaim>) -> Result<()> {
        registry_claim::handler(ctx)
    }

    pub fn registry_nonce_hash(ctx: Context<RegistryNonceHash>) -> Result<ThreadResponse> {
        registry_nonce_hash::handler(ctx)
    }

    pub fn registry_reset(ctx: Context<RegistryReset>) -> Result<()> {
        registry_reset::handler(ctx)
    }

    pub fn registry_unlock(ctx: Context<RegistryUnlock>) -> Result<()> {
        registry_unlock::handler(ctx)
    }

    pub fn worker_create(ctx: Context<WorkerCreate>) -> Result<()> {
        worker_create::handler(ctx)
    }

    pub fn worker_update(ctx: Context<WorkerUpdate>, settings: WorkerSettings) -> Result<()> {
        worker_update::handler(ctx, settings)
    }

    // DistributeFees job
    pub fn distribute_fees_job(ctx: Context<DistributeFeesJob>) -> Result<ThreadResponse> {
        jobs::distribute_fees::job::handler(ctx)
    }

    pub fn distribute_fees_process_worker(
        ctx: Context<DistributeFeesProcessWorker>,
    ) -> Result<ThreadResponse> {
        jobs::distribute_fees::process_worker::handler(ctx)
    }

    pub fn distribute_fees_process_snapshot(
        ctx: Context<DistributeFeesProcessSnapshot>,
    ) -> Result<ThreadResponse> {
        jobs::distribute_fees::process_snapshot::handler(ctx)
    }

    // TakeSnapshot job
    pub fn take_snapshot_job(ctx: Context<TakeSnapshotJob>) -> Result<ThreadResponse> {
        jobs::take_snapshot::job::handler(ctx)
    }

    pub fn take_snapshot_create_frame(
        ctx: Context<TakeSnapshotCreateFrame>,
    ) -> Result<ThreadResponse> {
        jobs::take_snapshot::create_frame::handler(ctx)
    }

    pub fn take_snapshot_create_snapshot(
        ctx: Context<TakeSnapshotCreateSnapshot>,
    ) -> Result<ThreadResponse> {
        jobs::take_snapshot::create_snapshot::handler(ctx)
    }

    // IncrementEpoch job
    pub fn increment_epoch(ctx: Context<EpochCutover>) -> Result<ThreadResponse> {
        jobs::increment_epoch::job::handler(ctx)
    }

    // Delete snapshot
    pub fn delete_snapshot_job(ctx: Context<DeleteSnapshotJob>) -> Result<ThreadResponse> {
        jobs::delete_snapshot::job::handler(ctx)
    }

    pub fn delete_snapshot_process_snapshot(
        ctx: Context<DeleteSnapshotProcessSnapshot>,
    ) -> Result<ThreadResponse> {
        jobs::delete_snapshot::process_snapshot::handler(ctx)
    }

    pub fn delete_snapshot_process_frame(
        ctx: Context<DeleteSnapshotProcessFrame>,
    ) -> Result<ThreadResponse> {
        jobs::delete_snapshot::process_frame::handler(ctx)
    }
}
