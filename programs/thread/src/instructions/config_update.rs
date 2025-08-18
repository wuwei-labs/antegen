use crate::{constants::*, errors::*, state::*};
use anchor_lang::prelude::*;

/// Parameters for updating the thread config
#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct ConfigUpdateParams {
    pub admin: Option<Pubkey>,
    pub paused: Option<bool>,
    pub commission_fee: Option<u64>,
    pub observer_fee_bps: Option<u64>,
    pub executor_helper_fee_bps: Option<u64>,
    pub observer_share_bps: Option<u64>,
    pub core_team_bps: Option<u64>,
    pub priority_window: Option<i64>,
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

pub fn handler(ctx: Context<ConfigUpdate>, params: ConfigUpdateParams) -> Result<()> {
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
    if let Some(observer_fee_bps) = params.observer_fee_bps {
        require!(
            observer_fee_bps <= 10000,
            AntegenThreadError::InvalidFeePercentage
        );
        config.observer_fee_bps = observer_fee_bps;
    }
    
    if let Some(executor_helper_fee_bps) = params.executor_helper_fee_bps {
        require!(
            executor_helper_fee_bps <= 10000,
            AntegenThreadError::InvalidFeePercentage
        );
        config.executor_helper_fee_bps = executor_helper_fee_bps;
    }
    
    if let Some(observer_share_bps) = params.observer_share_bps {
        require!(
            observer_share_bps <= 10000,
            AntegenThreadError::InvalidFeePercentage
        );
        config.observer_share_bps = observer_share_bps;
    }
    
    if let Some(core_team_bps) = params.core_team_bps {
        require!(
            core_team_bps <= 10000,
            AntegenThreadError::InvalidFeePercentage
        );
        config.core_team_bps = core_team_bps;
    }
    
    // Validate that total fees don't exceed 100%
    let total_when_observer_executes = config.observer_fee_bps + config.core_team_bps;
    let total_when_helped = config.observer_share_bps + config.executor_helper_fee_bps + config.core_team_bps;
    
    require!(
        total_when_observer_executes <= 10000,
        AntegenThreadError::InvalidFeePercentage
    );
    require!(
        total_when_helped <= 10000,
        AntegenThreadError::InvalidFeePercentage
    );
    
    // Update priority window if provided
    if let Some(priority_window) = params.priority_window {
        require!(
            priority_window >= 0 && priority_window <= 600, // Max 10 minutes
            AntegenThreadError::InvalidPriorityWindow
        );
        config.priority_window = priority_window;
        msg!("Priority window updated to: {} seconds", priority_window);
    }
    
    Ok(())
}