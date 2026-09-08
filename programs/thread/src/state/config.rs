use anchor_lang::prelude::*;
use antegen_fiber_program::state::Trailing;

/// Basis-point denominator.
const BPS_DENOMINATOR: u64 = 10_000;

/// Trait for calculating commission fees
pub trait CommissionCalculator {
    fn calculate_commission_multiplier(&self, time_since_ready: i64) -> f64;
    fn calculate_effective_commission(&self, time_since_ready: i64) -> u64;
    fn calculate_executor_fee(&self, effective_commission: u64) -> u64;
    fn calculate_core_team_fee(&self, effective_commission: u64) -> u64;
}

/// Struct to hold payment details
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PaymentDetails {
    pub fee_payer_reimbursement: u64,
    pub executor_commission: u64,
    pub core_team_fee: u64,
}

impl PaymentDetails {
    pub fn total(&self) -> u64 {
        self.fee_payer_reimbursement
            .saturating_add(self.executor_commission)
            .saturating_add(self.core_team_fee)
    }

    /// Cut these payments down to what the thread can actually afford,
    /// returning whether anything had to be given up.
    ///
    /// Reimbursement is paid first, then the executor's commission, then the
    /// core team's share. That ordering is the point: reimbursement is money
    /// the executor has already spent out of pocket, while the other two are
    /// profit. A thread that can only cover part of what it owes should cover
    /// the debt before the dividend.
    ///
    /// Needed at all because reimbursement stopped being a small constant. A
    /// flat 5000 was always affordable; a figure that scales with the fiber's
    /// own compute is not, and the alternative to clamping is an execution that
    /// fails *after* the fiber has run — leaving the executor with the full
    /// transaction fee, no reimbursement, and a thread that will do the same
    /// thing to the next executor who tries.
    pub fn clamp_to(&mut self, available: u64) -> bool {
        if self.total() <= available {
            return false;
        }

        let mut remaining = available;
        for field in [
            &mut self.fee_payer_reimbursement,
            &mut self.executor_commission,
            &mut self.core_team_fee,
        ] {
            let paid = (*field).min(remaining);
            *field = paid;
            remaining = remaining.saturating_sub(paid);
        }

        true
    }
}

/// Trait for processing payments
pub trait PaymentProcessor {
    fn calculate_payments(
        &self,
        time_since_ready: i64,
        balance_change: i64,
        forgo_commission: bool,
        transaction_fee: u64,
    ) -> PaymentDetails;

    fn should_pay(&self, balance_change: i64) -> bool {
        balance_change <= 0 // Pay if balance decreased or stayed same
    }

    /// What the executor is owed back.
    ///
    /// Two separate costs, which used to be treated as alternatives. The
    /// transaction fee is paid on every execution and is what `transaction_fee`
    /// prices — a flat 5000 until an admin configures [`FeeModel`], a measured
    /// figure once they have. Lamports the inner instruction moved off the
    /// executor are a second, independent cost, and an execution that incurs
    /// both should return both.
    ///
    /// Adding them is a change from the previous behaviour, which reimbursed
    /// the drained lamports *instead of* the fee. That was survivable while the
    /// fee was a flat 5000 and is not once it scales with compute: a fiber that
    /// takes one lamport from the executor would otherwise cancel a reimburse-
    /// ment worth tens of thousands.
    fn calculate_reimbursement(&self, balance_change: i64, transaction_fee: u64) -> u64 {
        if balance_change > 0 {
            // The inner instruction paid the executor rather than costing them.
            return 0;
        }

        transaction_fee.saturating_add(balance_change.unsigned_abs())
    }
}

/// What an execution cost the executor, and how that turns into lamports.
///
/// Grouped into one struct, and appended to [`ThreadConfig`] behind a single
/// [`Trailing`], because the config account is already live on mainnet. Adding
/// plain fields would make every existing copy too short to deserialize and
/// brick the program until a migration that would itself have to read the
/// account it cannot read. `Trailing` yields defaults when the bytes run out
/// instead, so an un-migrated config reads fine and the account grows the next
/// time `config_update` runs.
///
/// Defaults are all zero, and [`FeeModel::is_configured`] treats that as "not
/// set up", falling back to the flat reimbursement the program has always paid.
/// So the new fields change nothing until an admin deliberately fills them in.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default, InitSpace, PartialEq, Eq)]
pub struct FeeModel {
    /// The part of the fee that does not scale with compute.
    ///
    /// Today that is the whole fee: 5000 lamports per signature. Under
    /// SIMD-0553 the leader's share stays the same in absolute terms while the
    /// burned half is replaced by the resource fee, so this drops to 2500 at
    /// the same moment `resource_rate_num` becomes non-zero.
    ///
    /// Zero means the model is not configured. Doubles as the sentinel because
    /// no real deployment charges nothing per signature.
    pub inclusion_fee_lamports: u64,

    /// Lamports per cost unit, as a fraction.
    ///
    /// A fraction rather than a scaled integer because the published ramp is
    /// 0.1, 0.25 and 0.5 lamports per unit — none integers, all exact as `n/d`.
    pub resource_rate_num: u64,
    pub resource_rate_den: u64,

    /// Compute the program burns but cannot see: the ComputeBudget instructions
    /// ahead of the first meter reading, and its own tail after the last one.
    /// Small, roughly fixed, and not measurable from inside the span it
    /// brackets.
    pub execution_overhead_units: u32,

    /// The non-compute terms of `requested_cost_units` — signatures, write
    /// locks, instruction data bytes, loaded-accounts pages. A thread exec's
    /// shape is stable enough for one number.
    pub base_cost_units: u32,

    /// Ceiling on the margin a thread will pay for, in basis points.
    ///
    /// The fee is charged on the limit the executor *requested* while
    /// reimbursement is billed on what was *used*, so a well-behaved executor
    /// is short by whatever margin their estimate needed in order not to
    /// exhaust the budget and fail. Reimbursing their measured overshoot closes
    /// that exactly; this caps how much overshoot is reimbursable.
    ///
    /// The cap is what makes the executor's own number safe to read at all.
    /// Overshoot can only ever *reduce* the payout — asking for more than this
    /// earns nothing and the difference comes out of the executor's pocket,
    /// while asking for less risks exhausting the budget.
    pub max_slack_bps: u16,

    /// Hard ceiling on a single execution's reimbursement.
    ///
    /// The backstop for everything going wrong at once — a mis-entered rate, a
    /// fiber that burns the full 1.4M ceiling, an overhead constant that
    /// drifted. Bounds what one execution can take from a thread's balance
    /// regardless of what the arithmetic produces.
    pub max_reimbursement_per_exec: u64,
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
    /// How execution reimbursement is priced.
    ///
    /// Appended, so it must stay last: `Trailing` distinguishes "the admin has
    /// not migrated this account" from "a value follows" by whether the buffer
    /// is exhausted, and a field after it would never be reached on a config
    /// written before this existed.
    pub fee_model: Trailing<FeeModel>,
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
            // A config created fresh gets the fee model already filled in;
            // only accounts predating it read back unconfigured.
            fee_model: Trailing(FeeModel::preconfigured()),
        }
    }
}

impl FeeModel {
    /// The values to migrate an existing config to.
    ///
    /// Deliberately inert: `resource_rate_num` is zero, so the resource term
    /// vanishes and reimbursement is `inclusion_fee_lamports` — the same flat
    /// 5000 the program has always paid. Migrating therefore changes nothing on
    /// its own, and the rate is turned on separately once its feature gate has
    /// activated.
    pub fn preconfigured() -> Self {
        Self {
            inclusion_fee_lamports: crate::constants::TRANSACTION_BASE_FEE_REIMBURSEMENT,
            resource_rate_num: 0,
            resource_rate_den: 1,
            // Three ComputeBudget instructions at 150 units each, plus the
            // payment tail after the final reading.
            execution_overhead_units: 3_000,
            // One signature (720), three write locks (900), ten bytes of
            // instruction data, one 32 KiB loaded-accounts page (8).
            base_cost_units: 1_638,
            // Headroom over the ~5% overshoot a well-tuned oracle can reach.
            max_slack_bps: 1_000,
            max_reimbursement_per_exec: 2_000_000,
        }
    }

    /// Whether an admin has filled this in.
    ///
    /// A config written before the fee model existed reads back as all zeroes,
    /// which is indistinguishable from one an admin deliberately zeroed — and
    /// should behave the same way either way, since neither can price anything.
    pub fn is_configured(&self) -> bool {
        self.inclusion_fee_lamports > 0
    }

    /// Units burned between two readings of the remaining budget, plus the
    /// overhead outside them.
    ///
    /// `saturating_sub` rather than a checked subtraction because the ordering
    /// is an invariant of the call site, not of the type: readings only
    /// decrease, so `end > start` means the arguments were passed the wrong way
    /// round. Saturating to zero yields a minimum reimbursement instead of
    /// aborting an execution that has already done its work.
    pub fn units_used(&self, remaining_at_start: u64, remaining_at_end: u64) -> u64 {
        remaining_at_start
            .saturating_sub(remaining_at_end)
            .saturating_add(self.execution_overhead_units as u64)
    }

    /// How far the requested budget ran ahead of what was used, in basis points
    /// of what was used, capped at [`Self::max_slack_bps`].
    ///
    /// The unspent budget is simply `remaining_at_end` — the overhead terms
    /// cancel when the estimated budget and estimated usage are subtracted,
    /// leaving what the meter had left. It slightly overstates the true
    /// overshoot, because the reading is taken before the payment tail that
    /// `execution_overhead_units` accounts for, but the cap bounds that.
    pub fn overshoot_bps(&self, remaining_at_start: u64, remaining_at_end: u64) -> u64 {
        remaining_at_end
            .saturating_mul(BPS_DENOMINATOR)
            .checked_div(self.units_used(remaining_at_start, remaining_at_end))
            .unwrap_or(0)
            .min(self.max_slack_bps as u64)
    }

    /// What the thread owes the executor for the compute this execution burned.
    ///
    /// Saturating throughout. Every input is either measured or admin-set, and
    /// an admin who enters a rate with too many zeroes should get a
    /// reimbursement pinned at `max_reimbursement_per_exec`, not an execution
    /// that fails to complete after the fiber has already run.
    pub fn reimbursement(&self, remaining_at_start: u64, remaining_at_end: u64) -> u64 {
        if !self.is_configured() {
            return crate::constants::TRANSACTION_BASE_FEE_REIMBURSEMENT;
        }

        let used = self.units_used(remaining_at_start, remaining_at_end);
        let slack = self.overshoot_bps(remaining_at_start, remaining_at_end);

        let billable = (used as u128)
            .saturating_mul(BPS_DENOMINATOR.saturating_add(slack) as u128)
            / BPS_DENOMINATOR as u128;
        let cost_units = billable.saturating_add(self.base_cost_units as u128);

        // A zero denominator is a mis-entered rate, not a reason to abort a
        // completed execution; treat it as "no resource fee" and let the
        // inclusion fee stand.
        let resource = match self.resource_rate_den {
            0 => 0,
            den => cost_units.saturating_mul(self.resource_rate_num as u128) / den as u128,
        };

        let total = (self.inclusion_fee_lamports as u128).saturating_add(resource);
        u64::try_from(total)
            .unwrap_or(u64::MAX)
            .min(self.max_reimbursement_per_exec)
    }
}

impl ThreadConfig {
    /// What this execution cost the executor in transaction fees.
    pub fn transaction_fee(&self, remaining_at_start: u64, remaining_at_end: u64) -> u64 {
        self.fee_model
            .0
            .reimbursement(remaining_at_start, remaining_at_end)
    }

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

        let decay_end = match self
            .grace_period_seconds
            .checked_add(self.fee_decay_seconds)
        {
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
        transaction_fee: u64,
    ) -> PaymentDetails {
        // Calculate effective commission
        let effective_commission = self.calculate_effective_commission(time_since_ready);

        // Calculate reimbursement and commission for executor
        let (fee_payer_reimbursement, executor_commission) = if self.should_pay(balance_change) {
            let reimbursement = self.calculate_reimbursement(balance_change, transaction_fee);
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

#[cfg(test)]
mod fee_model_tests {
    use super::*;

    /// The rate the ramp reaches last, as a fraction.
    const TERMINAL: (u64, u64) = (1, 2);
    const BASELINE_UNITS: u64 = 23_463;

    fn at_rate(num: u64, den: u64) -> FeeModel {
        FeeModel {
            inclusion_fee_lamports: 2_500,
            resource_rate_num: num,
            resource_rate_den: den,
            ..FeeModel::preconfigured()
        }
    }

    /// The bytes actually on mainnet today.
    ///
    /// Everything about this migration rests on a config written before
    /// `fee_model` existed still deserializing. If it does not, the account
    /// cannot be read, which means it cannot be updated either, and the program
    /// is bricked with no way back. Asserted against a hand-built buffer that
    /// ends exactly where the old layout ended, rather than trusting that
    /// `Trailing` behaves the way its own tests say it does in a context they
    /// do not cover.
    #[test]
    fn a_config_written_before_the_fee_model_still_deserializes() {
        // The old layout, in field order, with no fee model after it.
        let mut old = Vec::new();
        1u64.serialize(&mut old).unwrap(); // version
        254u8.serialize(&mut old).unwrap(); // bump
        Pubkey::new_from_array([3u8; 32]).serialize(&mut old).unwrap(); // admin
        false.serialize(&mut old).unwrap(); // paused
        1000u64.serialize(&mut old).unwrap(); // commission_fee
        9000u64.serialize(&mut old).unwrap(); // executor_fee_bps
        1000u64.serialize(&mut old).unwrap(); // core_team_bps
        5i64.serialize(&mut old).unwrap(); // grace_period_seconds
        295i64.serialize(&mut old).unwrap(); // fee_decay_seconds

        let decoded = ThreadConfig::deserialize(&mut old.as_slice())
            .expect("a config predating the fee model must still decode");

        assert_eq!(decoded.commission_fee, 1000, "existing fields must survive");
        assert_eq!(decoded.bump, 254);
        assert!(
            !decoded.fee_model.0.is_configured(),
            "an absent fee model must read as unconfigured, not as a live one"
        );
        assert_eq!(
            decoded.transaction_fee(200_000, 176_537),
            crate::constants::TRANSACTION_BASE_FEE_REIMBURSEMENT,
            "and must therefore reimburse exactly what it did before"
        );
    }

    /// The account has to grow to hold the new field, which is what the
    /// `realloc` on `config_update` is for.
    #[test]
    fn the_fee_model_makes_the_account_bigger() {
        let config = ThreadConfig {
            version: 1,
            bump: 0,
            admin: Pubkey::default(),
            paused: false,
            commission_fee: 1000,
            executor_fee_bps: 9000,
            core_team_bps: 1000,
            grace_period_seconds: 5,
            fee_decay_seconds: 295,
            fee_model: Trailing(FeeModel::preconfigured()),
        };
        let mut written = Vec::new();
        config.serialize(&mut written).unwrap();

        assert!(
            8 + written.len() <= ThreadConfig::space(),
            "a fully populated config ({} bytes with discriminator) must fit the space \
             `config_update` reallocs to ({})",
            8 + written.len(),
            ThreadConfig::space()
        );
    }

    /// The migration case. A config written before this field existed reads
    /// back as all zeroes, and must reimburse exactly what it always did.
    #[test]
    fn an_unmigrated_config_pays_the_flat_reimbursement() {
        let unset = FeeModel::default();
        assert!(!unset.is_configured());
        for (start, end) in [(200_000, 176_537), (1_400_000, 0), (0, 0)] {
            assert_eq!(
                unset.reimbursement(start, end),
                crate::constants::TRANSACTION_BASE_FEE_REIMBURSEMENT
            );
        }
    }

    /// And migrating changes nothing until a rate is deliberately set.
    #[test]
    fn migrating_alone_does_not_move_any_money() {
        let migrated = FeeModel::preconfigured();
        assert!(migrated.is_configured());
        assert_eq!(
            migrated.reimbursement(200_000, 176_537),
            crate::constants::TRANSACTION_BASE_FEE_REIMBURSEMENT,
            "a migrated config with a zero rate must pay what an unmigrated one pays"
        );
    }

    /// The invariant the whole design rests on: nothing the executor chooses
    /// can raise what they are paid. Past the cap, more budget buys nothing.
    #[test]
    fn asking_for_a_bigger_budget_never_pays_more() {
        let model = at_rate(TERMINAL.0, TERMINAL.1);

        let mut highest = 0;
        for budget in [BASELINE_UNITS, 25_000, 30_000, 50_000, 200_000, 1_400_000] {
            highest = highest.max(model.reimbursement(budget, budget - BASELINE_UNITS));
        }

        assert_eq!(
            model.reimbursement(200_000, 200_000 - BASELINE_UNITS),
            model.reimbursement(1_400_000, 1_400_000 - BASELINE_UNITS),
            "past the cap, extra budget must be worth exactly nothing"
        );
        assert_eq!(
            highest,
            model.reimbursement(1_400_000, 1_400_000 - BASELINE_UNITS),
            "the payout must plateau at the cap rather than keep climbing"
        );
    }

    /// The point of billing measured overshoot rather than a fixed slack: an
    /// executor whose margin is within the cap gets back what they were charged.
    #[test]
    fn a_tight_executor_is_made_whole() {
        let model = at_rate(TERMINAL.0, TERMINAL.1);

        // A 5% margin, comfortably inside the 10% cap.
        let budget = BASELINE_UNITS * 105 / 100;
        let reimbursed = model.reimbursement(budget, budget - BASELINE_UNITS);

        // What SIMD-0553 actually charges.
        let charged = (budget
            + model.execution_overhead_units as u64
            + model.base_cost_units as u64)
            * TERMINAL.0
            / TERMINAL.1
            + model.inclusion_fee_lamports;

        let difference = (reimbursed as i64 - charged as i64).abs();
        assert!(
            difference < charged as i64 / 100,
            "a tight executor was reimbursed {} against {} charged; billing measured \
             overshoot exists so that these match",
            reimbursed,
            charged
        );
    }

    /// The other half of the incentive: overshoot past the cap comes out of the
    /// executor's own pocket, so there is a real reason to estimate tightly.
    #[test]
    fn a_sloppy_executor_eats_the_difference() {
        let model = at_rate(TERMINAL.0, TERMINAL.1);
        let sloppy = BASELINE_UNITS * 200 / 100;

        let reimbursed = model.reimbursement(sloppy, sloppy - BASELINE_UNITS);
        let charged = (sloppy
            + model.execution_overhead_units as u64
            + model.base_cost_units as u64)
            * TERMINAL.0
            / TERMINAL.1
            + model.inclusion_fee_lamports;

        assert!(
            reimbursed < charged,
            "a 100% margin was fully reimbursed ({} of {}); nothing then discourages \
             requesting the ceiling",
            reimbursed,
            charged
        );
    }

    /// The measurement is a delta, so it scales with the fiber's own cost —
    /// which is the whole reason a constant does not work.
    #[test]
    fn an_expensive_fiber_is_reimbursed_more_than_a_cheap_one() {
        let model = at_rate(TERMINAL.0, TERMINAL.1);
        let cheap = model.reimbursement(1_400_000, 1_400_000 - BASELINE_UNITS);
        let expensive = model.reimbursement(1_400_000, 1_400_000 - BASELINE_UNITS * 10);
        assert!(expensive > cheap, "{} was not more than {}", expensive, cheap);
    }

    /// The backstop for a mis-entered rate.
    #[test]
    fn the_ceiling_bounds_a_mis_entered_rate() {
        let model = FeeModel {
            resource_rate_num: 5,
            resource_rate_den: 1,
            max_reimbursement_per_exec: 50_000,
            ..FeeModel::preconfigured()
        };
        assert_eq!(model.reimbursement(1_400_000, 0), 50_000);
    }

    /// Readings only decrease. Arguments the wrong way round mean a bug at the
    /// call site, and the execution has already run — pay rather than abort.
    #[test]
    fn reversed_readings_do_not_abort_the_execution() {
        let model = at_rate(TERMINAL.0, TERMINAL.1);
        assert!(model.reimbursement(100, 200_000) > 0);
    }

    /// A zero denominator is a typo, not grounds for failing an execution whose
    /// side effects are already committed.
    #[test]
    fn a_zero_denominator_falls_back_to_the_inclusion_fee() {
        let model = FeeModel {
            resource_rate_num: 1,
            resource_rate_den: 0,
            inclusion_fee_lamports: 2_500,
            ..FeeModel::preconfigured()
        };
        assert_eq!(model.reimbursement(200_000, 100_000), 2_500);
    }

    /// `tests/exec_cost.rs` measures antegen's overhead at 23_463 units and
    /// projects 17_983 lamports for it at the terminal rate. The on-chain
    /// formula should land near that, or the two models have drifted apart and
    /// one of them is wrong.
    #[test]
    fn the_formula_agrees_with_the_measured_profile() {
        let model = at_rate(TERMINAL.0, TERMINAL.1);
        let measured = BASELINE_UNITS - model.execution_overhead_units as u64;
        let reimbursement = model.reimbursement(1_400_000, 1_400_000 - measured);

        let projected = 17_983i64;
        let difference = (reimbursement as i64 - projected).abs();
        assert!(
            difference < projected / 10,
            "on-chain formula produced {} where the off-chain projection says {}. These \
             describe the same transaction and should agree within rounding.",
            reimbursement,
            projected
        );
    }
}
