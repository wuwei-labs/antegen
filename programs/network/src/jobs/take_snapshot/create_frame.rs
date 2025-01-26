use {
    crate::state::*,
    anchor_lang::{
        prelude::*,
        solana_program::{instruction::Instruction, system_program},
        InstructionData,
    },
    antegen_utils::thread::{ThreadResponse, PAYER_PUBKEY},
    std::mem::size_of,
};

#[derive(Accounts)]
pub struct TakeSnapshotCreateFrame<'info> {
    #[account(address = Config::pubkey())]
    pub config: Account<'info, Config>,

    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        address = Registry::pubkey(),
        constraint = registry.locked
    )]
    pub registry: Account<'info, Registry>,

    #[account(
        mut,
        seeds = [
            SEED_SNAPSHOT,
            snapshot.id.to_be_bytes().as_ref(),
        ],
        bump,
        constraint = registry.current_epoch.checked_add(1).unwrap().eq(&snapshot.id),
        constraint = snapshot.total_frames < registry.total_workers,
    )]
    pub snapshot: Account<'info, Snapshot>,

    #[account(
        init,
        seeds = [
            SEED_SNAPSHOT_FRAME,
            snapshot.key().as_ref(),
            snapshot.total_frames.to_be_bytes().as_ref(),
        ],
        bump,
        payer = payer,
        space = 8 + size_of::<SnapshotFrame>(),
    )]
    pub snapshot_frame: Account<'info, SnapshotFrame>,

    #[account(address = system_program::ID)]
    pub system_program: Program<'info, System>,

    #[account(address = config.epoch_thread)]
    pub thread: Signer<'info>,

    #[account(
        address = worker.pubkey(),
        constraint = worker.id.eq(&snapshot.total_frames),
    )]
    pub worker: Account<'info, Worker>,
}

pub fn handler(ctx: Context<TakeSnapshotCreateFrame>) -> Result<ThreadResponse> {
    // Get accounts.
    let config = &ctx.accounts.config;
    let registry = &ctx.accounts.registry;
    let snapshot = &mut ctx.accounts.snapshot;
    let snapshot_frame = &mut ctx.accounts.snapshot_frame;
    let system_program = &ctx.accounts.system_program;
    let thread = &ctx.accounts.thread;
    let worker = &ctx.accounts.worker;

    // Initialize snapshot frame account.
    snapshot_frame.init(
        snapshot.total_frames,
        snapshot.key(),
        worker.key(),
    )?;

    // Update snapshot total workers.
    snapshot.total_frames = snapshot.total_frames.checked_add(1).unwrap();

    // Build the next instruction for the thread.
    let dynamic_instruction = if snapshot.total_frames.lt(&registry.total_workers) {
        let next_snapshot_frame_pubkey =
            SnapshotFrame::pubkey(snapshot.key(), snapshot_frame.id.checked_add(1).unwrap());
        let next_worker_pubkey = Worker::pubkey(worker.id.checked_add(1).unwrap());
        Some(
            Instruction {
                program_id: crate::ID,
                accounts: crate::accounts::TakeSnapshotCreateFrame {
                    config: config.key(),
                    payer: PAYER_PUBKEY,
                    registry: registry.key(),
                    snapshot: snapshot.key(),
                    snapshot_frame: next_snapshot_frame_pubkey,
                    system_program: system_program.key(),
                    thread: thread.key(),
                    worker: next_worker_pubkey
                }
                .to_account_metas(Some(true)),
                data: crate::instruction::TakeSnapshotCreateFrame {}.data(),
            }
            .into(),
        )
    } else {
        None
    };

    Ok(ThreadResponse {
        dynamic_instruction,
        close_to: None,
        trigger: None,
    })
}
