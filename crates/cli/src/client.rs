use anchor_lang::prelude::Clock;
use solana_client::{client_error, rpc_client::RpcClient};
use solana_sdk::{
    commitment_config::CommitmentConfig,
    hash::Hash,
    instruction::Instruction,
    message::{v0, VersionedMessage},
    program_error::ProgramError,
    pubkey::Pubkey,
    signature::{Keypair, Signature, Signer},
    signers::Signers,
    transaction::VersionedTransaction,
};
use std::{
    fmt::Debug,
    ops::{Deref, DerefMut},
    str::FromStr,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error(transparent)]
    Client(#[from] client_error::ClientError),

    #[error(transparent)]
    Program(#[from] ProgramError),

    #[error("Failed to deserialize account data")]
    DeserializationError,
}

pub type ClientResult<T> = Result<T, ClientError>;

pub struct Client {
    pub client: RpcClient,
    pub payer: Keypair,
}

impl Client {
    pub fn new(payer: Keypair, url: String) -> Self {
        let client = RpcClient::new_with_commitment::<String>(url, CommitmentConfig::processed());
        Self { client, payer }
    }

    pub fn get_clock(&self) -> ClientResult<Clock> {
        let clock_pubkey = Pubkey::from_str("SysvarC1ock11111111111111111111111111111111").unwrap();
        let clock_data = self.client.get_account_data(&clock_pubkey)?;
        bincode::deserialize::<Clock>(&clock_data).map_err(|_| ClientError::DeserializationError)
    }

    pub fn payer(&self) -> &Keypair {
        &self.payer
    }

    pub fn payer_pubkey(&self) -> Pubkey {
        self.payer.pubkey()
    }

    pub fn latest_blockhash(&self) -> ClientResult<Hash> {
        Ok(self.client.get_latest_blockhash()?)
    }

    pub fn send_and_confirm<T: Signers>(
        &self,
        ixs: &[Instruction],
        signers: &T,
    ) -> ClientResult<Signature> {
        let tx = self.transaction(ixs, signers)?;
        Ok(self.send_and_confirm_transaction(&tx)?)
    }

    fn transaction<T: Signers>(
        &self,
        ixs: &[Instruction],
        signers: &T,
    ) -> ClientResult<VersionedTransaction> {
        let blockhash = self.latest_blockhash()?;
        
        // Build v0 message
        let message = v0::Message::try_compile(
            &self.payer_pubkey(),
            ixs,
            &[], // No lookup tables for now
            blockhash,
        ).map_err(|e| ClientError::Client(
            client_error::ClientError::from(
                std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
            )
        ))?;
        
        // Create versioned transaction
        let tx = VersionedTransaction::try_new(
            VersionedMessage::V0(message),
            signers,
        ).map_err(|e| ClientError::Client(
            client_error::ClientError::from(
                std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
            )
        ))?;
        
        Ok(tx)
    }
}

impl Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RPC client payer {}", self.payer_pubkey())
    }
}

impl Deref for Client {
    type Target = RpcClient;

    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

impl DerefMut for Client {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.client
    }
}
