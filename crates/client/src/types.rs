//! Core types used throughout the client

use serde::{Deserialize, Serialize};
use solana_sdk::{clock::Clock, instruction::Instruction, pubkey::Pubkey};

/// Account update message sent from datasources to processor
#[derive(Debug, Clone)]
pub struct AccountUpdate {
    pub pubkey: Pubkey,
    pub data: Vec<u8>,
    pub slot: u64,
}

impl AccountUpdate {
    /// Create a new account update
    pub fn new(pubkey: Pubkey, data: Vec<u8>, slot: u64) -> Self {
        Self { pubkey, data, slot }
    }
}

/// Processor messages that can be sent to the submitter
#[derive(Clone, Debug)]
pub enum ProcessorMessage {
    /// Transaction to be submitted
    Transaction(TransactionMessage),
    /// Clock update from the blockchain
    Clock(Clock),
}

/// Message containing transaction details for submission
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransactionMessage {
    /// Instructions to execute
    pub instructions: Vec<Instruction>,
    /// Thread pubkey for context
    pub thread_pubkey: Pubkey,
    /// Executor pubkey for signing
    pub executor_pubkey: Pubkey,
    /// Optional priority fee
    pub priority_fee: Option<u64>,
    /// Optional compute units
    pub compute_units: Option<u32>,
}

/// Message containing durable transaction details
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DurableTransactionMessage {
    /// Instructions to execute
    pub instructions: Vec<Instruction>,
    /// Thread pubkey for context
    pub thread_pubkey: Pubkey,
    /// Executor pubkey for signing
    pub executor_pubkey: Pubkey,
    /// Nonce account pubkey
    pub nonce_pubkey: Pubkey,
    /// Optional priority fee
    pub priority_fee: Option<u64>,
    /// Optional compute units
    pub compute_units: Option<u32>,
    /// Original transaction signature for tracking
    pub original_signature: Option<String>,
    /// Retry count for replay attempts
    pub retry_count: u32,
    /// Base64 encoded transaction for replay
    pub base64_transaction: Option<String>,
    /// Timestamp when message was created
    pub created_at: std::time::SystemTime,
}

impl DurableTransactionMessage {
    /// Check if message has expired based on age
    pub fn is_expired_system_time(&self, max_age_ms: u64) -> bool {
        self.age_ms_system_time() > max_age_ms
    }

    /// Get age of message in milliseconds
    pub fn age_ms_system_time(&self) -> u64 {
        self.created_at.elapsed().unwrap_or_default().as_millis() as u64
    }
}
