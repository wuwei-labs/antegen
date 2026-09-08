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
    /// Replaces the whole fee model at once.
    ///
    /// Whole rather than field-by-field because its fields are not independent:
    /// the inclusion fee drops from 5000 to 2500 at the same moment the
    /// resource rate becomes non-zero, and applying either half alone prices
    /// executions against a world that does not exist. One value means the
    /// transition is a single atomic edit.
    pub fee_model: Option<FeeModel>,
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

    /// The config account to update.
    ///
    /// The `realloc` is the migration. A config written before the fee model
    /// existed is too short to hold it: reading works, because `Trailing`
    /// yields defaults once the bytes run out, but writing the struct back
    /// would not fit. Sizing to `ThreadConfig::space()` here grows it on the
    /// next update and is a no-op on every update after that, so the migration
    /// is "run `config_update` once" rather than an instruction of its own that
    /// someone has to remember exists.
    #[account(
        mut,
        seeds = [SEED_CONFIG],
        bump = config.bump,
        realloc = ThreadConfig::space(),
        realloc::payer = admin,
        realloc::zero = false,
        constraint = config.to_account_info().owner == &crate::ID
            @ AntegenThreadError::InvalidAccountOwner,
    )]
    pub config: Account<'info, ThreadConfig>,

    /// Funds the account growth on the first update after the fee model landed.
    pub system_program: Program<'info, System>,
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
            (0..=60).contains(&grace_period), // Max 60 seconds grace
            AntegenThreadError::InvalidFeePercentage
        );
        config.grace_period_seconds = grace_period;
        msg!("Grace period updated to: {} seconds", grace_period);
    }

    if let Some(decay_period) = params.fee_decay_seconds {
        require!(
            (0..=600).contains(&decay_period), // Max 10 minutes decay
            AntegenThreadError::InvalidFeePercentage
        );
        config.fee_decay_seconds = decay_period;
        msg!("Fee decay period updated to: {} seconds", decay_period);
    }

    if let Some(fee_model) = params.fee_model {
        require!(
            fee_model.resource_rate_den > 0,
            AntegenThreadError::InvalidFeeRate
        );
        // The published ramp ends at 0.5 lamports per cost unit, so a rate
        // above 1 is a misplaced decimal point rather than a policy. Catching
        // it here costs nothing and saves `max_reimbursement_per_exec` from
        // silently absorbing it on every execution until someone notices.
        require!(
            fee_model.resource_rate_num <= fee_model.resource_rate_den,
            AntegenThreadError::InvalidFeeRate
        );
        require!(
            fee_model.max_slack_bps as u64 <= 10_000,
            AntegenThreadError::InvalidFeePercentage
        );
        msg!(
            "Fee model updated: inclusion {} lamports, rate {}/{} per unit, slack cap {} bps, \
             ceiling {} lamports",
            fee_model.inclusion_fee_lamports,
            fee_model.resource_rate_num,
            fee_model.resource_rate_den,
            fee_model.max_slack_bps,
            fee_model.max_reimbursement_per_exec,
        );
        config.fee_model = fee_model.into();
    }

    // Validate that total fees equal 100%
    let total_fees = config
        .executor_fee_bps
        .checked_add(config.core_team_bps)
        .ok_or(AntegenThreadError::InvalidFeePercentage)?;
    require!(
        total_fees == 10000,
        AntegenThreadError::InvalidFeePercentage
    );

    Ok(())
}
