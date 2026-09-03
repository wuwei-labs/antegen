//! What a thread execution costs, in compute units and in lamports.
//!
//! Two questions live here, and they have very different confidence levels.
//!
//! **What does `thread_exec` consume?** Measured, exactly, by running the real
//! SBF bytecode under LiteSVM and reading `compute_units_consumed`. This is the
//! number nothing else in the repo knows, and every fee decision depends on it:
//! the reimbursement formula, the commission repricing, and the CU oracle's
//! floor and margin.
//!
//! Read the shape table for what it is. Every shape wraps a memo, so what it
//! measures is antegen's *overhead* — the trigger check, the schedule update,
//! the CPI, the payments — and not what an execution costs. Real fibers call
//! swaps and oracle refreshes, compute is additive across the CPI, and the
//! inner instruction is usually the larger half. The overhead is the part
//! antegen can do something about, which is why it is isolated; it is not the
//! bill. `report_inner_instruction_cost` puts the two together.
//!
//! **What will that cost under SIMD-0553?** Projected, from a rate model that
//! is written down once in [`model`] below. The SIMD is passed but not
//! activated, and its constants are transcribed here rather than imported —
//! there is no published crate to import them from. Treat the measurement as
//! fact and the projection as arithmetic on top of assumptions that are easy to
//! correct in one place.
//!
//! The distinction matters because the projection is what makes the case for
//! changing the program, and a reviewer needs to be able to see which half they
//! are being asked to trust.
//!
//! Run the report with:
//!
//! ```text
//! cargo test -p antegen-thread-program --test exec_cost -- --nocapture
//! ```
//!
//! The assertions run either way; the tables only print under `--nocapture`, so
//! this stays quiet in CI while remaining the thing you reach for when the
//! numbers need revisiting.

use solana_sdk::{
    instruction::AccountMeta,
    signature::{Keypair, Signer},
    transaction::Transaction,
};

mod common;
use common::*;

// ─────────────────────────────────────────────────────────────────────────────
// The cost model
// ─────────────────────────────────────────────────────────────────────────────

/// SIMD-0553's fee arithmetic, transcribed.
///
/// Every constant that is not measured lives here, so correcting the model when
/// the implementation lands is a single edit rather than a hunt.
mod model {
    /// What a signature contributes to `requested_cost_units`.
    pub const SIGNATURE_COST_UNITS: u64 = 720;

    /// What each writable account contributes.
    pub const WRITE_LOCK_COST_UNITS: u64 = 300;

    /// Loaded-accounts data is charged per 32 KiB page.
    pub const LOADED_ACCOUNTS_PAGE_BYTES: u64 = 32 * 1024;
    pub const LOADED_ACCOUNTS_COST_PER_PAGE: u64 = 8;

    /// A transaction that never sets `SetLoadedAccountsDataSizeLimit` is charged
    /// as though it might load the 64 MiB maximum.
    pub const DEFAULT_LOADED_ACCOUNTS_BYTES: u64 = 64 * 1024 * 1024;

    /// The leader's take, per signature. Unchanged by SIMD-0553 — today's 5000
    /// splits 50/50 burn/leader, and the SIMD keeps the leader's 2500 while
    /// replacing the burned half with the resource fee.
    pub const INCLUSION_FEE_PER_SIGNATURE: u64 = 2_500;

    /// What a signature costs today, before any of this.
    pub const LEGACY_FEE_PER_SIGNATURE: u64 = 5_000;

    /// The three feature-gated steps of the resource rate, in lamports per cost
    /// unit. Expressed as a fraction to keep the arithmetic integral — the
    /// on-chain formula will have to do the same.
    pub const RAMP: [(&str, u64, u64); 3] = [("0.1", 1, 10), ("0.25", 1, 4), ("0.5", 1, 2)];

    /// Instruction data bytes contribute to the cost. Whether the divisor is 1
    /// (as the SIMD reads) or 140 (as agave's existing cost model computes
    /// `data_bytes_cost`) is the one open question in this model, and it is
    /// immaterial: a thread exec carries ~100 bytes of instruction data against
    /// a six-figure execution cost, so the two readings differ by well under
    /// 0.1% of the total. Left at 1 as the conservative choice.
    pub const INSTRUCTION_DATA_BYTES_PER_UNIT: u64 = 1;

    /// Pages needed to hold `bytes`, rounded up.
    pub fn loaded_accounts_units(bytes: u64) -> u64 {
        bytes
            .div_ceil(LOADED_ACCOUNTS_PAGE_BYTES)
            .saturating_mul(LOADED_ACCOUNTS_COST_PER_PAGE)
    }
}

/// One `thread_exec` transaction, measured.
#[derive(Debug, Clone)]
struct Measurement {
    name: String,
    /// Compute units the SBF VM actually burned. The measured quantity.
    consumed_units: u64,
    signatures: u64,
    writable_accounts: u64,
    instruction_data_bytes: u64,
    /// Total data held by every account the transaction touches — the input to
    /// the client's `loaded_accounts_limit()`.
    loaded_accounts_bytes: u64,
}

/// How the transaction asks for compute. This is a *policy* choice, not a
/// measurement, and under SIMD-0553 it is charged whether or not it is used —
/// the execution term comes from the requested limit, not the consumed one.
#[derive(Debug, Clone, Copy)]
enum Request {
    /// What a transaction gets for free: 200k per instruction, never declared.
    /// Also what every antegen exec asked for before the CU oracle.
    Default200k,
    /// Consumed plus a margin, which is what the oracle produces.
    Measured { margin_bps: u64 },
}

impl Request {
    fn label(&self) -> String {
        match self {
            Request::Default200k => "200k default".to_string(),
            Request::Measured { margin_bps } => format!("measured +{}%", margin_bps / 100),
        }
    }

    fn units(&self, consumed: u64) -> u64 {
        match self {
            Request::Default200k => 200_000,
            Request::Measured { margin_bps } => consumed
                .saturating_mul(10_000 + margin_bps)
                .div_ceil(10_000),
        }
    }
}

impl Measurement {
    /// The same transaction with a more expensive instruction inside the fiber.
    ///
    /// Compute is additive across a CPI — the callee spends from the caller's
    /// budget — so a thread wrapping a 500k-unit swap costs antegen's overhead
    /// plus 500k. Asserted in `the_inner_instruction_adds_to_the_total` rather
    /// than taken on faith, because the entire fee projection for real threads
    /// rests on it.
    fn with_inner_instruction(&self, inner_units: u64) -> Measurement {
        Measurement {
            name: format!("{} + {}k inner", self.name, inner_units / 1_000),
            consumed_units: self.consumed_units.saturating_add(inner_units),
            ..self.clone()
        }
    }

    /// `requested_cost_units` — what the resource fee is charged against.
    fn requested_cost_units(&self, request: Request, declare_loaded_accounts: bool) -> u64 {
        let loaded = if declare_loaded_accounts {
            self.loaded_accounts_bytes
        } else {
            model::DEFAULT_LOADED_ACCOUNTS_BYTES
        };

        (self.signatures.saturating_mul(model::SIGNATURE_COST_UNITS))
            .saturating_add(
                self.writable_accounts
                    .saturating_mul(model::WRITE_LOCK_COST_UNITS),
            )
            .saturating_add(
                self.instruction_data_bytes / model::INSTRUCTION_DATA_BYTES_PER_UNIT,
            )
            .saturating_add(request.units(self.consumed_units))
            .saturating_add(model::loaded_accounts_units(loaded))
    }

    /// Total fee under SIMD-0553 at a given resource rate.
    fn projected_fee(
        &self,
        request: Request,
        declare_loaded_accounts: bool,
        rate_num: u64,
        rate_den: u64,
    ) -> u64 {
        let resource = self
            .requested_cost_units(request, declare_loaded_accounts)
            .saturating_mul(rate_num)
            .checked_div(rate_den)
            .unwrap_or(0);
        self.signatures
            .saturating_mul(model::INCLUSION_FEE_PER_SIGNATURE)
            .saturating_add(resource)
    }

    /// What this transaction costs today.
    fn current_fee(&self) -> u64 {
        self.signatures
            .saturating_mul(model::LEGACY_FEE_PER_SIGNATURE)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Building and measuring shapes
// ─────────────────────────────────────────────────────────────────────────────

/// A thread worth measuring.
struct Shape {
    name: &'static str,
    trigger: Trigger,
    /// Each fiber's memo payload and the signal it returns.
    fibers: Vec<(String, Option<Signal>)>,
    /// Seconds to advance the clock before executing, for triggers that need to
    /// come due.
    warp_seconds: i64,
    forgo_commission: bool,
}

impl Shape {
    fn new(name: &'static str, trigger: Trigger) -> Self {
        Self {
            name,
            trigger,
            fibers: vec![("memo".to_string(), None)],
            warp_seconds: 0,
            forgo_commission: false,
        }
    }

    fn fibers(mut self, fibers: Vec<(String, Option<Signal>)>) -> Self {
        self.fibers = fibers;
        self
    }

    fn warp(mut self, seconds: i64) -> Self {
        self.warp_seconds = seconds;
        self
    }

    fn forgo_commission(mut self) -> Self {
        self.forgo_commission = true;
        self
    }
}

/// The wall-clock instant every measurement runs at.
///
/// Any fixed value would do; this one is a Friday, so the weekday-restricted
/// cron shape reaches its next occurrence within the window the profile warps
/// through rather than sitting behind a weekend.
const FIXED_CLOCK: i64 = 1_800_000_000;

/// The authority every measured thread belongs to, and the id every measured
/// thread carries.
///
/// Both are fixed, and both have to be, for the same reason: they are seeds of
/// the thread PDA, and deriving a PDA costs 1500 units per bump the search has
/// to step over. A random authority — or an id that varies by shape — makes that
/// search a different length each time, and the measurement moves by whole
/// multiples of 1500 for reasons that have nothing to do with the code being
/// measured. Two consecutive runs of an early draft disagreed by 4500 units on
/// the same cron shape.
///
/// Holding both fixed gives every shape the identical set of addresses, so what
/// is left in the differences is the trigger and signal logic, which is the
/// thing worth knowing. Shapes never collide despite sharing an id because each
/// gets its own `LiteSVM`.
const PROFILE_THREAD_ID: &[u8] = b"profile";

fn profile_authority() -> Keypair {
    solana_sdk::signer::keypair::keypair_from_seed(&[7u8; 32]).expect("32 bytes is a valid seed")
}

/// What the fiber's target instruction costs on its own, run as an ordinary
/// top-level instruction with no thread around it.
///
/// The profile's shapes all wrap a memo, which is close to the cheapest
/// instruction that exists. That is deliberate — it isolates what antegen costs
/// — but it means the shape table measures antegen's *overhead* and not what a
/// real execution costs. Production fibers CPI into swaps, oracle refreshes and
/// liquidations, and those dominate. Subtracting this from a measured exec
/// splits the total into the part antegen controls and the part it does not.
fn measure_inner_instruction_alone(memo: &str) -> u64 {
    let (mut svm, _admin, payer) = create_test_env();
    let mut clock = get_clock(&svm);
    clock.unix_timestamp = FIXED_CLOCK;
    svm.set_sysvar(&clock);

    let ix = build_thread_memo(&payer.pubkey(), memo, None);
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], blockhash);
    svm.send_transaction(tx)
        .expect("a bare memo should succeed")
        .compute_units_consumed
}

/// Total data carried by every account a transaction touches.
///
/// This is what `SetLoadedAccountsDataSizeLimit` should be sized against, and
/// the gap between it and the 64 MiB default is one of the larger levers in the
/// whole model.
fn loaded_accounts_bytes(svm: &litesvm::LiteSVM, tx: &Transaction) -> u64 {
    tx.message
        .account_keys
        .iter()
        .map(|key| {
            svm.get_account(key)
                .map(|a| a.data.len() as u64)
                .unwrap_or(0)
        })
        .sum()
}

/// Build a thread matching `shape`, execute every fiber in its chain, and
/// measure each resulting transaction.
///
/// Returns one [`Measurement`] per transaction, because a chained thread pays
/// per exec rather than per trigger — a fact the fee model has to account for
/// and that a single-transaction measurement would hide.
fn measure(shape: &Shape) -> Vec<Measurement> {
    let (mut svm, admin, payer) = create_test_env();

    // Pin the clock before anything reads it.
    //
    // LiteSVM starts its clock at wall time, and `thread_exec` prices commission
    // by decaying it over `time_since_ready` — an `f64` interpolation that is
    // branch-heavy and only runs once the grace period has passed. Triggers
    // anchored to an absolute time (`Timestamp`, `Cron`) therefore land a
    // different distance past their ready moment on every run, and the measured
    // cost moves with it: an early draft of this file reported swings of 3000
    // units for the same shape between two consecutive runs, which is larger
    // than most of the differences it exists to detect.
    let mut clock = get_clock(&svm);
    clock.unix_timestamp = FIXED_CLOCK;
    svm.set_sysvar(&clock);

    let authority = profile_authority();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&payer.pubkey(), DEFAULT_AIRDROP.checked_mul(4).unwrap())
        .unwrap();

    let (config_pubkey, _) = config_pda();
    let id = PROFILE_THREAD_ID;
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), id);

    // Create the thread.
    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        200_000_000,
        ThreadId::Bytes(id.to_vec()),
        shape.trigger.clone(),
        None,
        None,
        None,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx)
        .unwrap_or_else(|e| panic!("{}: create_thread failed: {:?}", shape.name, e.err));

    // Create each fiber.
    let mut fiber_pubkeys = Vec::new();
    for (index, (memo, signal)) in shape.fibers.iter().enumerate() {
        let index = index as u8;
        let (fiber_pubkey, _) = fiber_pda(&thread_pubkey, index);
        let memo_ix = make_memo_instruction(memo, signal.clone());
        let serializable = make_serializable_instruction(&memo_ix);
        let ix = build_create_fiber(
            &authority.pubkey(),
            &thread_pubkey,
            &fiber_pubkey,
            index,
            serializable,
            0,
        );
        let blockhash = svm.latest_blockhash();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&payer.pubkey()),
            &[&payer, &authority],
            blockhash,
        );
        svm.send_transaction(tx)
            .unwrap_or_else(|e| panic!("{}: create_fiber {} failed: {:?}", shape.name, index, e.err));
        fiber_pubkeys.push(fiber_pubkey);
    }

    if shape.warp_seconds > 0 {
        advance_clock(&mut svm, shape.warp_seconds);
    }

    // Execute the chain, one transaction per fiber.
    let remaining = vec![
        AccountMeta::new_readonly(PROGRAM_ID, false),
        AccountMeta::new_readonly(executor.pubkey(), false),
    ];

    let mut measurements = Vec::new();
    for (cursor, fiber_pubkey) in fiber_pubkeys.iter().enumerate() {
        let ix = build_exec_thread(
            &executor.pubkey(),
            &thread_pubkey,
            fiber_pubkey,
            &config_pubkey,
            &admin.pubkey(),
            shape.forgo_commission,
            cursor as u8,
            &remaining,
        );
        let instruction_data_bytes = ix.data.len() as u64;

        let blockhash = svm.latest_blockhash();
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&executor.pubkey()),
            &[&executor],
            blockhash,
        );

        let header = &tx.message.header;
        let signatures = header.num_required_signatures as u64;
        let writable_accounts = tx.message.account_keys.len() as u64
            - header.num_readonly_signed_accounts as u64
            - header.num_readonly_unsigned_accounts as u64;
        let loaded = loaded_accounts_bytes(&svm, &tx);

        let meta = svm
            .send_transaction(tx)
            .unwrap_or_else(|e| panic!("{}: exec {} failed: {:?}", shape.name, cursor, e.err));

        let name = if shape.fibers.len() > 1 {
            format!("{} [{}/{}]", shape.name, cursor + 1, shape.fibers.len())
        } else {
            shape.name.to_string()
        };

        measurements.push(Measurement {
            name,
            consumed_units: meta.compute_units_consumed,
            signatures,
            writable_accounts,
            instruction_data_bytes,
            loaded_accounts_bytes: loaded,
        });
    }

    measurements
}

/// How late every shape executes, in seconds past its ready moment.
///
/// Held constant across shapes on purpose. Commission decays over this interval,
/// and the decay is the branch-heavy `f64` path, so a shape measured at a
/// different lateness is measuring the decay as much as the trigger. Sixty
/// seconds puts every shape in the same decaying regime — past the 5s grace
/// period, well inside the 295s decay window — which is also where a real
/// execution usually lands.
const SECONDS_LATE: i64 = 60;

/// Every shape worth a number.
fn shapes() -> Vec<Shape> {
    vec![
        Shape::new("immediate", Trigger::Immediate { jitter: 0 }).warp(SECONDS_LATE),
        // The same shape inside the grace period, where the multiplier returns
        // 1.0 immediately instead of interpolating. The gap between this and
        // `immediate` is what the decay arithmetic costs.
        Shape::new("immediate (in grace)", Trigger::Immediate { jitter: 0 }),
        Shape::new(
            "timestamp",
            Trigger::Timestamp {
                unix_ts: FIXED_CLOCK + 60,
                jitter: 0,
            },
        )
        .warp(60 + SECONDS_LATE),
        Shape::new(
            "interval",
            Trigger::Interval {
                seconds: 10,
                skippable: false,
                jitter: 0,
            },
        )
        .warp(10 + SECONDS_LATE),
        Shape::new("slot", Trigger::Slot { slot: 0 }).warp(SECONDS_LATE),
        // Cron is the shape to watch: it is the only trigger whose next
        // occurrence is computed on-chain, by parsing a schedule string.
        // Ready one minute after creation, so the warp clears it and leaves the
        // usual lateness.
        Shape::new(
            "cron (every minute)",
            Trigger::Cron {
                schedule: "0 * * * * * *".to_string(),
                skippable: false,
                jitter: 0,
            },
        )
        .warp(60 + SECONDS_LATE),
        // A sparse schedule, to separate the cost of parsing a schedule from the
        // cost of searching for its next occurrence. From 08:00 on a Friday the
        // next matching moment is 09:00, so the search has an hour of
        // non-matching minutes to step over.
        Shape::new(
            "cron (complex)",
            Trigger::Cron {
                schedule: "0 0,15,30,45 1-5,9-17 * * MON-FRI *".to_string(),
                skippable: false,
                jitter: 0,
            },
        )
        .warp(3600 + SECONDS_LATE),
        // Instruction payload size, to separate fixed overhead from per-byte cost.
        Shape::new("large memo (512B)", Trigger::Immediate { jitter: 0 })
            .fibers(vec![("m".repeat(512), None)])
            .warp(SECONDS_LATE),
        // Signals change the tail of the instruction, after the CPI returns.
        Shape::new("signal: close", Trigger::Immediate { jitter: 0 })
            .fibers(vec![("memo".to_string(), Some(Signal::Close))])
            .warp(SECONDS_LATE),
        // A chain pays per exec, not per trigger.
        Shape::new(
            "chain (2 fibers)",
            Trigger::Interval {
                seconds: 10,
                skippable: false,
                jitter: 0,
            },
        )
        .fibers(vec![
            ("first".to_string(), Some(Signal::Chain)),
            ("second".to_string(), None),
        ])
        .warp(10 + SECONDS_LATE),
        Shape::new("forgo commission", Trigger::Immediate { jitter: 0 })
            .forgo_commission()
            .warp(SECONDS_LATE),
    ]
}

// ─────────────────────────────────────────────────────────────────────────────
// The report
// ─────────────────────────────────────────────────────────────────────────────

/// Regression band for a single `thread_exec`.
///
/// Wide enough that ordinary refactoring does not trip it, narrow enough that a
/// change which doubles the cost of every execution on the network cannot land
/// unnoticed. These are the numbers the fee formula is calibrated against, so a
/// silent move in them is a silent move in what every executor is owed.
const CONSUMED_UNITS_CEILING: u64 = 120_000;
const CONSUMED_UNITS_FLOOR: u64 = 1_000;

#[test]
fn exec_cost_profile() {
    let measurements: Vec<Measurement> = shapes().iter().flat_map(measure).collect();

    println!("\n== Measured: what a thread_exec consumes ==\n");
    println!(
        "{:<24} {:>10} {:>6} {:>9} {:>9} {:>12}",
        "shape", "CU", "sigs", "writable", "ix bytes", "acct bytes"
    );
    println!("{}", "-".repeat(74));
    for m in &measurements {
        println!(
            "{:<24} {:>10} {:>6} {:>9} {:>9} {:>12}",
            m.name,
            m.consumed_units,
            m.signatures,
            m.writable_accounts,
            m.instruction_data_bytes,
            m.loaded_accounts_bytes,
        );
    }

    // Two baselines, because they behave differently enough that one number
    // would misrepresent the other: the ordinary triggers all cluster within a
    // few hundred units of each other, while cron costs nearly three times as
    // much and is the shape that actually breaks the flat reimbursement.
    for name in ["immediate", "cron (every minute)"] {
        let baseline = measurements
            .iter()
            .find(|m| m.name == name)
            .expect("baseline shape was measured");
        report_projection(baseline);
    }

    let baseline = measurements
        .iter()
        .find(|m| m.name == "immediate")
        .expect("baseline shape was measured");
    report_inner_instruction_cost(baseline);

    // Regression guard on the measured half.
    for m in &measurements {
        assert!(
            m.consumed_units > CONSUMED_UNITS_FLOOR,
            "{}: {} CU is implausibly cheap — the measurement is probably not \
             running the instruction",
            m.name,
            m.consumed_units
        );
        assert!(
            m.consumed_units < CONSUMED_UNITS_CEILING,
            "{}: {} CU exceeds the {} ceiling. Every executor pays this on every \
             execution, and the reimbursement formula is calibrated against it — \
             re-profile and reprice before raising the ceiling.",
            m.name,
            m.consumed_units,
            CONSUMED_UNITS_CEILING
        );
    }
}

/// The CU request policies worth comparing.
///
/// `Default200k` is what the node emitted before the oracle; the two measured
/// requests bracket what it emits now. Under SIMD-0553 the difference between
/// them is money rather than merely tidiness, which is the point of printing
/// them side by side.
const REQUESTS: [Request; 3] = [
    Request::Default200k,
    Request::Measured { margin_bps: 2_500 },
    Request::Measured { margin_bps: 300 },
];

/// What the thread's own instruction does to the bill.
///
/// The shape table above measures antegen wrapped around a memo, which is to say
/// it measures antegen. Real fibers call swaps and oracle refreshes, and compute
/// is additive across the CPI, so the executor's cost is antegen's overhead plus
/// whatever the thread's author chose to call.
///
/// That last clause is the whole problem. Under today's per-signature fee the
/// cost is the same 5000 whatever the fiber does, so a flat reimbursement is
/// exactly right. SIMD-0553 makes the cost proportional to compute — and the
/// compute is chosen by the thread's author while the bill is paid by whichever
/// executor picks the thread up. A flat reimbursement in that world is not
/// merely too small, it is the wrong shape, and it is one an author can exploit
/// deliberately by pointing a fiber at something expensive.
fn report_inner_instruction_cost(baseline: &Measurement) {
    // Roughly: a bare transfer, a token swap, a heavy CPI chain, and the
    // per-transaction ceiling.
    const INNER: [u64; 5] = [0, 50_000, 200_000, 500_000, 1_376_537];

    println!("\n== The fiber's own instruction, added to antegen's overhead ==\n");
    println!(
        "Antegen's overhead is {} units. The rest is whatever the fiber calls, \n\
         which the thread's author chooses and the executor pays for.\n",
        baseline.consumed_units
    );
    println!(
        "  {:>12} {:>12} {:>10} {:>12} {:>12}",
        "inner CU", "total CU", "@0.1", "@0.25", "@0.5"
    );
    for inner in INNER {
        let m = baseline.with_inner_instruction(inner);
        // The realistic policy: the oracle's request plus a declared
        // loaded-accounts limit, which is what the node emits today.
        let request = Request::Measured { margin_bps: 2_500 };
        let fees: Vec<u64> = model::RAMP
            .iter()
            .map(|(_, num, den)| m.projected_fee(request, true, *num, *den))
            .collect();
        println!(
            "  {:>12} {:>12} {:>10} {:>12} {:>12}",
            inner, m.consumed_units, fees[0], fees[1], fees[2],
        );
    }
    println!(
        "\n  (against a flat {} lamport reimbursement)\n",
        model::LEGACY_FEE_PER_SIGNATURE
    );
}

fn report_projection(baseline: &Measurement) {
    println!(
        "\n== Projected: lamports per exec under SIMD-0553 — `{}` ==",
        baseline.name
    );
    println!(
        "\n{} CU consumed. Today this costs {} lamports regardless.\n",
        baseline.consumed_units,
        baseline.current_fee()
    );

    for declare in [false, true] {
        println!(
            "loaded-accounts limit {}:",
            if declare {
                "declared (measured)"
            } else {
                "undeclared (64 MiB default)"
            }
        );
        println!(
            "  {:<18} {:>12} {:>12} {:>12} {:>12}",
            "CU request", "cost units", "@0.1", "@0.25", "@0.5"
        );
        for request in REQUESTS {
            let units = baseline.requested_cost_units(request, declare);
            let fees: Vec<u64> = model::RAMP
                .iter()
                .map(|(_, num, den)| baseline.projected_fee(request, declare, *num, *den))
                .collect();
            println!(
                "  {:<18} {:>12} {:>12} {:>12} {:>12}",
                request.label(),
                units,
                fees[0],
                fees[1],
                fees[2],
            );
        }
        println!();
    }

    println!(
        "reimbursement gap against the flat {} lamports the program pays \
         (state/config.rs, `calculate_reimbursement`):",
        model::LEGACY_FEE_PER_SIGNATURE
    );
    println!(
        "  {:<18} {:>10} {:>12} {:>12} {:>12}",
        "CU request", "declared", "@0.1", "@0.25", "@0.5"
    );
    for declare in [false, true] {
        for request in REQUESTS {
            let gaps: Vec<i64> = model::RAMP
                .iter()
                .map(|(_, num, den)| {
                    (baseline.projected_fee(request, declare, *num, *den) as i64)
                        .saturating_sub(model::LEGACY_FEE_PER_SIGNATURE as i64)
                })
                .collect();
            println!(
                "  {:<18} {:>10} {:>12} {:>12} {:>12}",
                request.label(),
                if declare { "yes" } else { "no" },
                gaps[0],
                gaps[1],
                gaps[2],
            );
        }
    }
    println!();
}

/// Even antegen's overhead alone outgrows the flat reimbursement.
///
/// This is the *floor* of the problem, not its size: it prices a thread whose
/// fiber does nothing, which no real thread does. A thread wrapping an ordinary
/// 200k-unit instruction is already six times underwater at the first ramp step
/// — see `report_inner_instruction_cost`. What this test pins is that the flat
/// reimbursement fails even in the case most favourable to it.
///
/// Bounds are asserted as multiples of the reimbursement rather than as absolute
/// lamports, so the test keeps meaning something if the measurement drifts.
/// Asserted at all so the program-side fix has something to turn green, and so
/// nobody has to re-derive the argument from a table.
#[test]
fn even_antegens_overhead_alone_outgrows_the_flat_reimbursement() {
    let baseline = measure(
        &Shape::new("immediate", Trigger::Immediate { jitter: 0 }).warp(SECONDS_LATE),
    )
    .into_iter()
    .next()
    .expect("one exec");

    // The best the client can currently do: the oracle's tight request plus a
    // declared loaded-accounts limit. Both are already implemented.
    let best = Request::Measured { margin_bps: 300 };

    let (_, num, den) = model::RAMP[0];
    let first_step = baseline.projected_fee(best, true, num, den);
    assert!(
        first_step < 2 * model::LEGACY_FEE_PER_SIGNATURE,
        "a tightly-requested exec costs {} lamports at the first ramp step, more \
         than double the {} it is reimbursed. The first step was the one the \
         current formula could absorb — if it no longer is, the program-side fix \
         has to land before the first gate activates rather than before the last.",
        first_step,
        model::LEGACY_FEE_PER_SIGNATURE
    );

    let (_, num, den) = model::RAMP[2];
    let terminal = baseline.projected_fee(best, true, num, den);
    assert!(
        terminal > 2 * model::LEGACY_FEE_PER_SIGNATURE,
        "even the best-case exec should cost more than double the flat {} \
         reimbursement by the terminal rate; it costs {}. If this stops holding \
         the fee model changed and the program-side plan should be revisited.",
        model::LEGACY_FEE_PER_SIGNATURE,
        terminal
    );

    // The case that actually decides the deadline: an ordinary fiber, at the
    // *first* ramp step. Antegen's overhead alone is survivable there; a thread
    // that calls a 200k-unit instruction is not, and that is the common case
    // rather than the pathological one.
    let realistic = baseline.with_inner_instruction(200_000);
    let (_, num, den) = model::RAMP[0];
    let realistic_first_step = realistic.projected_fee(best, true, num, den);
    assert!(
        realistic_first_step > 5 * model::LEGACY_FEE_PER_SIGNATURE,
        "a thread wrapping a 200k-unit instruction costs {} lamports at the first \
         ramp step against a {} reimbursement. This was expected to be several \
         times underwater at the very first gate — if it is not, the fee model \
         changed and the program-side deadline can move.",
        realistic_first_step,
        model::LEGACY_FEE_PER_SIGNATURE
    );

    // Cron is the shape that runs out of room first among the triggers: it
    // consumes nearly three times the overhead of the others.
    let cron = measure(
        &Shape::new(
            "cron",
            Trigger::Cron {
                schedule: "0 * * * * * *".to_string(),
                skippable: false,
                jitter: 0,
            },
        )
        .warp(60 + SECONDS_LATE),
    )
    .into_iter()
    .next()
    .expect("one exec");

    let (_, num, den) = model::RAMP[0];
    assert!(
        cron.projected_fee(best, true, num, den) > first_step,
        "a cron exec should cost more than an immediate one at the same rate; \
         if it does not, the on-chain schedule computation stopped being the \
         dominant cost and the reimbursement can be simpler than planned"
    );
}

/// The fiber's instruction adds to the total, one unit for one unit.
///
/// The whole projection for real threads rests on this: if compute were not
/// additive across the CPI, antegen's overhead would be the answer and the fee
/// model could be a constant. It is additive, so the answer is unbounded and the
/// fee model cannot be.
///
/// Checked by growing the memo — the only inner instruction here whose cost is
/// tunable — and confirming the exec grows by the same amount the bare
/// instruction does.
#[test]
fn the_inner_instruction_adds_to_the_total() {
    let small = "memo";
    let large = "m".repeat(512);

    let alone_delta = measure_inner_instruction_alone(&large)
        .saturating_sub(measure_inner_instruction_alone(small));

    let wrapped_small = measure(
        &Shape::new("small", Trigger::Immediate { jitter: 0 })
            .fibers(vec![(small.to_string(), None)])
            .warp(SECONDS_LATE),
    )[0]
        .consumed_units;
    let wrapped_large = measure(
        &Shape::new("large", Trigger::Immediate { jitter: 0 })
            .fibers(vec![(large.clone(), None)])
            .warp(SECONDS_LATE),
    )[0]
        .consumed_units;
    let wrapped_delta = wrapped_large.saturating_sub(wrapped_small);

    println!(
        "inner instruction grew by {} units alone, {} units wrapped in a thread",
        alone_delta, wrapped_delta
    );

    // Equal to within the noise of a differently-sized instruction payload
    // travelling through `thread_exec`'s own deserialization.
    let difference = (alone_delta as i64).saturating_sub(wrapped_delta as i64).abs();
    assert!(
        difference < 200,
        "growing the inner instruction by {} units alone moved the wrapped exec \
         by {}. These should track each other; if they have diverged, compute is \
         not passing through the CPI the way the fee projection assumes.",
        alone_delta,
        wrapped_delta
    );
}

/// Declining commission costs the executor more compute than taking it.
///
/// `calculate_payments` already computes the effective commission, and then the
/// `forgo_commission` branch in `thread_exec` computes it a second time — along
/// with the executor's share of it — purely to name the forgone amount in a
/// `msg!`. The multiplier is `f64`, which is not cheap under SBF.
///
/// Free today. Under SIMD-0553 the extra units are billed to the executor who
/// just declined to be paid, which is a strange thing to charge for.
#[test]
fn forgoing_commission_costs_the_executor_extra_compute() {
    let taken = measure(&Shape::new("take", Trigger::Immediate { jitter: 0 }).warp(SECONDS_LATE))
        .into_iter()
        .next()
        .expect("one exec");
    let forgone = measure(
        &Shape::new("forgo", Trigger::Immediate { jitter: 0 })
            .forgo_commission()
            .warp(SECONDS_LATE),
    )
    .into_iter()
    .next()
    .expect("one exec");

    let overhead = (forgone.consumed_units as i64).saturating_sub(taken.consumed_units as i64);
    println!(
        "forgoing commission costs {} extra CU ({} vs {})",
        overhead, forgone.consumed_units, taken.consumed_units
    );

    assert!(
        overhead > 0,
        "expected the forgo path to cost more, since it recomputes the commission \
         to log it; measured {} CU against {}. If this has been fixed, delete the \
         test rather than inverting it.",
        forgone.consumed_units,
        taken.consumed_units
    );
}
