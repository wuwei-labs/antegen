use {
  crate::state::*,
  anchor_lang::{prelude::*, solana_program::system_program},
};
use std::mem::size_of;

#[derive(Accounts)]
pub struct RegistryReset<'info> {
  #[account(mut)]
  pub admin: Signer<'info>,

  #[account(
      address = Config::pubkey(),
      has_one = admin
  )]
  pub config: Account<'info, Config>,

  #[account(
      mut,
      address = Registry::pubkey()
  )]
  pub registry: Account<'info, Registry>,

  #[account(
      init_if_needed,
      seeds = [
          SEED_SNAPSHOT,
          (0 as u64).to_be_bytes().as_ref(),
      ],
      bump,
      payer = admin,
      space = 8 + size_of::<Snapshot>(),
  )]
  pub snapshot: Account<'info, Snapshot>,

  #[account(address = system_program::ID)]
  pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<RegistryReset>) -> Result<()> {
  // Get accounts
  let registry: &mut Account<Registry> = &mut ctx.accounts.registry;
  let snapshot: &mut Account<Snapshot> = &mut ctx.accounts.snapshot;

  // Reset accounts to their initial state
  registry.reset()?;
  snapshot.init(0)?;

  Ok(())
}
