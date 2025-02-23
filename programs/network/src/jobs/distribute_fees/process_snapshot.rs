use anchor_lang::{prelude::*, solana_program::instruction::Instruction, InstructionData};
use antegen_utils::thread::ThreadResponse;

use crate::{state::*, ANTEGEN_SQUADS};

#[derive(Accounts)]
pub struct DistributeFeesProcessSnapshot<'info> {
    #[account(address = Config::pubkey())]
    pub config: Account<'info, Config>,

    #[account(seeds = [SEED_REGISTRY], bump)]
    pub registry: Account<'info, Registry>,

    #[account(
        address = snapshot.pubkey(),
        constraint = snapshot.id.eq(&registry.current_epoch)
    )]
    pub snapshot: Account<'info, Snapshot>,

    #[account(address = config.epoch_thread)]
    pub thread: Signer<'info>,
}

pub fn handler(ctx: Context<DistributeFeesProcessSnapshot>) -> Result<ThreadResponse> {
    let config: &Account<Config> = &ctx.accounts.config;
    let registry: &mut Account<Registry> = &mut ctx.accounts.registry;
    let snapshot: &Account<Snapshot> = &ctx.accounts.snapshot;
    let thread: &Signer = &ctx.accounts.thread;

    Ok(ThreadResponse {
        dynamic_instruction: if snapshot.total_frames.gt(&0) {
            Some(
                Instruction {
                    program_id: crate::ID,
                    accounts: crate::accounts::DistributeFeesProcessWorker {
                        config: config.key(),
                        commission: WorkerCommission::pubkey(Worker::pubkey(0)),
                        registry: registry.key(),
                        network_fee: ANTEGEN_SQUADS,
                        snapshot: snapshot.key(),
                        snapshot_frame: SnapshotFrame::pubkey(snapshot.key(), 0),
                        thread: thread.key(),
                        worker: Worker::pubkey(0),
                    }
                    .to_account_metas(Some(true)),
                    data: crate::instruction::DistributeFeesProcessWorker {}.data(),
                }
                .into(),
            )
        } else {
            None
        },
        ..ThreadResponse::default()
    })
}
