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
    Now,

    /// Allows a thread to be kicked off according to a unix timestamp.
    Timestamp { unix_ts: i64 },

    /// Allows a thread to be kicked off at regular intervals.
    Interval {
        /// Interval in seconds between executions
        seconds: i64,
        /// Boolean value indicating whether triggering moments may be skipped
        skippable: bool,
    },

    /// Allows a thread to be kicked off according to a one-time or recurring schedule.
    Cron {
        /// The schedule in cron syntax. Value must be parsable by the `solana_cron` package.
        #[max_len(255)]
        schedule: String,

        /// Boolean value indicating whether triggering moments may be skipped if they are missed (e.g. due to network downtime).
        /// If false, any "missed" triggering moments will simply be executed as soon as the network comes back online.
        skippable: bool,
    },

    /// Allows a thread to be kicked off according to a slot.
    Slot { slot: u64 },

    /// Allows a thread to be kicked off according to an epoch number.
    Epoch { epoch: u64 },
}

/// The event which allowed a particular transaction thread to be triggered.
#[derive(AnchorDeserialize, AnchorSerialize, Clone, InitSpace, Debug)]
pub enum TriggerContext {
    /// A running hash of the observed account data.
    Account { hash: u64 },

    /// The trigger context for Now, Timestamp, Cron, and Interval
    Timestamp { prev: i64, next: i64 },

    /// The trigger context for Slot and Epoch
    Block { prev: u64, next: u64 },
}

/// A response value target programs can return to update the thread.
#[derive(AnchorDeserialize, AnchorSerialize, Clone, Debug)]
pub struct ThreadResponse {
    pub close_to: Option<Pubkey>,
    pub next_fiber: Option<u8>,
    pub trigger: Option<Trigger>,
    pub append_instruction: Option<SerializableInstruction>,
}

impl Default for ThreadResponse {
    fn default() -> Self {
        Self {
            close_to: None,
            next_fiber: None,
            trigger: None,
            append_instruction: None,
        }
    }
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
    /// The version of this thread structure, for migration purposes.
    pub version: u8,
    /// The bump, used for PDA validation.
    pub bump: u8,
    /// The owner of this thread.
    pub authority: Pubkey,
    /// The raw bytes to ensure 32 byte seed limit is maintained
    #[max_len(32)]
    pub id: Vec<u8>,
    /// The string representation of the id
    #[max_len(64)]
    pub name: String,
    pub created_at: i64,
    /// Whether or not the thread is currently paused.
    pub paused: bool,
    /// The instructions to be executed. (aka fibers)
    #[max_len(50)]
    pub fibers: Vec<u8>,
    pub exec_index: u8,
    /// Total number of executions across all fibers
    pub exec_count: u64,

    pub nonce_account: Pubkey,
    #[max_len(44)]
    pub last_nonce: String,

    /// The triggering event to kickoff a thread.
    pub trigger: Trigger,
    pub trigger_context: TriggerContext,

    /// The last executor who successfully executed this thread (for load balancing)
    pub last_executor: Pubkey,
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
    }

    /// Advance exec_index to the next fiber in the sequence.
    pub fn advance_to_next_fiber(&mut self) {
        if self.fibers.is_empty() {
            self.exec_index = 0;
            return;
        }

        // Find current index position in fibers vec
        if let Some(current_pos) = self.fibers.iter().position(|&x| x == self.exec_index) {
            // Move to next fiber, or wrap to beginning
            let next_pos = (current_pos + 1) % self.fibers.len();
            self.exec_index = self.fibers[next_pos];
        } else {
            // Current exec_index not found, reset to first fiber
            self.exec_index = self.fibers.first().copied().unwrap_or(0);
        }
    }

    /// Get the fiber PDA for the current exec_index
    pub fn fiber(&self, thread_pubkey: &Pubkey) -> Pubkey {
        self.fiber_at_index(thread_pubkey, self.exec_index)
    }

    /// Get the fiber PDA for a specific index
    pub fn fiber_at_index(&self, thread_pubkey: &Pubkey, index: u8) -> Pubkey {
        Pubkey::find_program_address(
            &[b"thread_fiber", thread_pubkey.as_ref(), &[index]],
            &crate::ID,
        )
        .0
    }

    /// Get the next fiber PDA (for the next exec_index in the sequence)
    pub fn next_fiber(&self, thread_pubkey: &Pubkey) -> Pubkey {
        // Calculate next index based on fibers sequence
        let next_index = if self.fibers.is_empty() {
            0
        } else if let Some(current_pos) = self.fibers.iter().position(|&x| x == self.exec_index) {
            let next_pos = (current_pos + 1) % self.fibers.len();
            self.fibers[next_pos]
        } else {
            self.fibers.first().copied().unwrap_or(0)
        };

        self.fiber_at_index(thread_pubkey, next_index)
    }

    /// Check if thread is ready to execute based on trigger context
    pub fn is_ready(&self, current_slot: u64, current_timestamp: i64) -> bool {
        match &self.trigger_context {
            TriggerContext::Timestamp { next, .. } => current_timestamp >= *next,
            TriggerContext::Block { next, .. } => {
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
            TriggerContext::Account { .. } => {
                // Account triggers are handled by the observer
                false
            }
        }
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

/// Trait for processing trigger validation and context updates
pub trait TriggerProcessor {
    fn validate_and_update_context(
        &mut self,
        clock: &Clock,
        remaining_accounts: &[AccountInfo],
    ) -> Result<i64>; // Returns time_since_ready (elapsed time since trigger was ready)
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
    fn validate_and_update_context(
        &mut self,
        clock: &Clock,
        remaining_accounts: &[AccountInfo],
    ) -> Result<i64> {
        let last_started_at = self.get_last_started_at();

        // Determine trigger ready time and validate
        let trigger_ready_time = match &self.trigger {
            Trigger::Now => {
                self.trigger_context = TriggerContext::Timestamp {
                    prev: last_started_at,
                    next: clock.unix_timestamp,
                };
                clock.unix_timestamp
            }

            Trigger::Timestamp { unix_ts } => {
                require!(
                    clock.unix_timestamp >= *unix_ts,
                    AntegenThreadError::TriggerConditionFailed
                );
                self.trigger_context = TriggerContext::Timestamp {
                    prev: last_started_at,
                    next: *unix_ts,
                };
                *unix_ts
            }

            Trigger::Slot { slot } => {
                require!(
                    clock.slot >= *slot,
                    AntegenThreadError::TriggerConditionFailed
                );
                self.trigger_context = TriggerContext::Block {
                    prev: last_started_at as u64,
                    next: *slot,
                };
                // Approximate when slot was reached (assuming 400ms per slot)
                clock.unix_timestamp - ((clock.slot - slot) as i64 * 400 / 1000)
            }

            Trigger::Epoch { epoch } => {
                require!(
                    clock.epoch >= *epoch,
                    AntegenThreadError::TriggerConditionFailed
                );
                self.trigger_context = TriggerContext::Block {
                    prev: last_started_at as u64,
                    next: *epoch,
                };
                clock.unix_timestamp
            }

            Trigger::Interval { seconds, skippable } => {
                let next_timestamp = last_started_at.saturating_add(*seconds);
                require!(
                    clock.unix_timestamp >= next_timestamp,
                    AntegenThreadError::TriggerConditionFailed
                );

                let started_at = if *skippable {
                    clock.unix_timestamp
                } else {
                    next_timestamp
                };

                self.trigger_context = TriggerContext::Timestamp {
                    prev: started_at,
                    next: started_at.saturating_add(*seconds),
                };
                next_timestamp
            }

            Trigger::Cron {
                schedule,
                skippable,
            } => {
                let threshold_timestamp =
                    crate::utils::next_timestamp(last_started_at, schedule.clone())
                        .ok_or(AntegenThreadError::TriggerConditionFailed)?;

                require!(
                    clock.unix_timestamp >= threshold_timestamp,
                    AntegenThreadError::TriggerConditionFailed
                );

                let started_at = if *skippable {
                    clock.unix_timestamp
                } else {
                    threshold_timestamp
                };

                self.trigger_context = TriggerContext::Timestamp {
                    prev: last_started_at,
                    next: started_at,
                };
                threshold_timestamp
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
                if let TriggerContext::Account { hash: prior_hash } = &self.trigger_context {
                    require!(
                        data_hash.ne(prior_hash),
                        AntegenThreadError::TriggerConditionFailed
                    );
                }

                self.trigger_context = TriggerContext::Account { hash: data_hash };
                clock.unix_timestamp
            }
        };

        // Return elapsed time since trigger was ready
        Ok(clock.unix_timestamp.saturating_sub(trigger_ready_time))
    }

    fn get_last_started_at(&self) -> i64 {
        match &self.trigger_context {
            TriggerContext::Timestamp { prev, .. } => *prev,
            TriggerContext::Block { prev, .. } => *prev as i64,
            TriggerContext::Account { .. } => self.created_at,
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
