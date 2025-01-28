use anchor_lang::{prelude::*, AnchorDeserialize};

pub const SEED_WORKER_COMMISSION: &[u8] = b"commission";

/// Escrows the lamport balance owed to a particular worker.
#[account]
#[derive(Debug)]
pub struct WorkerCommission {
    pub bump: u8,
    pub worker: Pubkey
}

impl WorkerCommission {
    pub fn pubkey(worker: Pubkey) -> Pubkey {
        Pubkey::find_program_address(
            &[
                SEED_WORKER_COMMISSION,
                worker.as_ref(),
            ],
            &crate::ID,
        )
        .0
    }
}

/// Trait for reading and writing to a fee account.
pub trait WorkerCommissionAccount {
    /// Get the pubkey of the fee account.
    fn pubkey(&self) -> Pubkey;

    /// Initialize the account to hold fee object.
    fn init(&mut self, worker: Pubkey) -> Result<()>;
}

impl WorkerCommissionAccount for Account<'_, WorkerCommission> {
    fn pubkey(&self) -> Pubkey {
        WorkerCommission::pubkey(self.worker)
    }

    fn init(&mut self, worker: Pubkey) -> Result<()> {
        self.worker = worker;
        Ok(())
    }
}
