use {
    crate::{constants::*, errors::*, state::*},
    anchor_lang::prelude::*,
    antegen_network_program::state::{Builder, Registry, SEED_BUILDER, SEED_REGISTRY},
};

#[derive(Accounts)]
pub struct ThreadClaim<'info> {
    #[account(mut)]
    pub signatory: Signer<'info>,

    #[account(
        mut,
        seeds = [
            SEED_THREAD,
            thread.authority.as_ref(),
            thread.id.as_slice(),
        ],
        bump = thread.bump,
    )]
    pub thread: Account<'info, Thread>,

    #[account(
        seeds = [
            SEED_BUILDER,
            builder.id.to_be_bytes().as_ref(),
        ],
        bump = builder.bump,
        constraint = builder.signatory == signatory.key() @ ThreadError::InvalidSignatory,
        constraint = builder.is_active @ ThreadError::BuilderNotActive,
    )]
    pub builder: Account<'info, Builder>,

    #[account(
        seeds = [SEED_REGISTRY],
        bump = registry.bump,
    )]
    pub registry: Account<'info, Registry>,
}

pub fn handler(ctx: Context<ThreadClaim>) -> Result<()> {
    let thread = &mut ctx.accounts.thread;
    let builder = &ctx.accounts.builder;
    let clock = Clock::get()?;

    // Clear expired builders if claim window has passed
    thread.clear_expired_builders(clock.unix_timestamp, CLAIM_WINDOW_SECONDS);

    // Add this builder to the thread
    thread.add_builder(builder.id, clock.unix_timestamp)?;

    Ok(())
}
