use crate::{errors::AntegenThreadError, *};
use anchor_lang::{
    prelude::*,
    solana_program::instruction::{AccountMeta, Instruction},
    AnchorDeserialize, AnchorSerialize,
};
use std::{collections::hash_map::DefaultHasher, collections::HashMap, hash::Hasher};

/// Current version of the Thread structure.
pub const CURRENT_THREAD_VERSION: u8 = 1;

/// Static pubkey for the payer placeholder - this is a placeholder address
/// "AntegenPayer1111111111111111111111111111111" in base58  
pub const PAYER_PUBKEY: Pubkey = pubkey!("AntegenPayer1111111111111111111111111111111");

/// Serializable version of Solana's Instruction for easier handling
#[derive(AnchorDeserialize, AnchorSerialize, Clone, Debug)]
pub struct SerializableInstruction {
    pub program_id: Pubkey,
    pub accounts: Vec<SerializableAccountMeta>,
    pub data: Vec<u8>,
}

/// Serializable version of AccountMeta
#[derive(AnchorDeserialize, AnchorSerialize, Clone, Debug)]
pub struct SerializableAccountMeta {
    pub pubkey: Pubkey,
    pub is_signer: bool,
    pub is_writable: bool,
}

impl From<Instruction> for SerializableInstruction {
    fn from(ix: Instruction) -> Self {
        SerializableInstruction {
            program_id: ix.program_id,
            accounts: ix
                .accounts
                .into_iter()
                .map(|acc| SerializableAccountMeta {
                    pubkey: acc.pubkey,
                    is_signer: acc.is_signer,
                    is_writable: acc.is_writable,
                })
                .collect(),
            data: ix.data,
        }
    }
}

impl From<SerializableInstruction> for Instruction {
    fn from(ix: SerializableInstruction) -> Self {
        Instruction {
            program_id: ix.program_id,
            accounts: ix
                .accounts
                .into_iter()
                .map(|acc| AccountMeta {
                    pubkey: acc.pubkey,
                    is_signer: acc.is_signer,
                    is_writable: acc.is_writable,
                })
                .collect(),
            data: ix.data,
        }
    }
}

/// The triggering conditions of a thread.
#[derive(AnchorDeserialize, AnchorSerialize, Clone, InitSpace, PartialEq, Debug)]
pub enum Trigger {
    /// Allows a thread to be kicked off whenever the data of an account changes.
    Account {
        /// The address of the account to monitor.
        address: Pubkey,
        /// The byte offset of the account data to monitor.
        offset: u64,
        /// The size of the byte slice to monitor (must be less than 1kb)
        size: u64,
    },

    /// Allows a thread to be kicked off as soon as it's created.
    Immediate {
        /// Optional jitter in seconds to prevent thundering herd (0 = no jitter)
        jitter: u64,
    },

    /// Allows a thread to be kicked off according to a unix timestamp.
    Timestamp {
        unix_ts: i64,
        /// Optional jitter in seconds to spread execution across a window (0 = no jitter)
        jitter: u64,
    },

    /// Allows a thread to be kicked off at regular intervals.
    Interval {
        /// Interval in seconds between executions
        seconds: i64,
        /// Boolean value indicating whether triggering moments may be skipped
        skippable: bool,
        /// Optional jitter in seconds to prevent thundering herd (0 = no jitter)
        jitter: u64,
    },

    /// Allows a thread to be kicked off according to a one-time or recurring schedule.
    Cron {
        /// The schedule in cron syntax. Value must be parsable by the `antegen_cron` package.
        #[max_len(255)]
        schedule: String,

        /// Boolean value indicating whether triggering moments may be skipped if they are missed (e.g. due to network downtime).
        /// If false, any "missed" triggering moments will simply be executed as soon as the network comes back online.
        skippable: bool,

        /// Optional jitter in seconds to spread execution across a window (0 = no jitter)
        jitter: u64,
    },

    /// Allows a thread to be kicked off according to a slot.
    Slot { slot: u64 },

    /// Allows a thread to be kicked off according to an epoch number.
    Epoch { epoch: u64 },
}

/// Tracks the execution schedule - when the thread last ran and when it should run next
/// (was: TriggerContext)
#[derive(AnchorDeserialize, AnchorSerialize, Clone, InitSpace, Debug, PartialEq)]
pub enum Schedule {
    /// For Account triggers - tracks data hash for change detection
    OnChange { prev: u64 },

    /// For time-based triggers (Immediate, Timestamp, Interval, Cron)
    Timed { prev: i64, next: i64 },

    /// For block-based triggers (Slot, Epoch)
    Block { prev: u64, next: u64 },
}

/// Signal from a fiber about what should happen after execution.
/// Emitted via set_return_data(), received by thread program via get_return_data().
#[derive(AnchorDeserialize, AnchorSerialize, Clone, Default, InitSpace, Debug, PartialEq)]
pub enum Signal {
    #[default]
    None, // No signal - normal execution flow
    Chain,  // Chain to next fiber (same tx)
    Close,  // Chain to delete thread (same tx)
    Repeat, // Repeat this fiber on next trigger (skip cursor advancement)
    Next {
        index: u8, // Set specific fiber to execute on next trigger
    },
    UpdateTrigger {
        trigger: Trigger, // Update the thread's trigger
    },
}

/// Compiled instruction data for space-efficient storage
#[derive(AnchorDeserialize, AnchorSerialize, Clone, Debug)]
pub struct CompiledInstructionData {
    pub program_id_index: u8,
    pub accounts: Vec<u8>,
    pub data: Vec<u8>,
}

/// Compiled instruction containing deduplicated accounts
#[derive(AnchorDeserialize, AnchorSerialize, Clone, Debug)]
pub struct CompiledInstructionV0 {
    pub num_ro_signers: u8,
    pub num_rw_signers: u8,
    pub num_rw: u8,
    pub instructions: Vec<CompiledInstructionData>,
    pub signer_seeds: Vec<Vec<Vec<u8>>>,
    pub accounts: Vec<Pubkey>,
}

/// Tracks the current state of a transaction thread on Solana.
#[account]
#[derive(Debug, InitSpace)]
pub struct Thread {
    // Identity
    pub version: u8,
    pub bump: u8,
    pub authority: Pubkey,
    #[max_len(32)]
    pub id: Vec<u8>,
    #[max_len(64)]
    pub name: String,
    pub created_at: i64,

    // Scheduling
    pub trigger: Trigger,
    pub schedule: Schedule,

    // Default fiber (index 0, stored inline)
    #[max_len(1024)]
    pub default_fiber: Option<Vec<u8>>,
    pub default_fiber_priority_fee: u64,

    // Fibers
    #[max_len(50)]
    pub fiber_ids: Vec<u8>,
    pub fiber_cursor: u8,
    pub fiber_next_id: u8,
    pub fiber_signal: Signal,

    // Lifecycle
    pub paused: bool,

    // Execution tracking
    pub exec_count: u64,
    pub last_executor: Pubkey,
    pub last_error_time: Option<i64>,

    // Nonce (for durable transactions)
    pub nonce_account: Pubkey,
    #[max_len(44)]
    pub last_nonce: String,

    // Pre-compiled thread_delete instruction for self-closing
    #[max_len(256)]
    pub close_fiber: Vec<u8>,
}

impl Thread {
    /// Derive the pubkey of a thread account.
    pub fn pubkey(authority: Pubkey, id: impl AsRef<[u8]>) -> Pubkey {
        let id_bytes = id.as_ref();
        assert!(id_bytes.len() <= 32, "Thread ID must not exceed 32 bytes");

        Pubkey::find_program_address(&[SEED_THREAD, authority.as_ref(), id_bytes], &crate::ID).0
    }

    /// Check if this thread has a nonce account.
    pub fn has_nonce_account(&self) -> bool {
        self.nonce_account != anchor_lang::solana_program::system_program::ID
            && self.nonce_account != crate::ID
    }

    /// Advance fiber_cursor to the next fiber in the sequence.
    pub fn advance_to_next_fiber(&mut self) {
        if self.fiber_ids.is_empty() {
            self.fiber_cursor = 0;
            return;
        }

        // Find current index position in fiber_ids vec
        if let Some(current_pos) = self.fiber_ids.iter().position(|&x| x == self.fiber_cursor) {
            // Move to next fiber, or wrap to beginning
            let next_pos = (current_pos + 1) % self.fiber_ids.len();
            self.fiber_cursor = self.fiber_ids[next_pos];
        } else {
            // Current fiber_cursor not found, reset to first fiber
            self.fiber_cursor = self.fiber_ids.first().copied().unwrap_or(0);
        }
    }

    /// Get the next fiber index in sequence (without mutating).
    /// Used to validate Chain signals target the correct consecutive fiber.
    pub fn next_fiber_index(&self) -> u8 {
        if self.fiber_ids.is_empty() {
            return 0;
        }
        if let Some(current_pos) = self.fiber_ids.iter().position(|&x| x == self.fiber_cursor) {
            let next_pos = (current_pos + 1) % self.fiber_ids.len();
            self.fiber_ids[next_pos]
        } else {
            self.fiber_ids.first().copied().unwrap_or(0)
        }
    }

    /// Get the fiber PDA for the current fiber_cursor
    pub fn fiber(&self, thread_pubkey: &Pubkey) -> Pubkey {
        self.fiber_at_index(thread_pubkey, self.fiber_cursor)
    }

    /// Get the fiber PDA for a specific fiber_index
    pub fn fiber_at_index(&self, thread_pubkey: &Pubkey, fiber_index: u8) -> Pubkey {
        Pubkey::find_program_address(
            &[b"thread_fiber", thread_pubkey.as_ref(), &[fiber_index]],
            &crate::ID,
        )
        .0
    }

    /// Get the next fiber PDA (for the next fiber_cursor in the sequence)
    pub fn next_fiber(&self, thread_pubkey: &Pubkey) -> Pubkey {
        // Calculate next index based on fiber_ids sequence
        let next_index = if self.fiber_ids.is_empty() {
            0
        } else if let Some(current_pos) =
            self.fiber_ids.iter().position(|&x| x == self.fiber_cursor)
        {
            let next_pos = (current_pos + 1) % self.fiber_ids.len();
            self.fiber_ids[next_pos]
        } else {
            self.fiber_ids.first().copied().unwrap_or(0)
        };

        self.fiber_at_index(thread_pubkey, next_index)
    }

    /// Check if thread is ready to execute based on schedule
    pub fn is_ready(&self, current_slot: u64, current_timestamp: i64) -> bool {
        match &self.schedule {
            Schedule::Timed { next, .. } => current_timestamp >= *next,
            Schedule::Block { next, .. } => {
                match &self.trigger {
                    Trigger::Slot { .. } => current_slot >= *next,
                    Trigger::Epoch { .. } => {
                        // For epoch triggers, we'd need epoch info
                        // This is a simplified check
                        false
                    }
                    _ => false,
                }
            }
            Schedule::OnChange { .. } => {
                // Account triggers are handled by the observer
                false
            }
        }
    }

    /// Validate that the thread is ready for execution
    pub fn validate_for_execution(&self) -> Result<()> {
        // Check that thread has fibers
        require!(
            !self.fiber_ids.is_empty(),
            crate::errors::AntegenThreadError::ThreadHasNoFibersToExecute
        );

        // Check that fiber_cursor is valid
        if self.fiber_cursor == 0 {
            // For index 0, either default fiber must exist OR it must be in fiber_ids vec
            require!(
                self.default_fiber.is_some() || self.fiber_ids.contains(&0),
                crate::errors::AntegenThreadError::InvalidExecIndex
            );
        } else {
            // For other indices, must exist in fiber_ids vector
            require!(
                self.fiber_ids.contains(&self.fiber_cursor),
                crate::errors::AntegenThreadError::InvalidExecIndex
            );
        }

        Ok(())
    }
}

impl TryFrom<Vec<u8>> for Thread {
    type Error = Error;

    fn try_from(data: Vec<u8>) -> std::result::Result<Self, Self::Error> {
        Thread::try_deserialize(&mut data.as_slice())
    }
}

/// Compile an instruction into a space-efficient format
pub fn compile_instruction(
    instruction: Instruction,
    signer_seeds: Vec<Vec<Vec<u8>>>,
) -> Result<CompiledInstructionV0> {
    let mut pubkeys_to_metadata: HashMap<Pubkey, AccountMeta> = HashMap::new();

    // Add program ID
    pubkeys_to_metadata.insert(
        instruction.program_id,
        AccountMeta {
            pubkey: instruction.program_id,
            is_signer: false,
            is_writable: false,
        },
    );

    // Process accounts
    for acc in &instruction.accounts {
        let entry = pubkeys_to_metadata
            .entry(acc.pubkey)
            .or_insert(AccountMeta {
                pubkey: acc.pubkey,
                is_signer: false,
                is_writable: false,
            });
        entry.is_signer |= acc.is_signer;
        entry.is_writable |= acc.is_writable;
    }

    // Sort accounts by priority
    let mut sorted_accounts: Vec<Pubkey> = pubkeys_to_metadata.keys().cloned().collect();
    sorted_accounts.sort_by(|a, b| {
        let a_meta = &pubkeys_to_metadata[a];
        let b_meta = &pubkeys_to_metadata[b];

        fn get_priority(meta: &AccountMeta) -> u8 {
            match (meta.is_signer, meta.is_writable) {
                (true, true) => 0,
                (true, false) => 1,
                (false, true) => 2,
                (false, false) => 3,
            }
        }

        get_priority(a_meta).cmp(&get_priority(b_meta))
    });

    // Count account types
    let mut num_rw_signers = 0u8;
    let mut num_ro_signers = 0u8;
    let mut num_rw = 0u8;

    for pubkey in &sorted_accounts {
        let meta = &pubkeys_to_metadata[pubkey];
        if meta.is_signer && meta.is_writable {
            num_rw_signers += 1;
        } else if meta.is_signer && !meta.is_writable {
            num_ro_signers += 1;
        } else if meta.is_writable {
            num_rw += 1;
        }
    }

    // Create index mapping
    let accounts_to_index: HashMap<Pubkey, u8> = sorted_accounts
        .iter()
        .enumerate()
        .map(|(i, k)| (*k, i as u8))
        .collect();

    // Create compiled instruction
    let compiled_instruction = CompiledInstructionData {
        program_id_index: *accounts_to_index.get(&instruction.program_id).unwrap(),
        accounts: instruction
            .accounts
            .iter()
            .map(|acc| *accounts_to_index.get(&acc.pubkey).unwrap())
            .collect(),
        data: instruction.data,
    };

    Ok(CompiledInstructionV0 {
        num_ro_signers,
        num_rw_signers,
        num_rw,
        instructions: vec![compiled_instruction],
        signer_seeds,
        accounts: sorted_accounts,
    })
}

/// Decompile a compiled instruction back to a regular instruction
pub fn decompile_instruction(compiled: &CompiledInstructionV0) -> Result<Instruction> {
    if compiled.instructions.is_empty() {
        return Err(ProgramError::InvalidInstructionData.into());
    }

    let ix = &compiled.instructions[0];
    let program_id = compiled.accounts[ix.program_id_index as usize];

    let accounts: Vec<AccountMeta> = ix
        .accounts
        .iter()
        .enumerate()
        .map(|(_i, &idx)| {
            let pubkey = compiled.accounts[idx as usize];
            let is_writable = if idx < compiled.num_rw_signers {
                true
            } else if idx < compiled.num_rw_signers + compiled.num_ro_signers {
                false
            } else if idx < compiled.num_rw_signers + compiled.num_ro_signers + compiled.num_rw {
                true
            } else {
                false
            };
            let is_signer = idx < compiled.num_rw_signers + compiled.num_ro_signers;

            AccountMeta {
                pubkey,
                is_signer,
                is_writable,
            }
        })
        .collect();

    Ok(Instruction {
        program_id,
        accounts,
        data: ix.data.clone(),
    })
}

/// Trait for processing trigger validation and schedule updates
pub trait TriggerProcessor {
    fn validate_trigger(
        &self,
        clock: &Clock,
        remaining_accounts: &[AccountInfo],
        thread_pubkey: &Pubkey,
    ) -> Result<i64>; // Returns time_since_ready (elapsed time since trigger was ready)

    fn update_schedule(
        &mut self,
        clock: &Clock,
        remaining_accounts: &[AccountInfo],
        thread_pubkey: &Pubkey,
    ) -> Result<()>; // Updates schedule for next execution

    fn get_last_started_at(&self) -> i64;
}

/// Trait for getting thread seeds for signing  
pub trait ThreadSeeds {
    fn get_seed_bytes(&self) -> Vec<Vec<u8>>;

    /// Use seeds with a callback to avoid lifetime issues
    fn sign<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&[&[u8]]) -> R,
    {
        let seed_bytes = self.get_seed_bytes();
        let seeds: Vec<&[u8]> = seed_bytes.iter().map(|s| s.as_slice()).collect();
        f(&seeds)
    }
}

/// Trait for handling nonce account operations
pub trait NonceProcessor {
    fn advance_nonce_if_required<'info>(
        &self,
        thread_account_info: &AccountInfo<'info>,
        nonce_account: &Option<UncheckedAccount<'info>>,
        recent_blockhashes: &Option<UncheckedAccount<'info>>,
    ) -> Result<()>;
}

/// Trait for distributing payments
pub trait PaymentDistributor {
    fn distribute_payments<'info>(
        &self,
        thread_account: &AccountInfo<'info>,
        executor: &AccountInfo<'info>,
        admin: &AccountInfo<'info>,
        payments: &crate::state::PaymentDetails,
    ) -> Result<()>;
}

impl TriggerProcessor for Thread {
    fn validate_trigger(
        &self,
        clock: &Clock,
        remaining_accounts: &[AccountInfo],
        thread_pubkey: &Pubkey,
    ) -> Result<i64> {
        let last_started_at = self.get_last_started_at();

        // Determine trigger ready time and validate
        let trigger_ready_time = match &self.trigger {
            Trigger::Immediate { jitter } => {
                let jitter_offset =
                    crate::utils::calculate_jitter_offset(last_started_at, thread_pubkey, *jitter);
                clock.unix_timestamp.saturating_add(jitter_offset)
            }

            Trigger::Timestamp { unix_ts, jitter } => {
                let jitter_offset =
                    crate::utils::calculate_jitter_offset(last_started_at, thread_pubkey, *jitter);
                let trigger_time = unix_ts.saturating_add(jitter_offset);

                require!(
                    clock.unix_timestamp >= trigger_time,
                    AntegenThreadError::TriggerConditionFailed
                );
                trigger_time
            }

            Trigger::Slot { slot } => {
                require!(
                    clock.slot >= *slot,
                    AntegenThreadError::TriggerConditionFailed
                );
                // Approximate when slot was reached (assuming 400ms per slot)
                clock.unix_timestamp - ((clock.slot - slot) as i64 * 400 / 1000)
            }

            Trigger::Epoch { epoch } => {
                require!(
                    clock.epoch >= *epoch,
                    AntegenThreadError::TriggerConditionFailed
                );
                clock.unix_timestamp
            }

            Trigger::Interval {
                seconds: _,
                skippable: _,
                jitter: _,
            } => {
                // schedule.next already has jitter baked in from previous execution
                let trigger_time = match self.schedule {
                    Schedule::Timed { next, .. } => next,
                    _ => return Err(AntegenThreadError::TriggerConditionFailed.into()),
                };

                require!(
                    clock.unix_timestamp >= trigger_time,
                    AntegenThreadError::TriggerConditionFailed
                );
                trigger_time
            }

            Trigger::Cron {
                schedule: _,
                skippable: _,
                jitter: _,
            } => {
                // schedule.next already has jitter baked in from previous execution
                let trigger_time = match self.schedule {
                    Schedule::Timed { next, .. } => next,
                    _ => return Err(AntegenThreadError::TriggerConditionFailed.into()),
                };

                require!(
                    clock.unix_timestamp >= trigger_time,
                    AntegenThreadError::TriggerConditionFailed
                );
                trigger_time
            }

            Trigger::Account {
                address,
                offset,
                size,
            } => {
                // Verify proof account is provided
                let account_info = remaining_accounts
                    .first()
                    .ok_or(AntegenThreadError::TriggerConditionFailed)?;

                // Verify it's the correct account
                require!(
                    address.eq(account_info.key),
                    AntegenThreadError::TriggerConditionFailed
                );

                // Compute data hash
                let mut hasher = DefaultHasher::new();
                let data = &account_info.try_borrow_data()?;
                let offset = *offset as usize;
                let range_end = offset.checked_add(*size as usize).unwrap() as usize;

                use std::hash::Hash;
                if data.len() > range_end {
                    data[offset..range_end].hash(&mut hasher);
                } else {
                    data[offset..].hash(&mut hasher);
                }
                let data_hash = hasher.finish();

                // Verify hash changed
                if let Schedule::OnChange { prev: prior_hash } = &self.schedule {
                    require!(
                        data_hash.ne(prior_hash),
                        AntegenThreadError::TriggerConditionFailed
                    );
                }

                clock.unix_timestamp
            }
        };

        // Return elapsed time since trigger was ready
        Ok(clock.unix_timestamp.saturating_sub(trigger_ready_time))
    }

    fn update_schedule(
        &mut self,
        clock: &Clock,
        remaining_accounts: &[AccountInfo],
        thread_pubkey: &Pubkey,
    ) -> Result<()> {
        let current_timestamp = clock.unix_timestamp;

        self.schedule = match &self.trigger {
            Trigger::Account { offset, size, .. } => {
                // Compute data hash for Account trigger
                let account_info = remaining_accounts
                    .first()
                    .ok_or(AntegenThreadError::TriggerConditionFailed)?;

                let mut hasher = DefaultHasher::new();
                let data = &account_info.try_borrow_data()?;
                let offset = *offset as usize;
                let range_end = offset.checked_add(*size as usize).unwrap() as usize;

                use std::hash::Hash;
                if data.len() > range_end {
                    data[offset..range_end].hash(&mut hasher);
                } else {
                    data[offset..].hash(&mut hasher);
                }
                let data_hash = hasher.finish();

                Schedule::OnChange { prev: data_hash }
            }
            Trigger::Cron {
                schedule, jitter, ..
            } => {
                // Calculate next cron time WITH jitter baked in
                // Use current_timestamp since this is called right after execution
                let next_cron = crate::utils::next_timestamp(current_timestamp, schedule.clone())
                    .ok_or(AntegenThreadError::TriggerConditionFailed)?;
                let next_jitter = crate::utils::calculate_jitter_offset(
                    current_timestamp,
                    thread_pubkey,
                    *jitter,
                );
                let next_trigger_time = next_cron.saturating_add(next_jitter);

                Schedule::Timed {
                    prev: current_timestamp,
                    next: next_trigger_time,
                }
            }
            Trigger::Immediate { .. } => Schedule::Timed {
                prev: current_timestamp,
                next: 0, // Use 0 instead of i64::MAX to avoid JSON serialization issues
            },
            Trigger::Slot { slot } => Schedule::Block {
                prev: clock.slot,
                next: *slot,
            },
            Trigger::Epoch { epoch } => Schedule::Block {
                prev: clock.epoch,
                next: *epoch,
            },
            Trigger::Interval {
                seconds, jitter, ..
            } => {
                // Calculate next trigger time WITH jitter baked in
                // Use current_timestamp since this is called right after execution
                let next_base = current_timestamp.saturating_add(*seconds);
                let next_jitter = crate::utils::calculate_jitter_offset(
                    current_timestamp,
                    thread_pubkey,
                    *jitter,
                );
                let next_trigger_time = next_base.saturating_add(next_jitter);

                Schedule::Timed {
                    prev: current_timestamp,
                    next: next_trigger_time,
                }
            }
            Trigger::Timestamp { unix_ts, .. } => Schedule::Timed {
                prev: current_timestamp,
                next: *unix_ts,
            },
        };

        Ok(())
    }

    fn get_last_started_at(&self) -> i64 {
        match &self.schedule {
            Schedule::Timed { prev, .. } => *prev,
            Schedule::Block { prev, .. } => *prev as i64,
            Schedule::OnChange { .. } => self.created_at,
        }
    }
}

impl ThreadSeeds for Thread {
    fn get_seed_bytes(&self) -> Vec<Vec<u8>> {
        vec![
            SEED_THREAD.to_vec(),
            self.authority.to_bytes().to_vec(),
            self.id.clone(),
            vec![self.bump],
        ]
    }
}

impl PaymentDistributor for Thread {
    fn distribute_payments<'info>(
        &self,
        thread_account: &AccountInfo<'info>,
        executor: &AccountInfo<'info>,
        admin: &AccountInfo<'info>,
        payments: &crate::state::PaymentDetails,
    ) -> Result<()> {
        use crate::utils::transfer_lamports;

        // Combined payment to executor (reimbursement + commission)
        let total_executor_payment =
            payments.fee_payer_reimbursement + payments.executor_commission;

        // Log all payments in one line for conciseness
        if total_executor_payment > 0 || payments.core_team_fee > 0 {
            msg!(
                "Payments: executor {} (reimburse {}, commission {}), team {}",
                total_executor_payment,
                payments.fee_payer_reimbursement,
                payments.executor_commission,
                payments.core_team_fee
            );
        }

        if total_executor_payment > 0 {
            transfer_lamports(thread_account, executor, total_executor_payment)?;
        }

        // Transfer core team fee to admin
        if payments.core_team_fee > 0 {
            transfer_lamports(thread_account, admin, payments.core_team_fee)?;
        }

        Ok(())
    }
}

impl NonceProcessor for Thread {
    fn advance_nonce_if_required<'info>(
        &self,
        thread_account_info: &AccountInfo<'info>,
        nonce_account: &Option<UncheckedAccount<'info>>,
        recent_blockhashes: &Option<UncheckedAccount<'info>>,
    ) -> Result<()> {
        use anchor_lang::solana_program::{
            program::invoke_signed, system_instruction::advance_nonce_account,
        };

        if !self.has_nonce_account() {
            return Ok(());
        }

        match (nonce_account, recent_blockhashes) {
            (Some(nonce_acc), Some(recent_bh)) => {
                // Get thread key from account info
                let thread_key = *thread_account_info.key;

                // Use seeds with callback to handle invoke_signed
                self.sign(|seeds| {
                    invoke_signed(
                        &advance_nonce_account(&nonce_acc.key(), &thread_key),
                        &[
                            nonce_acc.to_account_info(),
                            recent_bh.to_account_info(),
                            thread_account_info.clone(),
                        ],
                        &[seeds],
                    )
                })?;
                Ok(())
            }
            _ => Err(AntegenThreadError::NonceRequired.into()),
        }
    }
}
