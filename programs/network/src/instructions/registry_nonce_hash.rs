use anchor_lang::solana_program::instruction::Instruction;
use antegen_utils::thread::ThreadResponse;

use {crate::state::*, anchor_lang::prelude::*, anchor_lang::InstructionData};

#[derive(Accounts)]
pub struct RegistryNonceHash<'info> {
    #[account(address = Config::pubkey())]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        seeds = [SEED_REGISTRY],
        bump
    )]
    pub registry: Account<'info, Registry>,

    #[account(address = config.hasher_thread)]
    pub thread: Signer<'info>,
}

pub fn handler(ctx: Context<RegistryNonceHash>) -> Result<ThreadResponse> {
    let registry = &mut ctx.accounts.registry;
    registry.hash_nonce()?;
    msg!("Registry nonce updated to: {}", registry.nonce);

    // Create a dynamic instruction to call RegistryNonceHash again.
    let dynamic_instruction = Some(
        Instruction {
            program_id: crate::ID,
            accounts: crate::accounts::RegistryNonceHash {
                config: ctx.accounts.config.key(),
                registry: registry.key(),
                thread: ctx.accounts.thread.key(),
            }
            .to_account_metas(Some(true)),
            data: crate::instruction::RegistryNonceHash {}.data(),
        }
        .into(),
    );

    Ok(ThreadResponse {
        dynamic_instruction,
        ..ThreadResponse::default()
    })
}
