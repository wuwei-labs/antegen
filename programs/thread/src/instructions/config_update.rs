use crate::{constants::*, errors::*, state::*};
use anchor_lang::prelude::*;

/// Parameters for updating the thread config
#[derive(AnchorSerialize, AnchorDeserialize, Default)]
pub struct ConfigUpdateParams {
    pub admin: Option<Pubkey>,
    pub paused: Option<bool>,
    pub commission_fee: Option<u64>,
    pub executor_fee_bps: Option<u64>,
    pub core_team_bps: Option<u64>,
    pub grace_period_seconds: Option<i64>,
    pub fee_decay_seconds: Option<i64>,
}

/// Accounts required by the `config_update` instruction.
#[derive(Accounts)]
pub struct ConfigUpdate<'info> {
    /// The admin updating the config
    #[account(
        mut,
        constraint = admin.key() == config.admin @ AntegenThreadError::InvalidAuthority
    )]
    pub admin: Signer<'info>,

    /// The config account to update
    #[account(
        mut,
        seeds = [SEED_CONFIG],
        bump = config.bump
    )]
    pub config: Account<'info, ThreadConfig>,
}

pub fn config_update(ctx: Context<ConfigUpdate>, params: ConfigUpdateParams) -> Result<()> {
    let config = &mut ctx.accounts.config;

    // Update admin if provided
    if let Some(new_admin) = params.admin {
        config.admin = new_admin;
        msg!("Config admin updated to: {}", new_admin);
    }

    // Update pause state if provided
    if let Some(paused) = params.paused {
        config.paused = paused;
        msg!("Config paused state updated to: {}", paused);
    }

    // Update commission fee if provided
    if let Some(commission_fee) = params.commission_fee {
        config.commission_fee = commission_fee;
        msg!("Commission fee updated to: {} lamports", commission_fee);
    }

    // Update fee percentages if provided
    if let Some(executor_fee_bps) = params.executor_fee_bps {
        require!(
            executor_fee_bps <= 10000,
            AntegenThreadError::InvalidFeePercentage
        );
        config.executor_fee_bps = executor_fee_bps;
        msg!("Executor fee updated to: {} bps", executor_fee_bps);
    }

    if let Some(core_team_bps) = params.core_team_bps {
        require!(
            core_team_bps <= 10000,
            AntegenThreadError::InvalidFeePercentage
        );
        config.core_team_bps = core_team_bps;
        msg!("Core team fee updated to: {} bps", core_team_bps);
    }

    // Update timing parameters if provided
    if let Some(grace_period) = params.grace_period_seconds {
        require!(
            grace_period >= 0 && grace_period <= 60, // Max 60 seconds grace
            AntegenThreadError::InvalidFeePercentage
        );
        config.grace_period_seconds = grace_period;
        msg!("Grace period updated to: {} seconds", grace_period);
    }

    if let Some(decay_period) = params.fee_decay_seconds {
        require!(
            decay_period >= 0 && decay_period <= 600, // Max 10 minutes decay
            AntegenThreadError::InvalidFeePercentage
        );
        config.fee_decay_seconds = decay_period;
        msg!("Fee decay period updated to: {} seconds", decay_period);
    }

    // Validate that total fees equal 100%
    let total_fees = config.executor_fee_bps + config.core_team_bps;
    require!(
        total_fees == 10000,
        AntegenThreadError::InvalidFeePercentage
    );

    Ok(())
}
