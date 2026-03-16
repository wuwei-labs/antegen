use crate::{errors::*, state::compile_instruction, *};
use anchor_lang::{prelude::*, solana_program::instruction::Instruction};

/// Accounts required by the `fiber_update` instruction.
#[derive(Accounts)]
#[instruction(fiber_index: u8, instruction: SerializableInstruction)]
pub struct FiberUpdate<'info> {
    /// The authority of the thread or the thread itself (also pays rent for init_if_needed)
    #[account(
        mut,
        constraint = authority.key().eq(&thread.authority) || authority.key().eq(&thread.key())
    )]
    pub authority: Signer<'info>,

    /// The thread the fiber belongs to
    #[account(
        mut,
        seeds = [SEED_THREAD, thread.authority.as_ref(), thread.id.as_slice()],
        bump = thread.bump,
    )]
    pub thread: Account<'info, Thread>,

    /// The fiber account — created on first call, updated on subsequent calls
    #[account(
        init_if_needed,
        seeds = [SEED_THREAD_FIBER, thread.key().as_ref(), &[fiber_index]],
        bump,
        payer = authority,
        space = 8 + FiberState::INIT_SPACE,
    )]
    pub fiber: Account<'info, FiberState>,

    #[account(address = anchor_lang::system_program::ID)]
    pub system_program: Program<'info, System>,
}

pub fn fiber_update(
    ctx: Context<FiberUpdate>,
    fiber_index: u8,
    instruction: Instruction,
    signer_seeds: Option<Vec<Vec<Vec<u8>>>>,
    priority_fee: Option<u64>,
) -> Result<()> {
    // Prevent thread_delete instructions in fibers
    if instruction.program_id.eq(&crate::ID)
        && instruction.data.len() >= 8
        && instruction.data[..8].eq(crate::instruction::DeleteThread::DISCRIMINATOR)
    {
        return Err(AntegenThreadError::InvalidInstruction.into());
    }

    let fiber = &mut ctx.accounts.fiber;
    let thread = &mut ctx.accounts.thread;

    // Detect first init: fiber.thread will be Pubkey::default() for a freshly created account
    let is_first_init = fiber.thread == Pubkey::default();

    if is_first_init {
        // Enforce sequential creation
        require!(
            fiber_index == thread.fiber_next_id,
            AntegenThreadError::InvalidFiberIndex
        );

        // Initialize fiber identity
        fiber.thread = thread.key();
        fiber.fiber_index = fiber_index;
        fiber.priority_fee = priority_fee.unwrap_or(0);

        // Update thread's fiber tracking
        if !thread.fiber_ids.contains(&fiber_index) {
            thread.fiber_ids.push(fiber_index);
            thread.fiber_ids.sort();
        }
        thread.fiber_next_id = thread.fiber_next_id.saturating_add(1);

        // Use provided signer_seeds or default to thread PDA seeds
        let seeds = signer_seeds.unwrap_or_else(|| {
            vec![vec![
                SEED_THREAD.to_vec(),
                thread.authority.to_bytes().to_vec(),
                thread.id.clone(),
            ]]
        });

        let compiled = compile_instruction(instruction, seeds)?;
        let compiled_bytes = borsh::to_vec(&compiled)?;
        fiber.compiled_instruction = compiled_bytes;
    } else {
        // Update path: use thread PDA seeds (existing behavior)
        let seeds = vec![vec![
            SEED_THREAD.to_vec(),
            thread.authority.to_bytes().to_vec(),
            thread.id.clone(),
        ]];

        let compiled = compile_instruction(instruction, seeds)?;
        let compiled_bytes = borsh::to_vec(&compiled)?;
        fiber.compiled_instruction = compiled_bytes;
    }

    // Common: reset execution stats
    fiber.last_executed = 0;
    fiber.exec_count = 0;

    Ok(())
}
