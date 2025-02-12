use anchor_lang::solana_program::instruction::Instruction;
use anchor_lang::{prelude::*, InstructionData, Discriminator};
use antegen_utils::thread::ThreadResponse;
use crate::state::*;

#[derive(Accounts)]
pub struct EpochCutoverAccounts<'info> {
    #[account(address = Config::pubkey())]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        seeds = [SEED_REGISTRY],
        bump,
    )]
    pub registry: Account<'info, Registry>,

    #[account(address = config.epoch_thread)]
    pub thread: Signer<'info>,
}

#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct EpochCutoverIx {}

// Explicit implementation of InstructionData for EpochCutoverIx.
impl InstructionData for EpochCutoverIx {
    fn data(&self) -> Vec<u8> {
        self.try_to_vec().unwrap()
    }
}

// Manually implement Discriminator using the correct computed value.
impl Discriminator for EpochCutoverIx {
    const DISCRIMINATOR: &'static [u8] = &[0x50, 0x72, 0xad, 0x36, 0x30, 0x45, 0x1e, 0xf1];
}

pub fn handler(ctx: Context<EpochCutoverAccounts>) -> Result<ThreadResponse> {
    let registry = &mut ctx.accounts.registry;
    registry.current_epoch = registry.current_epoch.checked_add(1).unwrap();
    registry.locked = false;
    msg!("Epoch updated to: {}", registry.current_epoch);

    // Build a dynamic instruction to re-trigger this job.
    let dynamic_instruction = Some(
        Instruction {
            program_id: crate::ID,
            accounts: crate::accounts::EpochCutoverAccounts {
                config: ctx.accounts.config.key(),
                registry: registry.key(),
                thread: ctx.accounts.thread.key(),
            }
            .to_account_metas(Some(true)),
            data: EpochCutoverIx {}.data(),
        }
        .into(),
    );

    Ok(ThreadResponse {
        dynamic_instruction,
        ..ThreadResponse::default()
    })
}