use anchor_lang::{prelude::*, AnchorDeserialize};

use crate::{errors::*, state::*};

pub const SEED_BUILDER: &[u8] = b"builder";

#[account]
#[derive(Debug, InitSpace)]
pub struct Builder {
    pub version: u64,
    pub bump: u8,
    /// The builder's authority (owner).
    pub authority: Pubkey,
    /// Commission in basis points (0-10000, where 10000 = 100%)
    pub commission_bps: u64,
    /// The builder's id.
    pub id: u32,
    /// Whether the builder is active in rotation.
    pub is_active: bool,
    /// The builder's signatory address (used to sign txs).
    pub signatory: Pubkey,
}

#[derive(Accounts)]
pub struct MigrateBuilder<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [SEED_BUILDER, builder.id.to_be_bytes().as_ref()],
        constraint = registry.admin == authority.key(),
        realloc = 8 + Builder::INIT_SPACE,
        realloc::payer = authority,
        realloc::zero = false,
        bump = builder.bump,
    )]
    pub builder: Account<'info, Builder>,

    #[account(
        seeds = [SEED_REGISTRY],
        bump = registry.bump,
    )]
    pub registry: Account<'info, Registry>,
    
    pub system_program: Program<'info, System>,
}

impl Builder {
    pub fn pubkey(id: u32) -> Pubkey {
        Pubkey::find_program_address(&[SEED_BUILDER, id.to_be_bytes().as_ref()], &crate::ID).0
    }
}

impl TryFrom<&[u8]> for Builder {
    type Error = Error;

    fn try_from(data: &[u8]) -> std::result::Result<Self, Self::Error> {
        Self::try_deserialize(&mut &*data)
    }
}

/// WorkerSettings
#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct BuilderSettings {
    pub commission_bps: u64,
    pub signatory: Pubkey,
}

/// WorkerAccount
pub trait BuilderAcount {
    fn pubkey(&self) -> Pubkey;
    fn init(&mut self, authority: &mut Signer, id: u32, signatory: &Signer, builder_commission_bps: u64) -> Result<()>;
    fn update(&mut self, settings: BuilderSettings, builder_commission_bps: u64) -> Result<()>;
}

impl BuilderAcount for Account<'_, Builder> {
    fn pubkey(&self) -> Pubkey {
        Builder::pubkey(self.id)
    }

    fn init(&mut self, authority: &mut Signer, id: u32, signatory: &Signer, builder_commission_bps: u64) -> Result<()> {
        self.authority = authority.key();
        self.commission_bps = builder_commission_bps; // Start at max, builder can lower if desired
        self.id = id;
        self.is_active = false;
        self.signatory = signatory.key();
        Ok(())
    }

    fn update(&mut self, settings: BuilderSettings, builder_commission_bps: u64) -> Result<()> {
        require!(
            settings.commission_bps.le(&builder_commission_bps),
            AntegenNetworkError::InvalidCommissionRate
        );
        self.commission_bps = settings.commission_bps;

        require!(
            settings.signatory.ne(&self.authority),
            AntegenNetworkError::InvalidSignatory
        );
        self.signatory = settings.signatory;
        Ok(())
    }
}
