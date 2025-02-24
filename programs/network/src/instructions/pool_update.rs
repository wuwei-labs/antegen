use {
    crate::state::*,
    anchor_lang::{
        prelude::*,
        solana_program::system_program,
        system_program::{transfer, Transfer},
    },
    std::mem::size_of,
};

#[derive(Accounts)]
#[instruction(settings: PoolSettings)]
pub struct PoolUpdate<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        address = Config::pubkey(), 
        has_one = admin
    )]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        address = pool.pubkey()
    )]
    pub pool: Account<'info, Pool>,

    #[account(address = system_program::ID)]
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<PoolUpdate>, settings: PoolSettings) -> Result<()> {
    // Get accounts
    let admin: &Signer = &ctx.accounts.admin;
    let pool: &mut Account<Pool> = &mut ctx.accounts.pool;
    let system_program: &Program<System> = &ctx.accounts.system_program;

    // Update the pool settings
    pool.update(&settings)?;

    // Reallocate memory for the pool account
    let data_len: usize = 8 + size_of::<Pool>() + (settings.size as usize).checked_mul(size_of::<Pubkey>()).unwrap();
    pool.to_account_info().realloc(data_len, false)?;

    // If lamports are required to maintain rent-exemption, pay them
    let minimum_rent: u64 = Rent::get().unwrap().minimum_balance(data_len);
    if minimum_rent > pool.to_account_info().lamports() {
        transfer(
            CpiContext::new(
                system_program.to_account_info(),
                Transfer {
                    from: admin.to_account_info(),
                    to: pool.to_account_info(),
                },
            ),
            minimum_rent
                .checked_sub(pool.to_account_info().lamports())
                .unwrap(),
        )?;
    }

    Ok(())
}
