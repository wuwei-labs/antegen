use anchor_lang::{prelude::*, InstructionData, solana_program::instruction::Instruction};
use antegen_utils::thread::ThreadResponse;

use crate::state::*;

#[derive(Accounts)]
pub struct DeleteSnapshotProcessFrame<'info> {
    #[account(address = Config::pubkey())]
    pub config: Account<'info, Config>,

    #[account(
        address = Registry::pubkey(),
        constraint = !registry.locked
    )]
    pub registry: Account<'info, Registry>,

    #[account(
        mut,
        seeds = [
            SEED_SNAPSHOT,
            snapshot.id.to_be_bytes().as_ref(),
        ],
        bump,
        constraint = snapshot.id.lt(&registry.current_epoch)
    )]
    pub snapshot: Account<'info, Snapshot>,

    #[account(
        mut,
        seeds = [
            SEED_SNAPSHOT_FRAME,
            snapshot_frame.snapshot.as_ref(),
            snapshot_frame.id.to_be_bytes().as_ref(),
        ],
        bump,
        has_one = snapshot,
    )]
    pub snapshot_frame: Account<'info, SnapshotFrame>,

    #[account(
        mut, 
        address = config.epoch_thread
    )]
    pub thread: Signer<'info>,
}

pub fn handler(ctx: Context<DeleteSnapshotProcessFrame>) -> Result<ThreadResponse> {
    // Get accounts
    let config = &ctx.accounts.config;
    let registry = &ctx.accounts.registry;
    let snapshot = &mut ctx.accounts.snapshot;
    let snapshot_frame = &mut ctx.accounts.snapshot_frame;
    let thread = &mut ctx.accounts.thread;

    // If this is the last frame in the snapshot, then close the snapshot account.
    if snapshot_frame.id.checked_add(1).unwrap().eq(&snapshot.total_frames) {
        let snapshot_lamports = snapshot.to_account_info().lamports();
        **snapshot.to_account_info().lamports.borrow_mut() = 0;
        **thread.to_account_info().lamports.borrow_mut() = thread
            .to_account_info()
            .lamports()
            .checked_add(snapshot_lamports)
            .unwrap();
    }

    // Build the next instruction.
    let dynamic_instruction = if 
        snapshot_frame
        .id
        .checked_add(1)
        .unwrap()
        .lt(&snapshot.total_frames)
    {
        // There are no more entries in this frame. Move on to the next frame.
        Some(
            Instruction {
                program_id: crate::ID,
                accounts: crate::accounts::DeleteSnapshotProcessFrame {
                    config: config.key(),
                    registry: registry.key(),
                    snapshot: snapshot.key(),
                    snapshot_frame: SnapshotFrame::pubkey(snapshot.key(), snapshot_frame.id.checked_add(1).unwrap()),
                    thread: thread.key(),
                }.to_account_metas(Some(true)),
                data: crate::instruction::DeleteSnapshotProcessFrame {}.data()
            }.into()
        )
    } else {
        None
    };

    Ok( ThreadResponse { dynamic_instruction, ..ThreadResponse::default() } )
}
