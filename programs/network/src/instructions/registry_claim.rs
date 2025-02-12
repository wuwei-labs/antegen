use {crate::state::*, anchor_lang::prelude::*};

#[derive(Accounts)]
pub struct RegistryClaim<'info> {
  #[account()]
  pub payer: Signer<'info>,

  /// CHECK: This field is verified by some external logic, which ensures its safety.
  #[account(address = config.admin)]
  pub admin: UncheckedAccount<'info>,

  #[account(
      address = Config::pubkey(), 
      has_one = admin
  )]
  pub config: Account<'info, Config>,

  #[account(address = Registry::pubkey())]
  pub registry: Account<'info, Registry>,

  #[account(
    mut,
    seeds = [SEED_REGISTRY_FEE, registry.key().as_ref()],
    bump,
    has_one = registry,
  )]
  pub registry_fee: Account<'info, RegistryFee>,
}

pub fn handler(ctx: Context<RegistryClaim>) -> Result<()> {
  let registry_fee = &mut ctx.accounts.registry_fee;
  let pay_to = &mut ctx.accounts.admin;

  let account_data_len = 8 + registry_fee.try_to_vec()?.len();
  let rent_exempt_lamports = Rent::get()?.minimum_balance(account_data_len);
  let current_lamports = registry_fee.to_account_info().lamports();

  let available_lamports = current_lamports
      .checked_sub(rent_exempt_lamports)
      .unwrap_or(0);

  if available_lamports.gt(&0) {
      **registry_fee.to_account_info().try_borrow_mut_lamports()? = registry_fee
          .to_account_info()
          .lamports()
          .checked_sub(available_lamports)
          .unwrap();

      **pay_to.to_account_info().try_borrow_mut_lamports()? = pay_to
          .to_account_info()
          .lamports()
          .checked_add(available_lamports)
          .unwrap();
  }

  Ok(())
}
