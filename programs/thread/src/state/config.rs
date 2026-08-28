use anchor_lang::prelude::*;

/// Trait for calculating commission fees
pub trait CommissionCalculator {
    fn calculate_commission_multiplier(&self, time_since_ready: i64) -> f64;
    fn calculate_effective_commission(&self, time_since_ready: i64) -> u64;
    fn calculate_executor_fee(&self, effective_commission: u64) -> u64;
    fn calculate_core_team_fee(&self, effective_commission: u64) -> u64;
}

/// Struct to hold payment details
#[derive(Debug)]
pub struct PaymentDetails {
    pub fee_payer_reimbursement: u64,
    pub executor_commission: u64,
    pub core_team_fee: u64,
}

/// Trait for processing payments
pub trait PaymentProcessor {
    fn calculate_payments(
        &self,
        time_since_ready: i64,
        balance_change: i64,
        forgo_commission: bool,
    ) -> PaymentDetails;

    fn should_pay(&self, balance_change: i64) -> bool {
        balance_change <= 0 // Pay if balance decreased or stayed same
    }

    fn calculate_reimbursement(&self, balance_change: i64) -> u64 {
        if balance_change < 0 {
            balance_change.unsigned_abs()
        } else if balance_change > 0 {
            0 // Already paid by inner instruction
        } else {
            5000u64 // Default reimbursement
        }
    }
}

/// Global configuration for the thread program
#[account]
#[derive(Debug, InitSpace)]
pub struct ThreadConfig {
    /// Version for future upgrades
    pub version: u64,
    /// Bump seed for PDA
    pub bump: u8,
    /// Admin who can update configuration
    pub admin: Pubkey,
    /// Global pause flag for all threads
    pub paused: bool,
    /// Base commission fee in lamports (when executed on time)
    pub commission_fee: u64,
    /// Fee percentage for executor (9000 = 90%)
    pub executor_fee_bps: u64,
    /// Core team fee percentage (1000 = 10%)
    pub core_team_bps: u64,
    /// Grace period in seconds where full commission applies
    pub grace_period_seconds: i64,
    /// Decay period in seconds after grace (commission decays to 0)
    pub fee_decay_seconds: i64,
}

/// Total on-chain size of the config account: Anchor's 8-byte discriminator
/// plus the state itself.
pub const CONFIG_ACCOUNT_SPACE: usize = 8 + ThreadConfig::INIT_SPACE;

impl Default for ThreadConfig {
    /// The fee policy a freshly initialized config starts with.
    ///
    /// `bump` and `admin` have no meaningful default — `config_init` fills
    /// them in from the account it just created and the signer that created it.
    fn default() -> Self {
        Self {
            version: 1,
            bump: 0,
            admin: Pubkey::default(),
            paused: false,
            commission_fee: 1000,   // lamports, base commission
            executor_fee_bps: 9000, // 90% to executor
            core_team_bps: 1000,    // 10% to core team
            grace_period_seconds: 5,
            fee_decay_seconds: 295, // 300s total, with the grace period
        }
    }
}

impl ThreadConfig {
    pub fn pubkey() -> Pubkey {
        Pubkey::find_program_address(&[crate::SEED_CONFIG], &crate::ID).0
    }

    pub fn space() -> usize {
        CONFIG_ACCOUNT_SPACE
    }
}

impl CommissionCalculator for ThreadConfig {
    fn calculate_commission_multiplier(&self, time_since_ready: i64) -> f64 {
        // Within grace period: full commission.
        if time_since_ready <= self.grace_period_seconds {
            return 1.0;
        }

        // A zero or negative decay window has no slope to interpolate along;
        // dividing by it produced NaN, which then silently became a zero fee.
        // Say so directly instead.
        if self.fee_decay_seconds <= 0 {
            return 0.0;
        }

        let decay_end = match self.grace_period_seconds.checked_add(self.fee_decay_seconds) {
            Some(end) => end,
            None => return 0.0,
        };
        if time_since_ready > decay_end {
            // After grace + decay period: no commission.
            return 0.0;
        }

        // Within decay period: linear decay from 100% to 0%.
        let time_into_decay = time_since_ready.saturating_sub(self.grace_period_seconds) as f64;
        let decay_progress = time_into_decay / self.fee_decay_seconds as f64;
        (1.0 - decay_progress).clamp(0.0, 1.0)
    }

    fn calculate_effective_commission(&self, time_since_ready: i64) -> u64 {
        let multiplier = self.calculate_commission_multiplier(time_since_ready);
        if !multiplier.is_finite() || multiplier <= 0.0 {
            return 0;
        }

        // Scale through basis points rather than multiplying a u64 by an f64.
        // `commission_fee` is admin-set and unbounded, and `f64 as u64`
        // saturates silently at the top of the range.
        let bps = (multiplier.min(1.0) * 10_000.0) as u64;
        self.commission_fee
            .checked_mul(bps)
            .map(|scaled| scaled / 10_000)
            .unwrap_or(self.commission_fee)
    }

    fn calculate_executor_fee(&self, effective_commission: u64) -> u64 {
        scale_bps(effective_commission, self.executor_fee_bps)
    }

    fn calculate_core_team_fee(&self, effective_commission: u64) -> u64 {
        scale_bps(effective_commission, self.core_team_bps)
    }
}

/// Applies a basis-point rate to a lamport amount.
///
/// Widened to `u128` first: the plain `u64` product of an admin-set commission
/// and a rate overflows before the division brings it back into range, which
/// aborts the execution rather than paying out a capped fee.
fn scale_bps(amount: u64, bps: u64) -> u64 {
    let scaled = (amount as u128).saturating_mul(bps as u128);
    let fee = scaled.checked_div(10_000).unwrap_or(0);
    u64::try_from(fee).unwrap_or(u64::MAX)
}

impl PaymentProcessor for ThreadConfig {
    fn calculate_payments(
        &self,
        time_since_ready: i64,
        balance_change: i64,
        forgo_commission: bool,
    ) -> PaymentDetails {
        // Calculate effective commission
        let effective_commission = self.calculate_effective_commission(time_since_ready);

        // Calculate reimbursement and commission for executor
        let (fee_payer_reimbursement, executor_commission) = if self.should_pay(balance_change) {
            let reimbursement = self.calculate_reimbursement(balance_change);
            let commission = if !forgo_commission {
                self.calculate_executor_fee(effective_commission)
            } else {
                0
            };
            (reimbursement, commission)
        } else {
            (0, 0)
        };

        // Calculate core team fee
        let core_team_fee = self.calculate_core_team_fee(effective_commission);

        PaymentDetails {
            fee_payer_reimbursement,
            executor_commission,
            core_team_fee,
        }
    }
}
