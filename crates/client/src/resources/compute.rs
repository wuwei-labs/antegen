//! Per-thread compute unit budgeting.
//!
//! A transaction is charged on the compute limit it *requests*, not the units
//! it consumes. Today that costs nothing — the base fee is a flat 5,000
//! lamports per signature whatever the request — so the executor has always
//! padded generously: `estimate * 1.25 + 10_000`. Over-reserving was free and
//! `ComputationalBudgetExceeded` costs the whole trigger window plus a retry,
//! so the trade was one-sided.
//!
//! SIMD-0553 ends that. The burned half of the fee becomes a resource fee
//! proportional to `requested_cost_units`, of which the requested compute limit
//! is ~90%. At the terminal rate of 0.5 lamports per unit, a thread that
//! consumes 200k units but requests 250k burns 25,000 lamports per execution
//! for units it never used — five times today's entire fee, as pad.
//!
//! The pad cannot simply be cut: the simulation it scales runs against the
//! `processed` bank at a different clock and possibly different account state,
//! so some headroom is genuinely required, and the amount differs per thread.
//! A fiber that reads a growing account needs more; a fixed transfer needs
//! almost none.
//!
//! So the margin is learned per thread rather than assumed. It starts at
//! exactly today's 25% — the new code cannot behave worse than the old on its
//! first execution — decays slowly while executions land, and jumps back up the
//! moment one exceeds its budget. Additive decrease, multiplicative increase:
//! the direction that costs money is approached in small steps and the
//! direction that costs a trigger window is retreated from in large ones.

use dashmap::DashMap;
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use std::time::{Duration, Instant};

/// Solana's per-transaction compute ceiling.
pub const MAX_COMPUTE_UNITS: u32 = 1_400_000;

/// Tuning for the margin controller. All values are basis points of the
/// simulated estimate except `floor_units`, which is absolute.
#[derive(Debug, Clone, Copy)]
pub struct CuOracleConfig {
    /// Margin a thread starts at, before any execution has been observed.
    ///
    /// Defaults to 2,500 bps — the 25% the executor used unconditionally — so
    /// a cold node requests exactly what it used to.
    pub initial_margin_bps: u32,
    /// Floor the margin decays toward. Not zero: simulation and execution run
    /// against different bank states, and some drift is structural rather than
    /// learnable.
    pub min_margin_bps: u32,
    /// Ceiling the margin climbs to after repeated overruns.
    pub max_margin_bps: u32,
    /// How much a landed execution removes from the margin.
    pub decrease_step_bps: u32,
    /// Added after doubling when an execution exceeds its budget, so a thread
    /// sitting at the floor still makes a meaningful jump.
    pub increase_step_bps: u32,
    /// Absolute headroom added on top of the proportional margin.
    ///
    /// A percentage of a small estimate is a small number, and the fixed costs
    /// that differ between simulation and execution do not scale with the
    /// fiber's size.
    pub floor_units: u32,
    /// Added to a *measured* margin requirement before it is adopted.
    ///
    /// An observation says what one execution needed, not what the next one
    /// will. Tracking the measurement exactly would leave every thread one
    /// unlucky slot away from an overrun.
    pub observation_safety_bps: u32,
    /// Weight given to a new observation, in basis points, when folding it into
    /// the running margin. 2,500 means each measurement moves the margin a
    /// quarter of the way to its target.
    pub observation_weight_bps: u32,
}

impl Default for CuOracleConfig {
    fn default() -> Self {
        Self {
            initial_margin_bps: 2_500,
            min_margin_bps: 300,
            max_margin_bps: 10_000,
            decrease_step_bps: 25,
            increase_step_bps: 500,
            floor_units: 3_000,
            observation_safety_bps: 500,
            observation_weight_bps: 2_500,
        }
    }
}

/// How long an unobserved in-flight execution is kept before being swept.
///
/// Log delivery is best-effort: a dropped subscription or a missed
/// notification leaves an entry with nothing to resolve it.
const PENDING_TTL: Duration = Duration::from_secs(120);

/// An execution awaiting its logs.
#[derive(Debug, Clone, Copy)]
struct Pending {
    thread: Pubkey,
    simulated_units: u64,
    at: Instant,
}

/// Learns how much compute headroom each thread actually needs.
#[derive(Debug)]
pub struct CuOracle {
    config: CuOracleConfig,
    /// Current margin per thread, in basis points.
    ///
    /// Grows with the number of distinct threads this process has executed,
    /// which is bounded by what the node subscribes to — a `u32` per thread,
    /// not worth evicting.
    margins: DashMap<Pubkey, u32>,
    /// Submitted executions whose logs have not arrived yet, keyed by the
    /// signature the logs will identify them by.
    pending: DashMap<Signature, Pending>,
}

impl CuOracle {
    pub fn new(config: CuOracleConfig) -> Self {
        Self {
            config,
            margins: DashMap::new(),
            pending: DashMap::new(),
        }
    }

    /// The compute unit limit to request for this thread.
    pub fn limit(&self, thread: &Pubkey, simulated_units: u64) -> u32 {
        let margin_bps = self.margin_bps(thread);
        let scaled = simulated_units.saturating_mul(10_000 + margin_bps as u64) / 10_000;
        let padded = scaled.saturating_add(self.config.floor_units as u64);
        padded.min(MAX_COMPUTE_UNITS as u64) as u32
    }

    /// Current margin for a thread, in basis points.
    pub fn margin_bps(&self, thread: &Pubkey) -> u32 {
        self.margins
            .get(thread)
            .map(|m| *m)
            .unwrap_or(self.config.initial_margin_bps)
    }

    /// An execution landed within its budget: the margin can afford to shrink.
    pub fn record_landed(&self, thread: &Pubkey) {
        self.adjust(thread, |current, config| {
            current
                .saturating_sub(config.decrease_step_bps)
                .max(config.min_margin_bps)
        });
    }

    /// An execution ran out of compute: retreat, hard.
    ///
    /// Doubling rather than stepping because the cost of being wrong in this
    /// direction is a missed trigger window plus a retry, and one overrun says
    /// the current margin is wrong by an unknown amount rather than by one step.
    pub fn record_exceeded(&self, thread: &Pubkey) {
        self.adjust(thread, |current, config| {
            current
                .saturating_mul(2)
                .saturating_add(config.increase_step_bps)
                .min(config.max_margin_bps)
        });
    }

    /// Note a submitted execution so its logs can be attributed when they
    /// arrive.
    pub fn register(&self, signature: Signature, thread: Pubkey, simulated_units: u64) {
        self.sweep_expired();
        self.pending.insert(
            signature,
            Pending {
                thread,
                simulated_units,
                at: Instant::now(),
            },
        );
    }

    /// Attribute observed usage to whichever execution produced it.
    ///
    /// Returns the thread it belonged to, or `None` for a signature this node
    /// did not submit — the log subscription is filtered by executor address,
    /// not by our own sends, so it also carries transactions in which the
    /// executor merely appears.
    pub fn observe(&self, signature: &Signature, usage: ComputeUsage) -> Option<Pubkey> {
        let (_, pending) = self.pending.remove(signature)?;
        self.record_observed(&pending.thread, pending.simulated_units, usage);
        Some(pending.thread)
    }

    /// Fold a measurement into a thread's margin.
    ///
    /// This is what the additive-decrease path is a substitute for. Where
    /// `record_landed` only learns that the budget was *sufficient* — and so
    /// has to walk downward until it overshoots and costs a trigger window —
    /// a measurement says how much was actually needed, in one execution.
    pub fn record_observed(&self, thread: &Pubkey, simulated_units: u64, usage: ComputeUsage) {
        if simulated_units == 0 {
            return;
        }

        // How far execution ran past the simulation, as a fraction of it.
        // Frequently negative — a simulation against the `processed` bank often
        // consumes *more* than execution does — in which case the requirement
        // is zero and only the safety band and the floors keep the margin up.
        let overshoot_bps = usage
            .consumed
            .saturating_mul(10_000)
            .checked_div(simulated_units)
            .unwrap_or(10_000)
            .saturating_sub(10_000)
            .min(u32::MAX as u64) as u32;

        let target = overshoot_bps.saturating_add(self.config.observation_safety_bps);
        let weight = self.config.observation_weight_bps.min(10_000) as u64;

        self.adjust(thread, |current, _| {
            let blended = (current as u64 * (10_000 - weight) + target as u64 * weight) / 10_000;
            blended.min(u32::MAX as u64) as u32
        });
    }

    pub fn pending_observations(&self) -> usize {
        self.pending.len()
    }

    /// Drop in-flight entries whose logs never arrived.
    fn sweep_expired(&self) {
        if self.pending.len() < 512 {
            return;
        }
        self.pending
            .retain(|_, pending| pending.at.elapsed() < PENDING_TTL);
    }

    /// Forget a thread's learned margin — it no longer exists on chain, and a
    /// future thread at the same address is a different workload.
    pub fn forget(&self, thread: &Pubkey) {
        self.margins.remove(thread);
    }

    pub fn tracked_threads(&self) -> usize {
        self.margins.len()
    }

    fn adjust(&self, thread: &Pubkey, f: impl Fn(u32, &CuOracleConfig) -> u32) {
        let current = self.margin_bps(thread);
        let next =
            f(current, &self.config).clamp(self.config.min_margin_bps, self.config.max_margin_bps);
        self.margins.insert(*thread, next);
    }
}

impl Default for CuOracle {
    fn default() -> Self {
        Self::new(CuOracleConfig::default())
    }
}

/// Granularity the cost model charges loaded account data in: 8 cost units per
/// 32 KiB page, or part thereof.
pub const LOADED_ACCOUNTS_PAGE_BYTES: u32 = 32 * 1024;

/// The runtime's default when a transaction requests no limit: 64 MiB, which is
/// 2,048 pages and so 16,384 cost units — more, at the terminal resource-fee
/// rate, than today's entire transaction fee, charged on every transaction that
/// stays silent about what it actually loads.
pub const MAX_LOADED_ACCOUNTS_DATA_SIZE_BYTES: u32 = 64 * 1024 * 1024;

/// Turn a measured loaded-accounts size into the limit to request.
///
/// Rounds up to the page the cost model charges in and adds `slack_pages`.
/// Slack is nearly free — a page is 8 cost units, so two pages of headroom cost
/// 16 against the 16,384 that requesting nothing costs — while being short is
/// a failed transaction and a missed trigger window. There is no reason to cut
/// this fine.
pub fn loaded_accounts_limit(measured_bytes: u32, slack_pages: u32) -> u32 {
    let pages = measured_bytes.div_ceil(LOADED_ACCOUNTS_PAGE_BYTES);
    pages
        .saturating_add(slack_pages)
        .saturating_mul(LOADED_ACCOUNTS_PAGE_BYTES)
        .min(MAX_LOADED_ACCOUNTS_DATA_SIZE_BYTES)
}

/// What a landed transaction actually used, read from its logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComputeUsage {
    /// Units consumed by top-level program instructions.
    pub consumed: u64,
    /// Budget remaining when the first top-level program was entered.
    ///
    /// Not quite the requested limit — the ComputeBudget instructions
    /// themselves have already been charged by then — but within a couple of
    /// hundred units of it, and it is what the logs actually state.
    pub budget: u64,
}

/// Extract compute usage from a transaction's log lines.
///
/// The relevant lines look like:
///
/// ```text
/// Program <id> invoke [1]
/// Program <inner> invoke [2]
/// Program <inner> consumed 1234 of 190000 compute units
/// Program <inner> success
/// Program <id> consumed 45678 of 199850 compute units
/// Program <id> success
/// ```
///
/// Only depth-1 frames are summed. A CPI's consumption is already included in
/// its caller's total, so counting both double-counts every inner instruction —
/// which for a thread executing a fiber is most of the transaction.
///
/// Depth is tracked from the `invoke [n]` lines, and a `consumed` line closes
/// the frame it belongs to. Native programs such as ComputeBudget log an invoke
/// and a success but no consumed line; since every invoke line restates its own
/// depth, that leaves nothing to correct.
pub fn parse_compute_usage(logs: &[String]) -> Option<ComputeUsage> {
    let mut depth = 0usize;
    let mut consumed = 0u64;
    let mut budget = None;

    for line in logs {
        let line = line.trim();
        if !line.starts_with("Program ") {
            continue;
        }

        if let Some(rest) = line.rsplit_once(" invoke [") {
            if let Some(n) = rest.1.strip_suffix(']').and_then(|d| d.parse().ok()) {
                depth = n;
                continue;
            }
        }

        // "Program <id> consumed <n> of <m> compute units"
        if let Some(idx) = line.find(" consumed ") {
            let tail = &line[idx + " consumed ".len()..];
            let Some(tail) = tail.strip_suffix(" compute units") else {
                continue;
            };
            let Some((used, total)) = tail.split_once(" of ") else {
                continue;
            };
            let (Ok(used), Ok(total)) = (used.parse::<u64>(), total.parse::<u64>()) else {
                continue;
            };

            if depth == 1 {
                consumed = consumed.saturating_add(used);
                budget.get_or_insert(total);
            }
            depth = depth.saturating_sub(1);
        }
    }

    budget.map(|budget| ComputeUsage { consumed, budget })
}

/// Whether a failure was the transaction running out of compute.
///
/// Matched on both the variant name and its rendered text: the string reaching
/// the executor comes from an RPC error whose formatting is not ours to
/// guarantee, and missing an overrun means the margin never learns from the one
/// event it exists to react to.
pub fn is_compute_exceeded(error: &str) -> bool {
    error.contains("ComputationalBudgetExceeded")
        || error.contains("Computational budget exceeded")
        || error.contains("exceeded CUs")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oracle() -> CuOracle {
        CuOracle::default()
    }

    /// The behaviour this replaces, so a cold node is never worse off than the
    /// unconditional pad it used to apply.
    ///
    /// The old rule was `estimate * 1.25 + 10_000`. The margin matches at 25%;
    /// the absolute floor is deliberately lower, because 10,000 units is 5,000
    /// lamports at the terminal resource-fee rate — today's entire transaction
    /// fee, spent on headroom, on every execution.
    #[test]
    fn first_execution_requests_the_historical_margin() {
        let thread = Pubkey::new_unique();
        assert_eq!(oracle().margin_bps(&thread), 2_500);
        assert_eq!(oracle().limit(&thread, 200_000), 250_000 + 3_000);
    }

    #[test]
    fn landing_shrinks_the_margin_toward_the_floor() {
        let o = oracle();
        let thread = Pubkey::new_unique();

        o.record_landed(&thread);
        assert_eq!(o.margin_bps(&thread), 2_475);

        for _ in 0..1_000 {
            o.record_landed(&thread);
        }
        assert_eq!(
            o.margin_bps(&thread),
            300,
            "decay stops at the floor rather than reaching zero headroom"
        );
    }

    #[test]
    fn an_overrun_retreats_faster_than_success_advances() {
        let o = oracle();
        let thread = Pubkey::new_unique();

        for _ in 0..1_000 {
            o.record_landed(&thread);
        }
        assert_eq!(o.margin_bps(&thread), 300);

        o.record_exceeded(&thread);
        assert_eq!(o.margin_bps(&thread), 1_100, "300 * 2 + 500");

        // One overrun undoes many successes — the asymmetry is the point.
        let recovered = o.margin_bps(&thread);
        o.record_landed(&thread);
        assert_eq!(o.margin_bps(&thread), recovered - 25);
    }

    #[test]
    fn repeated_overruns_stop_at_the_ceiling() {
        let o = oracle();
        let thread = Pubkey::new_unique();

        for _ in 0..20 {
            o.record_exceeded(&thread);
        }
        assert_eq!(o.margin_bps(&thread), 10_000);
    }

    /// The saving the whole module exists for, in units.
    #[test]
    fn a_settled_thread_requests_far_less_than_a_cold_one() {
        let o = oracle();
        let thread = Pubkey::new_unique();

        let cold = o.limit(&thread, 200_000);
        for _ in 0..1_000 {
            o.record_landed(&thread);
        }
        let settled = o.limit(&thread, 200_000);

        assert_eq!(cold, 253_000);
        assert_eq!(settled, 209_000);
        assert!(cold - settled == 44_000);
    }

    #[test]
    fn the_request_never_exceeds_the_transaction_ceiling() {
        let o = oracle();
        let thread = Pubkey::new_unique();
        assert_eq!(o.limit(&thread, 1_400_000), MAX_COMPUTE_UNITS);
        assert_eq!(o.limit(&thread, u64::MAX), MAX_COMPUTE_UNITS);
    }

    #[test]
    fn margins_are_per_thread() {
        let o = oracle();
        let (a, b) = (Pubkey::new_unique(), Pubkey::new_unique());

        o.record_exceeded(&a);
        assert_eq!(o.margin_bps(&a), 5_500);
        assert_eq!(o.margin_bps(&b), 2_500, "b is untouched by a's overrun");
    }

    #[test]
    fn forgetting_a_thread_returns_it_to_the_default() {
        let o = oracle();
        let thread = Pubkey::new_unique();

        o.record_exceeded(&thread);
        assert_ne!(o.margin_bps(&thread), 2_500);

        o.forget(&thread);
        assert_eq!(o.margin_bps(&thread), 2_500);
        assert_eq!(o.tracked_threads(), 0);
    }

    /// Cost units the runtime charges for a given limit: 8 per 32 KiB page.
    fn cost_units(limit_bytes: u32) -> u32 {
        limit_bytes.div_ceil(LOADED_ACCOUNTS_PAGE_BYTES) * 8
    }

    /// The saving this exists for. Requesting nothing is charged the 64 MiB
    /// default; a realistic thread loads a few hundred KiB.
    #[test]
    fn requesting_a_measured_limit_beats_the_runtime_default() {
        // ~400 KiB of programs and accounts.
        let limit = loaded_accounts_limit(400 * 1024, 2);

        assert_eq!(cost_units(MAX_LOADED_ACCOUNTS_DATA_SIZE_BYTES), 16_384);
        assert_eq!(cost_units(limit), 120);
    }

    #[test]
    fn a_measurement_is_rounded_up_to_a_whole_page() {
        // One byte into a second page still occupies two, plus slack.
        assert_eq!(
            loaded_accounts_limit(LOADED_ACCOUNTS_PAGE_BYTES + 1, 0),
            2 * LOADED_ACCOUNTS_PAGE_BYTES
        );
        assert_eq!(
            loaded_accounts_limit(LOADED_ACCOUNTS_PAGE_BYTES, 0),
            LOADED_ACCOUNTS_PAGE_BYTES,
            "an exact page is not rounded to the next one"
        );
    }

    #[test]
    fn slack_is_added_in_whole_pages() {
        let bare = loaded_accounts_limit(100 * 1024, 0);
        let padded = loaded_accounts_limit(100 * 1024, 2);
        assert_eq!(padded - bare, 2 * LOADED_ACCOUNTS_PAGE_BYTES);
        assert_eq!(
            cost_units(padded) - cost_units(bare),
            16,
            "two pages of headroom against the 16,384 that requesting nothing costs"
        );
    }

    /// A measurement near the ceiling must not produce a limit above it, which
    /// the runtime would reject.
    #[test]
    fn the_limit_never_exceeds_what_the_runtime_permits() {
        assert_eq!(
            loaded_accounts_limit(MAX_LOADED_ACCOUNTS_DATA_SIZE_BYTES, 2),
            MAX_LOADED_ACCOUNTS_DATA_SIZE_BYTES
        );
        assert_eq!(
            loaded_accounts_limit(u32::MAX, 100),
            MAX_LOADED_ACCOUNTS_DATA_SIZE_BYTES
        );
    }

    fn logs(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    const THREAD_PROGRAM: &str = "AgThr1111111111111111111111111111111111111";
    const CPI_TARGET: &str = "Tar9et1111111111111111111111111111111111111";

    /// A CPI's consumption is already inside its caller's total. Summing both
    /// would roughly double the measurement for every thread that executes a
    /// fiber — which is all of them.
    #[test]
    fn nested_invocations_are_not_counted_twice() {
        let usage = parse_compute_usage(&logs(&[
            "Program ComputeBudget111111111111111111111111111111 invoke [1]",
            "Program ComputeBudget111111111111111111111111111111 success",
            &format!("Program {THREAD_PROGRAM} invoke [1]"),
            "Program log: Executed 0s after trigger",
            &format!("Program {CPI_TARGET} invoke [2]"),
            &format!("Program {CPI_TARGET} consumed 1234 of 190000 compute units"),
            &format!("Program {CPI_TARGET} success"),
            &format!("Program {THREAD_PROGRAM} consumed 45678 of 199850 compute units"),
            &format!("Program {THREAD_PROGRAM} success"),
        ]))
        .expect("depth-1 frame present");

        assert_eq!(
            usage.consumed, 45_678,
            "the inner 1,234 is already included"
        );
        assert_eq!(usage.budget, 199_850);
    }

    /// A batched thread execution runs several top-level instructions; each is
    /// its own charge.
    #[test]
    fn sibling_top_level_instructions_are_summed() {
        let usage = parse_compute_usage(&logs(&[
            &format!("Program {THREAD_PROGRAM} invoke [1]"),
            &format!("Program {THREAD_PROGRAM} consumed 30000 of 400000 compute units"),
            &format!("Program {THREAD_PROGRAM} success"),
            &format!("Program {THREAD_PROGRAM} invoke [1]"),
            &format!("Program {THREAD_PROGRAM} consumed 25000 of 370000 compute units"),
            &format!("Program {THREAD_PROGRAM} success"),
        ]))
        .unwrap();

        assert_eq!(usage.consumed, 55_000);
        assert_eq!(usage.budget, 400_000, "budget is read at first entry");
    }

    #[test]
    fn logs_without_a_consumed_line_yield_nothing() {
        assert!(parse_compute_usage(&logs(&[
            "Program ComputeBudget111111111111111111111111111111 invoke [1]",
            "Program ComputeBudget111111111111111111111111111111 success",
        ]))
        .is_none());
        assert!(parse_compute_usage(&[]).is_none());
    }

    /// A measurement reaches a sane margin immediately, where the blind path
    /// needs roughly ninety landings and one overrun to get there.
    #[test]
    fn one_measurement_beats_ninety_blind_successes() {
        let o = oracle();
        let thread = Pubkey::new_unique();

        // Execution used 6% more than the simulation predicted.
        let usage = ComputeUsage {
            consumed: 212_000,
            budget: 250_000,
        };

        // A single measurement moves the margin 350 bps toward its target. A
        // single landing on the blind path moves it 25.
        o.record_observed(&thread, 200_000, usage);
        assert_eq!(o.margin_bps(&thread), 2_150);

        let blind = oracle();
        blind.record_landed(&thread);
        assert_eq!(blind.margin_bps(&thread), 2_475);

        for _ in 0..40 {
            o.record_observed(&thread, 200_000, usage);
        }

        // 600 bps overshoot + 500 bps safety.
        assert_eq!(o.margin_bps(&thread), 1_100);
    }

    /// Simulation routinely consumes more than execution. That is not a reason
    /// to request less than the floor.
    #[test]
    fn an_execution_cheaper_than_its_simulation_still_keeps_headroom() {
        let o = oracle();
        let thread = Pubkey::new_unique();

        let usage = ComputeUsage {
            consumed: 150_000,
            budget: 250_000,
        };
        for _ in 0..50 {
            o.record_observed(&thread, 200_000, usage);
        }

        assert_eq!(
            o.margin_bps(&thread),
            500,
            "no overshoot, so the safety band alone sets the margin"
        );
    }

    #[test]
    fn observations_are_attributed_by_signature() {
        let o = oracle();
        let thread = Pubkey::new_unique();
        let sig = Signature::new_unique();
        let usage = ComputeUsage {
            consumed: 212_000,
            budget: 250_000,
        };

        o.register(sig, thread, 200_000);
        assert_eq!(o.pending_observations(), 1);

        assert_eq!(o.observe(&sig, usage), Some(thread));
        assert_eq!(o.pending_observations(), 0, "resolved entries are dropped");

        // The subscription is filtered by executor address, so it also carries
        // transactions this node never sent.
        assert_eq!(o.observe(&Signature::new_unique(), usage), None);
    }

    #[test]
    fn compute_overruns_are_recognised_in_either_rendering() {
        assert!(is_compute_exceeded(
            "InstructionError(0, ComputationalBudgetExceeded)"
        ));
        assert!(is_compute_exceeded(
            "Transaction simulation failed: Computational budget exceeded"
        ));
        assert!(!is_compute_exceeded("BlockhashNotFound"));
        assert!(!is_compute_exceeded("WouldExceedMaxBlockCostLimit"));
    }
}
