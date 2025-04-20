use crate::state::*;
use anchor_lang::{
    prelude::*,
    solana_program::{
        program::invoke_signed, system_instruction::authorize_nonce_account, system_program,
    },
};

#[derive(Accounts)]
#[instruction(hash: String)]
pub struct ThreadClaim<'info> {
    #[account(
        address = thread.authority
    )]
    pub authority: SystemAccount<'info>,

    /// The signatory.
    #[account(mut)]
    pub signatory: Signer<'info>,

    /// CHECK: For new nonce authority
    #[account(
      mut,
      address = thread.nonce_account,
    )]
    pub nonce_account: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [
            SEED_THREAD,
            thread.authority.key().as_ref(),
            thread.id.as_ref(),
        ],
        bump = thread.bump,
    )]
    pub thread: Account<'info, Thread>,

    #[account(address = system_program::ID)]
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<ThreadClaim>, hash: String) -> Result<()> {
    let thread: &mut Account<Thread> = &mut ctx.accounts.thread;
    let nonce_account: &UncheckedAccount = &ctx.accounts.nonce_account;
    let signatory: &Signer = &ctx.accounts.signatory;
    let system_program: &Program<System> = &ctx.accounts.system_program;

    let thread_authority: Pubkey = thread.authority.key();
    let thread_seeds = &[
        SEED_THREAD,
        thread_authority.as_ref(),
        thread.id.as_ref(),
        &[thread.bump],
    ];

    // Invoke the system program with the thread PDA as signer
    invoke_signed(
        &authorize_nonce_account(&nonce_account.key(), &thread.key(), &signatory.key()),
        &[
            nonce_account.to_account_info(),
            thread.to_account_info(),
            system_program.to_account_info(),
        ],
        &[thread_seeds],
    )?;

    thread.last_nonce = hash;
    Ok(())
}
