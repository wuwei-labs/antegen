//! Executor Logic
//!
//! Handles building and simulating thread execution transactions.
//! Includes batching detection via Signal and compute budget estimation.
//!
//! Batching is determined by the fiber's Signal return value:
//! - Signal::Chain → batch another exec for next fiber in sequence
//! - Signal::Close → batch a delete instruction
//! - Other signals → no batching needed

use crate::exec_ix::{build_thread_exec_instruction, thread_exec_base_accounts, ThreadExecParams};
use crate::resources::SharedResources;
use crate::rpc::response::decode_account_data;
use crate::tx::{self, TxConfig, TxVersion};
use anchor_lang::{AccountDeserialize, AnchorDeserialize, InstructionData};
use antegen_thread_program::fiber::{decompile_instruction, CompiledInstructionV0, Fiber};
use antegen_thread_program::state::PAYER_PUBKEY;
use antegen_thread_program::{
    instruction::ExecThread,
    state::{Signal, Thread, ThreadConfig},
};
use solana_sdk::{
    account::Account,
    hash::Hash,
    instruction::{AccountMeta, Instruction},
    message::Message,
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
};
use std::collections::HashSet;

use anyhow::{anyhow, Result};
use log::{debug, warn};
use std::sync::Arc;

/// One transaction's worth of a thread's execution, as planned.
#[derive(Debug, Default, Clone)]
pub struct ExecBatch {
    pub instructions: Vec<Instruction>,
    pub priority_fee: u64,
    /// Whether the chain still has work for this thread after this batch lands.
    pub needs_continuation: bool,
    /// Cursor the continuation batch starts from. Needed because an on-chain
    /// Chain signal does not advance `fiber_cursor`.
    pub next_fiber_cursor: Option<u8>,
    /// Compute units observed by the batching simulation, but only when that
    /// simulation covered the final instruction set. `None` when an instruction
    /// was appended after the last simulate, in which case the caller must
    /// estimate separately.
    pub simulated_units: Option<u64>,
    /// Account data bytes the simulation loaded. `None` when unmeasured, in
    /// which case no limit is requested and the runtime's 64 MiB default
    /// applies.
    pub loaded_accounts_bytes: Option<u32>,
}

/// Compute and account-data footprint measured by a simulation.
#[derive(Debug, Default, Clone, Copy)]
pub struct SimulatedResources {
    pub units: u64,
    pub loaded_accounts_bytes: Option<u32>,
}

/// Executor logic for building thread execution transactions
#[derive(Clone)]
pub struct ExecutorLogic {
    /// Executor keypair for signing transactions
    keypair: Arc<Keypair>,
    /// Shared resources (RPC pool, cache)
    resources: SharedResources,
    /// Whether to forgo executor commission
    forgo_executor_commission: bool,
    /// Thread program ID (configurable)
    program_id: Pubkey,
    /// Message format to build. Governs both the size ceiling batching aims at
    /// and how resource limits are encoded, which is why it is held here rather
    /// than read at each build site.
    tx_version: TxVersion,
}

impl ExecutorLogic {
    /// Create a new executor logic instance
    pub fn new(
        keypair: Arc<Keypair>,
        resources: SharedResources,
        forgo_executor_commission: bool,
    ) -> Self {
        let program_id = resources.program_id;
        Self {
            keypair,
            resources,
            forgo_executor_commission,
            program_id,
            tx_version: TxVersion::default(),
        }
    }

    /// Select the message format to build.
    ///
    /// Separate from `new` so the existing constructor keeps its signature —
    /// the default is legacy, which is what every caller was getting before the
    /// format became selectable.
    pub fn with_tx_version(mut self, tx_version: TxVersion) -> Self {
        self.tx_version = tx_version;
        self
    }

    /// Message format this executor builds.
    pub fn tx_version(&self) -> TxVersion {
        self.tx_version
    }

    /// Get executor pubkey
    pub fn pubkey(&self) -> Pubkey {
        self.keypair.pubkey()
    }

    /// Get the keypair reference
    pub fn keypair(&self) -> &Arc<Keypair> {
        &self.keypair
    }

    /// Build a single transaction batch to execute a thread with automatic batching.
    ///
    /// Simulates to detect chaining signals and estimate CU consumption.
    /// Batching is determined by the Signal returned from the fiber:
    /// - Signal::Chain → batch another exec for next fiber in sequence
    /// - Signal::Close → batch a delete instruction
    ///
    /// If chained instructions would exceed Solana's max transaction size, only
    /// the instructions that fit are returned and `needs_continuation` is set to
    /// `true`. The caller should submit this batch, confirm it, re-fetch the
    /// thread, and call this method again to build the next batch.
    ///
    /// Reusing the batching simulation's measurements saves a full simulate
    /// round trip on the single-fiber path, which is the overwhelming majority
    /// of executions.
    pub async fn build_execute_transaction(
        &self,
        thread_pubkey: &Pubkey,
        thread: &Thread,
        override_fiber_cursor: Option<u8>,
    ) -> Result<ExecBatch> {
        // Log thread state for debugging
        self.log_thread_debug(thread, thread_pubkey);

        const MAX_BATCHED_EXECS: usize = 5;
        let mut priority_fee: u64 = 0;
        let mut ixs: Vec<Instruction> = Vec::new();
        let mut needs_continuation = false;
        let mut next_fiber_cursor: Option<u8> = None;
        // Measurements from the most recent simulate, valid only while `ixs` is
        // unchanged since. Cleared on every push.
        let mut simulated: Option<SimulatedResources> = None;

        // Track fiber_cursor through the chaining loop
        // Signal::Chain tells us to execute next fiber in sequence
        let mut current_fiber_cursor = override_fiber_cursor.unwrap_or(thread.fiber_cursor);

        // Build first instruction
        debug!(
            "{}: starting build: thread.fiber_cursor={}, override={:?}, using={}",
            thread_pubkey, thread.fiber_cursor, override_fiber_cursor, current_fiber_cursor
        );
        let first_ix = self
            .build_thread_exec_ix(
                &mut priority_fee,
                thread_pubkey,
                thread,
                current_fiber_cursor,
            )
            .await?;

        // Empty fiber — nothing to submit
        let Some(first_ix) = first_ix else {
            debug!("{}: first fiber is empty, nothing to submit", thread_pubkey);
            return Ok(ExecBatch::default());
        };

        debug!(
            "First instruction built successfully, priority_fee: {}",
            priority_fee
        );

        // Verify single instruction fits in a transaction
        if !self.would_fit_in_transaction(std::slice::from_ref(&first_ix)) {
            return Err(anyhow!(
                "Single instruction exceeds max transaction size for thread {}",
                thread_pubkey
            ));
        }
        ixs.push(first_ix);

        loop {
            if ixs.len() >= MAX_BATCHED_EXECS {
                warn!(
                    "Reached max batched executions ({}), stopping",
                    MAX_BATCHED_EXECS
                );
                break;
            }

            // Simulate current batch to check for batching signals
            debug!(
                "Simulating transaction with {} instruction(s) to check for batching...",
                ixs.len()
            );
            let (signal, resources) = self.simulate_transaction(&ixs, thread_pubkey).await?;
            simulated = Some(resources);
            debug!(
                "{}: fiber {} simulation signal={:?}",
                thread_pubkey, current_fiber_cursor, signal
            );

            // Handle signal - only Chain and Close trigger batching
            match signal {
                Signal::Chain => {
                    // Calculate next fiber in sequence
                    current_fiber_cursor =
                        Self::next_fiber_in_sequence(&thread.fiber_ids, current_fiber_cursor);
                    debug!(
                        "Batching: Signal::Chain, adding thread_exec for fiber {}",
                        current_fiber_cursor
                    );
                    let next_ix = self
                        .build_thread_exec_ix(
                            &mut priority_fee,
                            thread_pubkey,
                            thread,
                            current_fiber_cursor,
                        )
                        .await?;

                    // Empty fiber — stop chaining
                    let Some(next_ix) = next_ix else {
                        debug!(
                            "{}: chained fiber {} is empty, stopping chain",
                            thread_pubkey, current_fiber_cursor
                        );
                        break;
                    };

                    // Check if adding this instruction would exceed transaction size
                    let mut trial = ixs.clone();
                    trial.push(next_ix.clone());
                    let trial_size = self.estimate_transaction_size_with_budget(&trial);
                    if trial_size <= self.tx_version.max_transaction_size() {
                        ixs.push(next_ix);
                        // The batch grew; the measurement no longer covers it.
                        // If the loop now exits on MAX_BATCHED_EXECS this stays
                        // None and the caller estimates properly.
                        simulated = None;
                    } else {
                        // Doesn't fit — return what we have and signal continuation.
                        // The worker will submit this batch, confirm it, re-fetch
                        // the thread, and call us again for the next batch.
                        let current_size = self.estimate_transaction_size_with_budget(&ixs);
                        debug!(
                            "{}: transaction full ({} ix, {} bytes), adding fiber {} would be {} bytes (max {}), needs continuation",
                            thread_pubkey,
                            ixs.len(),
                            current_size,
                            current_fiber_cursor,
                            trial_size,
                            self.tx_version.max_transaction_size()
                        );
                        needs_continuation = true;
                        next_fiber_cursor = Some(current_fiber_cursor);
                        break;
                    }
                }
                Signal::Close => {
                    // Build thread_exec that executes the pre-compiled close_fiber
                    debug!("Signal::Close detected - building thread_exec with close_fiber");
                    let close_ix = self.build_close_thread_exec(thread_pubkey, thread).await?;

                    // Check if close instruction fits in current batch
                    let mut trial = ixs.clone();
                    trial.push(close_ix.clone());
                    if self.would_fit_in_transaction(&trial) {
                        ixs.push(close_ix);
                        // close_ix was never simulated.
                        simulated = None;
                    } else {
                        debug!(
                            "{}: transaction full ({} ix), close deferred to continuation",
                            thread_pubkey,
                            ixs.len()
                        );
                        needs_continuation = true;
                    }
                    break;
                }
                _ => {
                    // No batching needed for None, Repeat, Next, Update
                    debug!(
                        "{}: signal={:?}, no chaining needed ({} exec instruction(s))",
                        thread_pubkey,
                        signal,
                        ixs.len()
                    );
                    break;
                }
            }
        }

        // Transaction-level account audit for batched instructions
        if ixs.len() > 1 {
            let mut all_pubkeys: HashSet<Pubkey> = HashSet::new();
            for (i, ix) in ixs.iter().enumerate() {
                let ix_pubkeys: HashSet<Pubkey> = ix.accounts.iter().map(|a| a.pubkey).collect();
                debug!(
                    "{}: ix[{}] has {} accounts ({} unique)",
                    thread_pubkey,
                    i,
                    ix.accounts.len(),
                    ix_pubkeys.len()
                );
                all_pubkeys.extend(ix_pubkeys);
            }
            let message = Message::new(&ixs, Some(&self.keypair.pubkey()));
            debug!(
                "{}: batched transaction: {} instructions, {} unique accounts in message, {} account_keys",
                thread_pubkey,
                ixs.len(),
                all_pubkeys.len(),
                message.account_keys.len()
            );
        }

        debug!(
            "{}: built {} instruction(s), priority_fee={}, continuation={}",
            thread_pubkey,
            ixs.len(),
            priority_fee,
            needs_continuation
        );

        Ok(ExecBatch {
            instructions: ixs,
            priority_fee,
            needs_continuation,
            next_fiber_cursor,
            simulated_units: simulated.map(|s| s.units),
            loaded_accounts_bytes: simulated.and_then(|s| s.loaded_accounts_bytes),
        })
    }

    /// Fetch thread account from RPC and deserialize.
    pub async fn fetch_thread(&self, thread_pubkey: &Pubkey) -> Result<Thread> {
        // Bypass cache — we need fresh on-chain state after a confirmed transaction
        let ui_account = self
            .resources
            .rpc_client
            .get_account(thread_pubkey)
            .await
            .map_err(|e| anyhow!("Failed to fetch thread {}: {}", thread_pubkey, e))?
            .ok_or_else(|| anyhow!("Thread {} not found (may have been closed)", thread_pubkey))?;

        let data =
            crate::rpc::response::decode_account_data(&ui_account.data.0, &ui_account.data.1)
                .map_err(|e| anyhow!("Failed to decode thread account data: {}", e))?;

        Thread::try_deserialize(&mut data.as_slice())
            .map_err(|e| anyhow!("Failed to deserialize thread {}: {}", thread_pubkey, e))
    }

    /// Estimate transaction size including the compute budget overhead the
    /// worker will prepend later.
    fn estimate_transaction_size_with_budget(&self, instructions: &[Instruction]) -> usize {
        tx::estimate_size(
            self.tx_version,
            &self.keypair.pubkey(),
            instructions,
            &TxConfig::reserving_limits(),
        )
        .unwrap_or(usize::MAX)
    }

    /// Check if instructions (plus compute budget overhead) would fit in one transaction.
    fn would_fit_in_transaction(&self, instructions: &[Instruction]) -> bool {
        self.estimate_transaction_size_with_budget(instructions)
            <= self.tx_version.max_transaction_size()
    }

    /// Estimate compute units for a set of instructions via simulation.
    pub async fn estimate_resources(
        &self,
        instructions: &[Instruction],
        thread_pubkey: &Pubkey,
    ) -> Result<SimulatedResources> {
        let (_, resources) = self
            .simulate_transaction(instructions, thread_pubkey)
            .await?;
        Ok(resources)
    }

    /// Log thread state for debugging
    fn log_thread_debug(&self, thread: &Thread, thread_pubkey: &Pubkey) {
        debug!("Building execute transaction for thread: {}", thread_pubkey);
        debug!("  fiber_cursor: {}", thread.fiber_cursor);
        debug!("  fiber_ids: {:?}", thread.fiber_ids);
        debug!("  fiber_signal: {:?}", thread.fiber_signal);
        debug!("  trigger: {:?}", thread.trigger);
        debug!("  schedule: {:?}", thread.schedule);
        debug!("  paused: {}", thread.paused);
        debug!("  exec_count: {}", thread.exec_count);
    }

    /// Calculate the next fiber index in sequence
    /// Used for Signal::Chain which always chains to the next consecutive fiber
    fn next_fiber_in_sequence(fiber_ids: &[u8], current_cursor: u8) -> u8 {
        if fiber_ids.is_empty() {
            return 0;
        }
        if let Some(current_pos) = fiber_ids.iter().position(|&x| x == current_cursor) {
            let next_pos = (current_pos + 1) % fiber_ids.len();
            fiber_ids[next_pos]
        } else {
            fiber_ids.first().copied().unwrap_or(0)
        }
    }

    /// Build thread_exec instruction for a specific fiber, returning the instruction.
    ///
    /// Fetches the external fiber account to get compiled instruction and priority fee.
    async fn build_thread_exec_ix(
        &self,
        priority_fee: &mut u64,
        thread_pubkey: &Pubkey,
        thread: &Thread,
        fiber_cursor: u8,
    ) -> Result<Option<Instruction>> {
        debug!("build_thread_exec_ix: fiber_cursor={}", fiber_cursor);

        // Fetch the fiber account
        let fiber_pubkey = thread.fiber_at_index(thread_pubkey, fiber_cursor);

        debug!(
            "Fetching fiber account: {} (fiber_cursor={})",
            fiber_pubkey, fiber_cursor
        );

        let account = self.fetch_fiber_account(&fiber_pubkey).await?;
        let fiber_read = Fiber::try_deserialize(&mut account.data.as_slice())
            .map_err(|e| anyhow!("Failed to deserialize fiber {}: {}", fiber_pubkey, e))?;

        // Empty compiled_instruction = cleared fiber (e.g. after close). Skip.
        if fiber_read.compiled_instruction().is_empty() {
            debug!(
                "fiber_{} has empty compiled_instruction, skipping",
                fiber_cursor
            );
            return Ok(None);
        }

        debug!("Fiber fetched, priority_fee={}", fiber_read.priority_fee());

        // Build execute instruction
        let ix = self
            .build_execute_instruction(
                thread_pubkey,
                thread,
                fiber_cursor,
                fiber_read.compiled_instruction(),
            )
            .await?;

        *priority_fee = (*priority_fee).max(fiber_read.priority_fee());

        Ok(Some(ix))
    }

    /// Build base ThreadExec accounts (shared by build_execute_instruction and build_close_thread_exec)
    async fn build_thread_exec_base_accounts(
        &self,
        thread_pubkey: &Pubkey,
        thread: &Thread,
        fiber_pubkey: Pubkey,
    ) -> Result<(Vec<AccountMeta>, ThreadConfig)> {
        let config_pubkey = ThreadConfig::pubkey();
        let config = self.fetch_thread_config(&config_pubkey).await?;

        let accounts = thread_exec_base_accounts(
            self.keypair.pubkey(),
            *thread_pubkey,
            thread,
            fiber_pubkey,
            config_pubkey,
            config.admin,
        );

        Ok((accounts, config))
    }

    /// Build exec_thread instruction
    async fn build_execute_instruction(
        &self,
        thread_pubkey: &Pubkey,
        thread: &Thread,
        fiber_cursor: u8,
        compiled_instruction: &[u8],
    ) -> Result<Instruction> {
        debug!(
            "Building exec_thread instruction: thread={}, fiber_cursor={}",
            thread_pubkey, fiber_cursor,
        );

        // Get compiled instruction from fiber account
        let fiber_pubkey = thread.fiber_at_index(thread_pubkey, fiber_cursor);
        let compiled = CompiledInstructionV0::deserialize(&mut &compiled_instruction[..])?;

        // Diagnostic: decompile and verify all instruction accounts are in compiled.accounts
        let remaining_pubkeys: HashSet<Pubkey> = compiled
            .accounts
            .iter()
            .map(|pk| {
                if pk.eq(&PAYER_PUBKEY) {
                    self.keypair.pubkey()
                } else {
                    *pk
                }
            })
            .collect();

        match decompile_instruction(&compiled) {
            Ok(decompiled) => {
                debug!(
                    "fiber_{} account audit: compiled_table={} unique, decompiled_accounts={}, program_id={}",
                    fiber_cursor,
                    compiled.accounts.len(),
                    decompiled.accounts.len(),
                    decompiled.program_id
                );

                let program_id_resolved = if decompiled.program_id.eq(&PAYER_PUBKEY) {
                    self.keypair.pubkey()
                } else {
                    decompiled.program_id
                };
                if !remaining_pubkeys.contains(&program_id_resolved) {
                    warn!(
                        "MISSING: program_id {} not in compiled.accounts table!",
                        program_id_resolved
                    );
                }

                for (i, acc) in decompiled.accounts.iter().enumerate() {
                    let resolved = if acc.pubkey.eq(&PAYER_PUBKEY) {
                        self.keypair.pubkey()
                    } else {
                        acc.pubkey
                    };
                    if !remaining_pubkeys.contains(&resolved) {
                        warn!(
                            "MISSING: account[{}] {} (signer={}, writable={}) not in compiled.accounts table!",
                            i, resolved, acc.is_signer, acc.is_writable
                        );
                    }
                    debug!(
                        "  decompiled[{}]: {} signer={} writable={} in_table={}",
                        i,
                        resolved,
                        acc.is_signer,
                        acc.is_writable,
                        remaining_pubkeys.contains(&resolved)
                    );
                }
            }
            Err(e) => {
                warn!("Failed to decompile instruction for audit: {}", e);
            }
        }

        // Assembly is shared with the CLI's `thread exec` — see
        // antegen_client::exec_ix. Only the fetching differs between callers.
        let config_pubkey = ThreadConfig::pubkey();
        let config = self.fetch_thread_config(&config_pubkey).await?;

        let ix = build_thread_exec_instruction(&ThreadExecParams {
            program_id: self.program_id,
            executor: self.keypair.pubkey(),
            thread_pubkey: *thread_pubkey,
            thread,
            fiber_pubkey,
            compiled: &compiled,
            config_pubkey,
            admin: config.admin,
            fiber_cursor,
            forgo_commission: self.forgo_executor_commission,
        });

        debug!(
            "fiber_{} instruction: program={}, remaining={}, total={}, data_len={}",
            fiber_cursor,
            self.program_id,
            compiled.accounts.len(),
            ix.accounts.len(),
            ix.data.len()
        );

        Ok(ix)
    }

    /// Build thread_exec instruction that executes close_fiber to delete the thread
    ///
    /// When Signal::Close is detected, we build a thread_exec that:
    /// 1. Executes the pre-compiled close_fiber (which CPIs to thread_delete)
    /// 2. Passes all external fiber accounts as remaining_accounts for cleanup
    async fn build_close_thread_exec(
        &self,
        thread_pubkey: &Pubkey,
        thread: &Thread,
    ) -> Result<Instruction> {
        debug!(
            "Building close thread_exec: thread={}, fiber_ids={:?}",
            thread_pubkey, thread.fiber_ids
        );

        // Use the first fiber for the fiber account field
        let first_fiber_index = thread.fiber_ids.first().copied().unwrap_or(0);
        let first_fiber_pubkey = thread.fiber_at_index(thread_pubkey, first_fiber_index);

        // Build base accounts
        let (mut accounts, _config) = self
            .build_thread_exec_base_accounts(thread_pubkey, thread, first_fiber_pubkey)
            .await?;

        // Add external fiber accounts as remaining_accounts for thread_delete to close
        for &fiber_index in &thread.fiber_ids {
            let fiber_pda = thread.fiber_at_index(thread_pubkey, fiber_index);
            debug!(
                "Adding fiber account for deletion: {} (index={})",
                fiber_pda, fiber_index
            );
            accounts.push(AccountMeta {
                pubkey: fiber_pda,
                is_signer: false,
                is_writable: true, // Needs to be writable to close
            });
        }

        // Add accounts needed for close_fiber CPI to thread_delete
        // The close_fiber is a compiled thread_delete instruction that needs these accounts
        // in remaining_accounts for invoke_signed to find them:

        // 1. Thread account (needed for CPI even though it's in ThreadExec accounts)
        accounts.push(AccountMeta {
            pubkey: *thread_pubkey,
            is_signer: false,
            is_writable: true,
        });

        // 2. close_to (thread.authority - receives rent)
        accounts.push(AccountMeta {
            pubkey: thread.authority,
            is_signer: false,
            is_writable: true,
        });

        // 3. Thread program ID (for CPI)
        accounts.push(AccountMeta {
            pubkey: self.program_id,
            is_signer: false,
            is_writable: false,
        });

        // 4. Fiber program ID (needed by ThreadClose for CPI to close_fiber)
        accounts.push(AccountMeta {
            pubkey: antegen_thread_program::fiber::ID,
            is_signer: false,
            is_writable: false,
        });

        // Build instruction data - fiber_cursor doesn't matter since Signal::Close is set
        let data = ExecThread {
            forgo_commission: self.forgo_executor_commission,
            fiber_cursor: 0,
        }
        .data();

        debug!(
            "Close thread_exec built: {} accounts, {} external fibers",
            accounts.len(),
            thread.fiber_ids.len()
        );

        Ok(Instruction {
            program_id: self.program_id,
            accounts,
            data,
        })
    }

    /// Simulate transaction and extract Signal and compute units consumed
    ///
    /// Uses the RPC pool for failover and health tracking.
    ///
    /// Returns (signal, units_consumed)
    /// - signal: The fiber_signal from post-simulation thread state (determines batching)
    /// - units_consumed: compute units used by the transaction
    async fn simulate_transaction(
        &self,
        instructions: &[Instruction],
        thread_pubkey: &Pubkey,
    ) -> Result<(Signal, SimulatedResources)> {
        debug!(
            "Simulating transaction: thread={}, num_instructions={}",
            thread_pubkey,
            instructions.len(),
        );

        // Log instruction details
        for (i, ix) in instructions.iter().enumerate() {
            debug!(
                "  Instruction {}: program={}, num_accounts={}, data_len={}",
                i,
                ix.program_id,
                ix.accounts.len(),
                ix.data.len()
            );
            if i == 0 {
                for (j, acc) in ix.accounts.iter().enumerate() {
                    debug!(
                        "    account[{}]: {} (signer={}, writable={})",
                        j, acc.pubkey, acc.is_signer, acc.is_writable
                    );
                }
            }
        }

        // 1. Build transaction with generous CU limit for simulation headroom.
        // The actual CU limit is set precisely later by the worker.
        //
        // No blockhash is fetched: the simulation request sets
        // `replaceRecentBlockhash: true` and `sigVerify: false`, so the
        // validator substitutes its own and ignores whatever we send. Fetching
        // one here was a round trip on the critical path whose result was
        // discarded server-side.
        //
        // The simulation runs in whatever format the executor is configured to
        // emit. Simulating in one format and submitting in another would hide
        // exactly the class of failure that changing format introduces.
        let tx = tx::build_transaction(
            self.tx_version,
            &[self.keypair.as_ref()],
            &self.keypair.pubkey(),
            instructions,
            &TxConfig::new().with_compute_unit_limit(tx::MAX_COMPUTE_UNITS),
            Hash::default(),
        )?;

        // 2. Simulate via RPC pool (handles failover, returns result with accounts)
        let result = match self
            .resources
            .rpc_client
            .simulate_transaction(&tx, &[*thread_pubkey])
            .await
        {
            Ok(r) => r,
            Err(e) => {
                // One line at warn so the failure is visible, with the account
                // dump behind debug.
                //
                // Trigger-not-ready and paused stay silent entirely: they are
                // expected outcomes of firing on a projected clock and the
                // caller retries them. Everything else used to dump every
                // account of every instruction at warn on every attempt — for a
                // thread whose fiber had gone stale, that was around fifty lines
                // every few seconds for eight hours, which buries the very
                // problems the dump exists to surface.
                let rendered = e.to_string();
                if !rendered.contains("6004") && !rendered.contains("6006") {
                    warn!(
                        "Simulation failed for thread {} ({} instruction(s)): {}",
                        thread_pubkey,
                        instructions.len(),
                        rendered
                    );
                    for (i, ix) in instructions.iter().enumerate() {
                        debug!(
                            "  IX[{}] program={}, {} accounts:",
                            i,
                            ix.program_id,
                            ix.accounts.len()
                        );
                        for (j, acc) in ix.accounts.iter().enumerate() {
                            debug!(
                                "    [{}]: {} signer={} writable={}",
                                j, acc.pubkey, acc.is_signer, acc.is_writable
                            );
                        }
                    }
                }
                return Err(e);
            }
        };

        // Log simulation logs
        if let Some(logs) = &result.value.logs {
            debug!("Simulation logs ({} lines):", logs.len());
            for (i, log) in logs.iter().enumerate() {
                if i < 20 {
                    debug!("  [{}] {}", i, log);
                } else if i == 20 {
                    debug!("  ... ({} more log lines)", logs.len() - 20);
                    break;
                }
            }
        }

        // 4. Extract the resource footprint (units_consumed safely handles float).
        //
        // `loaded_accounts_data_size` is what the transaction actually needed to
        // load. Absent on older RPC versions, in which case no limit is
        // requested and the runtime charges its 64 MiB default — the behaviour
        // that applied before this was measured at all.
        let resources = SimulatedResources {
            units: result.value.units_consumed.unwrap_or(0),
            loaded_accounts_bytes: result.value.loaded_accounts_data_size,
        };
        debug!(
            "Simulation consumed {} CU, loaded {:?} bytes of account data",
            resources.units, resources.loaded_accounts_bytes
        );

        // 5. Extract signal from thread account
        let signal = if let Some(accounts) = &result.value.accounts {
            if let Some(Some(ui_account)) = accounts.first() {
                // Decode account data (supports base64 and base64+zstd)
                match decode_account_data(&ui_account.data.0, &ui_account.data.1) {
                    Ok(data) => {
                        if data.len() < 8 {
                            debug!("Thread account has insufficient data (likely closed)");
                            Signal::None
                        } else {
                            match Thread::try_deserialize(&mut data.as_slice()) {
                                Ok(thread) => {
                                    debug!(
                                        "{}: extracted signal={:?} from simulation",
                                        thread_pubkey, thread.fiber_signal
                                    );
                                    thread.fiber_signal
                                }
                                Err(e) => {
                                    warn!("Failed to deserialize thread from simulation: {}", e);
                                    Signal::None
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to decode account data: {}", e);
                        Signal::None
                    }
                }
            } else {
                warn!(
                    "{}: no account data in simulation response (account is null)",
                    thread_pubkey
                );
                Signal::None
            }
        } else {
            warn!("{}: no accounts in simulation response", thread_pubkey);
            Signal::None
        };

        Ok((signal, resources))
    }

    /// Fetch fiber account directly from RPC, bypassing cache.
    /// Fiber compiled_instruction may change via fiber_update; stale cache
    /// causes MissingAccount when remaining_accounts diverge from on-chain state.
    async fn fetch_fiber_account(&self, pubkey: &Pubkey) -> Result<Account> {
        let ui_account = self
            .resources
            .rpc_client
            .get_account(pubkey)
            .await
            .map_err(|e| anyhow!("Failed to fetch fiber {}: {}", pubkey, e))?
            .ok_or_else(|| anyhow!("Fiber {} not found", pubkey))?;

        let data = decode_account_data(&ui_account.data.0, &ui_account.data.1)
            .map_err(|e| anyhow!("Failed to decode fiber account data: {}", e))?;

        Ok(Account {
            lamports: ui_account.lamports,
            data,
            owner: ui_account.owner.parse().unwrap_or_default(),
            executable: ui_account.executable,
            rent_epoch: ui_account.rent_epoch,
        })
    }

    /// Fetch thread config with caching
    async fn fetch_thread_config(&self, config_pubkey: &Pubkey) -> Result<ThreadConfig> {
        // Try cache first
        if let Some(cached) = self.resources.cache.get(config_pubkey).await {
            if let Ok(config) = ThreadConfig::try_deserialize(&mut cached.data.as_slice()) {
                return Ok(config);
            }
        }

        // Fetch from RPC using custom client
        let ui_account = self
            .resources
            .rpc_client
            .get_account(config_pubkey)
            .await
            .map_err(|e| anyhow!("Failed to fetch thread config {}: {}", config_pubkey, e))?
            .ok_or_else(|| anyhow!("Thread config {} not found", config_pubkey))?;

        // Decode account data (supports base64 and base64+zstd)
        let account_data = decode_account_data(&ui_account.data.0, &ui_account.data.1)
            .map_err(|e| anyhow!("Failed to decode account data: {}", e))?;

        let config = ThreadConfig::try_deserialize(&mut account_data.as_slice())
            .map_err(|e| anyhow!("Failed to deserialize thread config: {}", e))?;

        // Cache it (unknown trigger type for config accounts)
        self.resources
            .cache
            .put_simple(*config_pubkey, account_data, 0)
            .await;

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    // Integration tests would require RPC connection
    // Unit tests for the module structure
    #[test]
    fn test_executor_logic_creation() {
        // Just verify the struct can be created
        // Full tests require RPC mocking
    }
}
