use anchor_lang::{prelude::*, AnchorDeserialize};

pub const SEED_REGISTRY_FEE: &[u8] = b"fee";

/// Escrows the network fees
#[account]
#[derive(Debug)]
pub struct RegistryFee {
    pub bump: u8,
    pub registry: Pubkey
}

impl RegistryFee {
    pub fn pubkey(registry: Pubkey) -> Pubkey {
        Pubkey::find_program_address(
            &[
                SEED_REGISTRY_FEE,
                registry.as_ref(),
            ],
            &crate::ID,
        )
        .0
    }

    pub fn init(&mut self, registry: Pubkey) -> Result<()> {
        self.registry = registry;
        Ok(())
    }
}
