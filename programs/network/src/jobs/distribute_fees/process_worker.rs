use anchor_lang::{prelude::*, solana_program::instruction::Instruction, InstructionData};
use antegen_utils::thread::ThreadResponse;

use crate::state::*;

pub const TOTAL_BASIS_POINTS: u64 = 10_000;

#[derive(Accounts)]
pub struct DistributeFeesProcessWorker<'info> {
    #[account(address = Config::pubkey())]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        seeds = [
            SEED_WORKER_COMMISSION,
            commission.worker.as_ref(),
        ],
        bump,
        has_one = worker,
    )]
    pub commission: Account<'info, WorkerCommission>,

    #[account(address = Registry::pubkey())]
    pub registry: Account<'info, Registry>,

    #[account(
        mut,
        seeds = [
            SEED_REGISTRY_FEE,
            registry.key().as_ref(),
        ],
        bump,
        has_one = registry,
    )]
    pub registry_fee: Account<'info, RegistryFee>,

    #[account(
        address = snapshot.pubkey(),
        constraint = snapshot.id.eq(&registry.current_epoch)
    )]
    pub snapshot: Account<'info, Snapshot>,

    #[account(
        address = snapshot_frame.pubkey(),
        has_one = snapshot,
        has_one = worker,
    )]
    pub snapshot_frame: Account<'info, SnapshotFrame>,

    #[account(address = config.epoch_thread)]
    pub thread: Signer<'info>,

    #[account(mut)]
    pub worker: Account<'info, Worker>,
}

pub fn handler(ctx: Context<DistributeFeesProcessWorker>) -> Result<ThreadResponse> {
    // Get accounts.
    let config: &Account<Config> = &ctx.accounts.config;
    let commission: &mut Account<WorkerCommission> = &mut ctx.accounts.commission;
    let registry: &Account<Registry> = &ctx.accounts.registry;
    let registry_fee: &mut Account<RegistryFee> = &mut ctx.accounts.registry_fee;
    let snapshot: &Account<Snapshot> = &ctx.accounts.snapshot;
    let snapshot_frame: &Account<SnapshotFrame> = &ctx.accounts.snapshot_frame;
    let thread: &Signer = &ctx.accounts.thread;
    let worker: &mut Account<Worker> = &mut ctx.accounts.worker;

    // Calculate the fee account's usuable balance.
    let commission_lamport_balance = commission.to_account_info().lamports();
    let commission_data_len = 8 + commission.try_to_vec()?.len();
    let commission_rent_balance = Rent::get().unwrap().minimum_balance(commission_data_len);
    let commission_usable_balance = commission_lamport_balance.checked_sub(commission_rent_balance).unwrap();

    // Calculate the commission to be retained by the worker.
    let commission_bps = worker.commission_rate.checked_mul(100).unwrap(); // Convert percentage to basis points
    let commission_balance = commission_usable_balance
        .checked_mul(commission_bps)
        .unwrap()
        .checked_div(TOTAL_BASIS_POINTS)
        .unwrap();
    let registry_fees = commission_usable_balance.checked_sub(commission_balance).unwrap();

    // Transfer commission to the worker.
    if commission_balance.gt(&0) {
        **commission.to_account_info().try_borrow_mut_lamports()? = commission
        .to_account_info()
        .lamports()
        .checked_sub(commission_balance)
        .unwrap();

        **worker.to_account_info().try_borrow_mut_lamports()? = worker
            .to_account_info()
            .lamports()
            .checked_add(commission_balance)
            .unwrap();
    }

    // Transfer network fees to registry.
    if registry_fees.gt(&0) {
        **commission.to_account_info().try_borrow_mut_lamports()? = commission
            .to_account_info()
            .lamports()
            .checked_sub(registry_fees)  // Subtract registry_fees from commission
            .unwrap();

        **registry_fee.to_account_info().try_borrow_mut_lamports()? = registry_fee
            .to_account_info()
            .lamports()
            .checked_add(registry_fees)
            .unwrap();
    }

    // Build next instruction for the thread.
    let dynamic_instruction = if snapshot_frame
        .id
        .checked_add(1)
        .unwrap()
        .lt(&snapshot.total_frames)
    {
        let next_worker_pubkey = Worker::pubkey(worker.id.checked_add(1).unwrap());
        let next_snapshot_frame_pubkey =
            SnapshotFrame::pubkey(snapshot.key(), snapshot_frame.id.checked_add(1).unwrap());
        Some(
            Instruction {
                program_id: crate::ID,
                accounts: crate::accounts::DistributeFeesProcessWorker {
                    config: config.key(),
                    commission: WorkerCommission::pubkey(next_worker_pubkey),
                    registry: registry.key(),
                    registry_fee: registry_fee.key(),
                    snapshot: snapshot.key(),
                    snapshot_frame: next_snapshot_frame_pubkey,
                    thread: thread.key(),
                    worker: next_worker_pubkey,
                }
                .to_account_metas(Some(true)),
                data: crate::instruction::DistributeFeesProcessWorker {}.data(),
            }
            .into(),
        )
    } else {
        None
    };

    Ok(ThreadResponse {
        dynamic_instruction,
        ..ThreadResponse::default()
    })
}
