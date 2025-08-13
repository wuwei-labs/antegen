use crate::*;
use anchor_lang::{prelude::*, AnchorDeserialize, AnchorSerialize};
use antegen_utils::thread::{Trigger, TriggerContext};

/// Current version of the Thread structure.
pub const CURRENT_THREAD_VERSION: u8 = 1;

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

    pub nonce_account: Pubkey,
    #[max_len(44)]
    pub last_nonce: String,

    /// The triggering event to kickoff a thread.
    pub trigger: Trigger,
    pub trigger_context: TriggerContext,
    
    /// Builder IDs currently building this thread
    #[max_len(10)]
    pub builders: Vec<u32>,
    /// When the first builder claimed this thread
    pub claim_window_start: Option<i64>,
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

    /// Clear all builders from the thread
    pub fn clear_builders(&mut self) {
        self.builders.clear();
        self.claim_window_start = None;
    }

    /// Clear expired builders from the thread
    pub fn clear_expired_builders(&mut self, current_timestamp: i64, claim_window_seconds: i64) {
        if let Some(start) = self.claim_window_start {
            if current_timestamp > start + claim_window_seconds {
                self.clear_builders();
            }
        }
    }

    /// Add a builder to the thread
    pub fn add_builder(&mut self, builder_id: u32, current_timestamp: i64) -> Result<()> {
        if self.builders.is_empty() {
            self.claim_window_start = Some(current_timestamp);
        }
        
        if !self.builders.contains(&builder_id) {
            self.builders.push(builder_id);
        }
        
        Ok(())
    }

    /// Check if a builder has claimed this thread
    pub fn has_builder(&self, builder_id: u32) -> bool {
        self.builders.contains(&builder_id)
    }

    /// Check if any builders have claimed this thread
    pub fn has_builders(&self) -> bool {
        !self.builders.is_empty()
    }
}

impl TryFrom<Vec<u8>> for Thread {
    type Error = Error;

    fn try_from(data: Vec<u8>) -> std::result::Result<Self, Self::Error> {
        Thread::try_deserialize(&mut data.as_slice())
    }
}
