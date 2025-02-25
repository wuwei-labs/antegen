use {
    crate::{errors::*, state::*},
    anchor_lang::{
        prelude::*,
        solana_program::system_program,
    },
    std::mem::size_of,
};

#[derive(Accounts)]
pub struct WorkerCreate<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(constraint = signatory.key().ne(&authority.key()) @ AntegenNetworkError::InvalidSignatory)]
    pub signatory: Signer<'info>,

    #[account(
        init_if_needed,
        seeds = [
            SEED_WORKER,
            registry.total_workers.to_be_bytes().as_ref(),
        ],
        bump,
        payer = authority,
        space = 8 + size_of::<Worker>(),
    )]
    pub worker: Account<'info, Worker>,

    #[account(
        init_if_needed,
        seeds = [
            SEED_WORKER_COMMISSION,
            worker.key().as_ref(),
        ],
        bump,
        payer = authority,
        space = 8 + size_of::<WorkerCommission>(),
    )]
    pub commission: Account<'info, WorkerCommission>,

    #[account(
        mut, 
        address = Registry::pubkey(),
        constraint = !registry.locked @ AntegenNetworkError::RegistryLocked
    )]
    pub registry: Account<'info, Registry>,

    #[account(address = system_program::ID)]
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<WorkerCreate>) -> Result<()> {
    // Get accounts
    let authority: &mut Signer = &mut ctx.accounts.authority;
    let commission: &mut Account<WorkerCommission> = &mut ctx.accounts.commission;
    let registry: &mut Account<Registry> = &mut ctx.accounts.registry;
    let signatory: &mut Signer = &mut ctx.accounts.signatory;
    let worker: &mut Account<Worker> = &mut ctx.accounts.worker;

    // Initialize the worker accounts.
    worker.init(authority, registry.total_workers, signatory)?;
    commission.init(worker.key())?;

    // Update the registry's worker counter.
    registry.total_workers = registry.total_workers.checked_add(1).unwrap();

    Ok(())
}
