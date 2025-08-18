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

pub fn handler(ctx: Context<ConfigInit>) -> Result<()> {
    let config = &mut ctx.accounts.config;
    let admin = &ctx.accounts.admin;

    // Initialize with default values
    config.version = 1;
    config.bump = ctx.bumps.config;
    config.admin = admin.key();
    config.paused = false;
    config.commission_fee = 1000; // 1000 lamports
    config.observer_fee_bps = 9000; // 90% when observer executes
    config.executor_helper_fee_bps = 500; // 5% when helping observer
    config.observer_share_bps = 8500; // 85% observer share when helped
    config.core_team_bps = 1000; // 10% core team
    config.priority_window = 120; // 2 minutes

    msg!("Thread config initialized with admin: {}", admin.key());

    Ok(())
}
