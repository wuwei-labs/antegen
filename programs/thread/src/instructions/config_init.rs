use crate::{constants::*, state::*};
use anchor_lang::prelude::*;

/// Accounts required by the `config_initialize` instruction.
#[derive(Accounts)]
pub struct ConfigInit<'info> {
    /// The admin initializing the config
    #[account(mut)]
    pub admin: Signer<'info>,

    /// The config account to initialize
    #[account(
        init,
        payer = admin,
        space = ThreadConfig::space(),
        seeds = [SEED_CONFIG],
        bump
    )]
    pub config: Account<'info, ThreadConfig>,

    /// System program
    pub system_program: Program<'info, System>,
}

pub fn config_init(ctx: Context<ConfigInit>) -> Result<()> {
    let config = &mut ctx.accounts.config;
    let admin = &ctx.accounts.admin;

    // Initialize with default values
    config.version = 1;
    config.bump = ctx.bumps.config;
    config.admin = admin.key();
    config.paused = false;
    config.commission_fee = 1000; // 1000 lamports base commission
    config.executor_fee_bps = 9000; // 90% to executor
    config.core_team_bps = 1000; // 10% to core team
    config.grace_period_seconds = 5; // 5 second grace period
    config.fee_decay_seconds = 295; // 295 second decay (total 300s = 5 minutes)

    msg!("Thread config initialized with admin: {}", admin.key());

    Ok(())
}
