use anchor_lang::{prelude::*, solana_program::instruction::Instruction, InstructionData};
use antegen_utils::thread::{transfer_lamports, ThreadResponse};

use crate::{state::*, ANTEGEN_SQUADS};

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
        address = ANTEGEN_SQUADS
    )]
    pub network_fee: SystemAccount<'info>,

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
    let network_fee: &mut SystemAccount = &mut ctx.accounts.network_fee;
    let snapshot: &Account<Snapshot> = &ctx.accounts.snapshot;
    let snapshot_frame: &Account<SnapshotFrame> = &ctx.accounts.snapshot_frame;
    let thread: &Signer = &ctx.accounts.thread;
    let worker: &mut Account<Worker> = &mut ctx.accounts.worker;

    // Calculate the fee account's usuable balance.
    let commission_lamport_balance: u64 = commission.to_account_info().lamports();
    let commission_data_len: usize = 8 + commission.try_to_vec()?.len();
    let commission_rent_balance: u64 = Rent::get().unwrap().minimum_balance(commission_data_len);
    let rent_with_buffer: u64 = commission_rent_balance.saturating_add(10_000);
    let commission_usable_balance: u64 = commission_lamport_balance.checked_sub(rent_with_buffer).unwrap_or(0);

    // Calculate the commission due to worker
    let commission_bps: u64 = worker.commission_rate.checked_mul(100).unwrap(); // Convert percentage to basis points
    let commission_due: u64 = commission_usable_balance
        .checked_mul(commission_bps)
        .unwrap()
        .checked_div(TOTAL_BASIS_POINTS)
        .unwrap();
    let network_fees: u64 = commission_usable_balance.checked_sub(commission_due).unwrap();

    if commission_due.gt(&0) {
        transfer_lamports(
            &commission.to_account_info(),
            &worker.to_account_info(),
            commission_due,
        )?;
    }

    if network_fees.gt(&0) {
        transfer_lamports(
            &commission.to_account_info(),
            &network_fee.to_account_info(),
            network_fees,
        )?;
    }

    // Build next instruction for the thread.
    let dynamic_instruction = if snapshot_frame
        .id
        .checked_add(1)
        .unwrap()
        .lt(&snapshot.total_frames)
    {
        let next_worker_pubkey: Pubkey = Worker::pubkey(worker.id.checked_add(1).unwrap());
        let next_snapshot_frame_pubkey: Pubkey =
            SnapshotFrame::pubkey(snapshot.key(), snapshot_frame.id.checked_add(1).unwrap());
        Some(
            Instruction {
                program_id: crate::ID,
                accounts: crate::accounts::DistributeFeesProcessWorker {
                    config: config.key(),
                    commission: WorkerCommission::pubkey(next_worker_pubkey),
                    registry: registry.key(),
                    network_fee: network_fee.key(),
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
