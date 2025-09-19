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
        balance_change <= 0  // Pay if balance decreased or stayed same
    }
    
    fn calculate_reimbursement(&self, balance_change: i64) -> u64 {
        if balance_change < 0 {
            balance_change.abs() as u64
        } else if balance_change > 0 {
            0  // Already paid by inner instruction
        } else {
            5000u64  // Default reimbursement
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

impl ThreadConfig {
    pub fn pubkey() -> Pubkey {
        Pubkey::find_program_address(&[crate::SEED_CONFIG], &crate::ID).0
    }

    pub fn space() -> usize {
        8 + Self::INIT_SPACE
    }
}

impl CommissionCalculator for ThreadConfig {
    fn calculate_commission_multiplier(&self, time_since_ready: i64) -> f64 {
        if time_since_ready <= self.grace_period_seconds {
            // Within grace period: full commission
            1.0
        } else if time_since_ready <= self.grace_period_seconds + self.fee_decay_seconds {
            // Within decay period: linear decay from 100% to 0%
            let time_into_decay = (time_since_ready - self.grace_period_seconds) as f64;
            let decay_progress = time_into_decay / self.fee_decay_seconds as f64;
            1.0 - decay_progress
        } else {
            // After grace + decay period: no commission
            0.0
        }
    }
    
    fn calculate_effective_commission(&self, time_since_ready: i64) -> u64 {
        let multiplier = self.calculate_commission_multiplier(time_since_ready);
        (self.commission_fee as f64 * multiplier) as u64
    }
    
    fn calculate_executor_fee(&self, effective_commission: u64) -> u64 {
        (effective_commission * self.executor_fee_bps) / 10_000
    }
    
    fn calculate_core_team_fee(&self, effective_commission: u64) -> u64 {
        (effective_commission * self.core_team_bps) / 10_000
    }
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
