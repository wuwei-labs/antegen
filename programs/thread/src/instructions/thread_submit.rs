use {
    crate::{constants::*, errors::*, state::*},
    anchor_lang::{
        prelude::*,
        solana_program::{
            instruction::Instruction,
            program::invoke,
        },
    },
    antegen_network_program::state::{Builder, Registry, SEED_BUILDER, SEED_REGISTRY},
};

#[derive(Accounts)]
pub struct ThreadSubmit<'info> {
    #[account(mut)]
    pub submitter: Signer<'info>,

    #[account(
        mut,
        seeds = [
            SEED_THREAD,
            thread.authority.as_ref(),
            thread.id.as_slice(),
        ],
        bump = thread.bump,
    )]
    pub thread: Account<'info, Thread>,

    #[account(
        seeds = [
            SEED_BUILDER,
            builder.id.to_be_bytes().as_ref(),
        ],
        bump = builder.bump,
        constraint = thread.has_builder(builder.id) @ ThreadError::BuilderNotClaimed,
    )]
    pub builder: Account<'info, Builder>,

    #[account(
        seeds = [SEED_REGISTRY],
        bump = registry.bump,
    )]
    pub registry: Account<'info, Registry>,

    /// The authority of the thread (for fee distribution)
    /// CHECK: This is validated by the thread account
    #[account(
        mut,
        constraint = thread_authority.key() == thread.authority @ ThreadError::InvalidThreadAuthority,
    )]
    pub thread_authority: UncheckedAccount<'info>,

    /// The builder's authority (for fee distribution)
    /// CHECK: This is validated by the builder account
    #[account(
        mut,
        constraint = builder_authority.key() == builder.authority @ ThreadError::InvalidBuilderAuthority,
    )]
    pub builder_authority: UncheckedAccount<'info>,

    /// The registry admin (for core team fee distribution)
    /// CHECK: This is validated by the registry account
    #[account(
        mut,
        constraint = registry_admin.key() == registry.admin @ ThreadError::InvalidRegistryAdmin,
    )]
    pub registry_admin: UncheckedAccount<'info>,

    /// The thread execution program
    pub thread_program: Program<'info, crate::program::ThreadProgram>,
}

pub fn handler(
    ctx: Context<ThreadSubmit>,
    thread_exec_ix_data: Vec<u8>,
) -> Result<()> {
    let thread = &mut ctx.accounts.thread;
    let builder = &ctx.accounts.builder;
    let registry = &ctx.accounts.registry;
    let submitter = &ctx.accounts.submitter;

    // Build the thread_exec instruction
    let thread_exec_ix = Instruction {
        program_id: crate::ID,
        accounts: ctx.remaining_accounts.iter().map(|acc| {
            anchor_lang::solana_program::instruction::AccountMeta {
                pubkey: acc.key(),
                is_signer: acc.is_signer,
                is_writable: acc.is_writable,
            }
        }).collect(),
        data: thread_exec_ix_data,
    };

    // Execute the thread_exec instruction via CPI
    invoke(&thread_exec_ix, &ctx.remaining_accounts)?;

    // Calculate fee distribution
    let total_fee = registry.commission_fee;
    let builder_fee = (total_fee * registry.builder_commission_bps) / 10_000;
    let submitter_fee = (total_fee * registry.submitter_commission_bps) / 10_000;
    let core_team_fee = (total_fee * registry.core_team_bps) / 10_000;

    // Transfer fees
    let is_builder_submitter = builder.signatory == submitter.key();
    
    if is_builder_submitter {
        // Builder is also the submitter, gets both shares
        let combined_fee = builder_fee + submitter_fee;
        
        // Transfer to builder
        **ctx.accounts.builder_authority.try_borrow_mut_lamports()? += combined_fee;
        **ctx.accounts.thread_authority.try_borrow_mut_lamports()? -= combined_fee;
    } else {
        // Separate builder and submitter
        
        // Transfer to builder
        **ctx.accounts.builder_authority.try_borrow_mut_lamports()? += builder_fee;
        **ctx.accounts.thread_authority.try_borrow_mut_lamports()? -= builder_fee;
        
        // Transfer to submitter
        **ctx.accounts.submitter.try_borrow_mut_lamports()? += submitter_fee;
        **ctx.accounts.thread_authority.try_borrow_mut_lamports()? -= submitter_fee;
    }

    // Transfer to core team
    **ctx.accounts.registry_admin.try_borrow_mut_lamports()? += core_team_fee;
    **ctx.accounts.thread_authority.try_borrow_mut_lamports()? -= core_team_fee;

    // Clear builders after successful submission
    thread.clear_builders();

    Ok(())
}