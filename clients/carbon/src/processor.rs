use async_trait::async_trait;
use carbon_core::{
    account::{AccountProcessorInputType, DecodedAccount},
    error::CarbonResult,
    metrics::MetricsCollection,
    processor::Processor,
};
use crossbeam::channel::Sender;
use log::{debug, error};
use solana_sdk::{account::Account, pubkey::Pubkey};
use std::sync::Arc;

use antegen_adapter::events::ObservedEvent;

/// Processor that converts Carbon account updates to Antegen ObservedEvents
pub struct ThreadAccountProcessor {
    /// Channel to send events to the adapter
    sender: Sender<ObservedEvent>,
    /// Thread program ID to filter accounts
    thread_program_id: Pubkey,
}

impl ThreadAccountProcessor {
    pub fn new(sender: Sender<ObservedEvent>, thread_program_id: Pubkey) -> Self {
        Self {
            sender,
            thread_program_id,
        }
    }
}

#[async_trait]
impl Processor for ThreadAccountProcessor {
    type InputType = AccountProcessorInputType<Vec<u8>>;

    async fn process(
        &mut self,
        input: Self::InputType,
        _metrics: Arc<MetricsCollection>,
    ) -> CarbonResult<()> {
        let (metadata, decoded_account, _raw_account) = input;

        // Extract account information
        let pubkey = metadata.pubkey;
        let slot = metadata.slot;

        // Convert Carbon's decoded account to Solana SDK Account
        let account = Account {
            lamports: decoded_account.lamports,
            data: decoded_account.data,
            owner: decoded_account.owner,
            executable: decoded_account.executable,
            rent_epoch: decoded_account.rent_epoch,
        };

        // Only process accounts owned by the thread program
        if account.owner != self.thread_program_id {
            debug!(
                "Skipping account {} - not owned by thread program (owner: {})",
                pubkey, account.owner
            );
            return Ok(());
        }

        // Create ObservedEvent
        let event = ObservedEvent::Account {
            pubkey,
            account,
            slot,
        };

        // Send event to adapter
        if let Err(e) = self.sender.send(event) {
            error!("Failed to send event to adapter: {}", e);
            return Err(carbon_core::error::Error::Custom(format!(
                "Failed to send event: {}",
                e
            )));
        }

        debug!("Sent account update for {} at slot {}", pubkey, slot);

        Ok(())
    }
}

/// Simple decoder that passes through raw account data
pub struct BasicAccountDecoder;

impl<'a> carbon_core::account::AccountDecoder<'a> for BasicAccountDecoder {
    type AccountType = Vec<u8>;

    fn decode_account(
        &self,
        account: &'a solana_account::Account,
    ) -> Option<DecodedAccount<Self::AccountType>> {
        Some(DecodedAccount {
            lamports: account.lamports,
            data: account.data.clone(),
            owner: account.owner,
            executable: account.executable,
            rent_epoch: account.rent_epoch,
        })
    }
}
