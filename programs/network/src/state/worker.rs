use anchor_lang::{prelude::*, AnchorDeserialize};

use crate::errors::*;

pub const SEED_WORKER: &[u8] = b"worker";
pub const MAX_COMMISSION_RATE: u64 = 90;
/// Worker
#[account]
#[derive(Debug)]
pub struct Worker {
    /// The worker's authority (owner).
    pub authority: Pubkey,
    /// Integer between 0 and MAX_COMMISSION_RATE determining the percentage of fees worker will keep as commission.
    pub commission_rate: u64,
    /// The worker's id.
    pub id: u64,
    /// The worker's signatory address (used to sign txs).
    pub signatory: Pubkey
}

impl Worker {
    pub fn pubkey(id: u64) -> Pubkey {
        Pubkey::find_program_address(&[SEED_WORKER, id.to_be_bytes().as_ref()], &crate::ID).0
    }
}

/// WorkerSettings
#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct WorkerSettings {
    pub commission_rate: u64,
    pub signatory: Pubkey,
}

/// WorkerAccount
pub trait WorkerAccount {
    fn pubkey(&self) -> Pubkey;

    fn init(&mut self, authority: &mut Signer, id: u64, signatory: &Signer) -> Result<()>;

    fn update(&mut self, settings: WorkerSettings) -> Result<()>;
}

impl WorkerAccount for Account<'_, Worker> {
    fn pubkey(&self) -> Pubkey {
        Worker::pubkey(self.id)
    }

    fn init(&mut self, authority: &mut Signer, id: u64, signatory: &Signer) -> Result<()> {
        self.authority = authority.key();
        self.commission_rate = MAX_COMMISSION_RATE;
        self.id = id;
        self.signatory = signatory.key();
        Ok(())
    }

    fn update(&mut self, settings: WorkerSettings) -> Result<()> {
        require!(
            settings.commission_rate.ge(&0) && settings.commission_rate.le(&MAX_COMMISSION_RATE),
            AntegenNetworkError::InvalidCommissionRate
        );
        self.commission_rate = settings.commission_rate;

        require!(
            settings.signatory.ne(&self.authority),
            AntegenNetworkError::InvalidSignatory
        );
        self.signatory = settings.signatory;
        Ok(())
    }
}
