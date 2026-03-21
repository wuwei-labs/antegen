use crate::constants::*;
use crate::errors::AntegenFiberError;
use crate::state::*;
use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::Instruction;
use anchor_lang::solana_program::program::invoke_signed;
use anchor_lang::solana_program::system_instruction;

/// Accounts required by the `create_fiber` instruction.
/// Thread PDA is the signer (authority). Fiber must be pre-funded with rent lamports.
#[derive(Accounts)]
#[instruction(fiber_index: u8)]
pub struct FiberCreate<'info> {
    /// Thread PDA - signer (via invoke_signed from Thread Program)
    pub thread: Signer<'info>,

    /// CHECK: The fiber account to create — validated manually via PDA derivation
    #[account(mut)]
    pub fiber: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub fn fiber_create(
    ctx: Context<FiberCreate>,
    fiber_index: u8,
    instruction: Instruction,
    priority_fee: u64,
) -> Result<()> {
    let thread_key = ctx.accounts.thread.key();
    let fiber_info = ctx.accounts.fiber.to_account_info();

    if fiber_info.data_len() == 0 {
        // Not initialized — full init
        initialize_fiber(
            &ctx.accounts.fiber,
            &ctx.accounts.system_program,
            &thread_key,
            fiber_index,
            &instruction,
            priority_fee,
        )
    } else {
        // Already initialized — update in place (same as fiber_update)
        let mut data = fiber_info.try_borrow_mut_data()?;
        let discriminator = FiberState::DISCRIMINATOR;
        if data[..8] != discriminator[..] {
            return Err(anchor_lang::error::ErrorCode::AccountDiscriminatorMismatch.into());
        }

        let compiled = compile_instruction(instruction)?;
        let compiled_bytes = borsh::to_vec(&compiled)?;

        let mut state: FiberState = FiberState::try_deserialize(&mut &data[..])?;
        state.thread = thread_key;
        state.compiled_instruction = compiled_bytes;
        state.priority_fee = priority_fee;
        state.last_executed = 0;
        state.exec_count = 0;

        let state_bytes = borsh::to_vec(&state)?;
        data[8..8 + state_bytes.len()].copy_from_slice(&state_bytes);
        Ok(())
    }
}

/// Shared helper for manual fiber account initialization.
/// Derives PDA with known fiber_index (single find_program_address call, same as Anchor),
/// validates key match, checks not already initialized, allocates + assigns via invoke_signed,
/// then writes discriminator + state.
pub fn initialize_fiber<'info>(
    fiber: &UncheckedAccount<'info>,
    system_program: &Program<'info, System>,
    thread_key: &Pubkey,
    fiber_index: u8,
    instruction: &Instruction,
    priority_fee: u64,
) -> Result<()> {
    let fiber_info = fiber.to_account_info();

    // Derive PDA with known seeds — single call, same as Anchor does
    let (expected_pda, bump) = Pubkey::find_program_address(
        &[SEED_THREAD_FIBER, thread_key.as_ref(), &[fiber_index]],
        &crate::ID,
    );
    require!(
        expected_pda.eq(&fiber.key()),
        AntegenFiberError::InvalidFiberPDA
    );

    // Verify rent
    let space = 8 + FiberState::INIT_SPACE;
    let rent = Rent::get()?;
    let min_lamports = rent.minimum_balance(space);
    require!(
        fiber_info.lamports().ge(&min_lamports),
        AntegenFiberError::InsufficientRent
    );

    // Allocate space via invoke_signed (fiber PDA is derived from fiber program)
    let seeds: &[&[u8]] = &[
        SEED_THREAD_FIBER,
        thread_key.as_ref(),
        &[fiber_index],
        &[bump],
    ];

    invoke_signed(
        &system_instruction::allocate(&fiber.key(), space as u64),
        &[fiber_info.clone(), system_program.to_account_info()],
        &[seeds],
    )?;

    // Assign to this program
    invoke_signed(
        &system_instruction::assign(&fiber.key(), &crate::ID),
        &[fiber_info.clone(), system_program.to_account_info()],
        &[seeds],
    )?;

    // Write discriminator + state
    let compiled = compile_instruction(instruction.clone())?;
    let compiled_bytes = borsh::to_vec(&compiled)?;

    let state = FiberState {
        thread: *thread_key,
        compiled_instruction: compiled_bytes,
        priority_fee,
        last_executed: 0,
        exec_count: 0,
    };

    // Write anchor discriminator
    let mut data = fiber_info.try_borrow_mut_data()?;
    let discriminator = FiberState::DISCRIMINATOR;
    data[..8].copy_from_slice(discriminator);

    // Serialize state after discriminator
    let state_bytes = borsh::to_vec(&state)?;
    data[8..8 + state_bytes.len()].copy_from_slice(&state_bytes);

    Ok(())
}
