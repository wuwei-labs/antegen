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
pub struct Create<'info> {
    /// Thread PDA - signer (via invoke_signed from Thread Program)
    pub thread: Signer<'info>,

    /// CHECK: The fiber account to create — validated manually via PDA derivation
    #[account(mut)]
    pub fiber: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub fn create(
    ctx: Context<Create>,
    fiber_index: u8,
    instruction: Instruction,
    priority_fee: u64,
    lookup_tables: Vec<Pubkey>,
) -> Result<()> {
    require!(
        lookup_tables.len() <= MAX_LOOKUP_TABLES_PER_FIBER,
        AntegenFiberError::LookupTablesExceedMax
    );

    let thread_key = ctx.accounts.thread.key();
    let fiber_info = ctx.accounts.fiber.to_account_info();

    if fiber_info.data_len() == 0 {
        initialize_fiber(
            &ctx.accounts.fiber,
            &ctx.accounts.system_program,
            &thread_key,
            fiber_index,
            &instruction,
            priority_fee,
            lookup_tables,
        )
    } else {
        // Already initialized — update in place. Dispatch by discriminator so
        // we never re-write a legacy fiber with a v1 shape (would corrupt the
        // account on disk).
        let compiled = compile_instruction(instruction)?;
        let compiled_bytes = borsh::to_vec(&compiled)?;

        let fiber_read = {
            let data = fiber_info.try_borrow_data()?;
            Fiber::try_deserialize(&mut &data[..])?
        };

        match fiber_read {
            Fiber::Legacy(mut state) => {
                require!(
                    lookup_tables.is_empty(),
                    AntegenFiberError::LegacyFiberLookupTablesUnsupported
                );
                state.thread = thread_key;
                state.compiled_instruction = compiled_bytes;
                state.priority_fee = priority_fee;
                state.last_executed = 0;
                state.exec_count = 0;
                write_legacy(&fiber_info, &state)?;
            }
            Fiber::V1(mut state) => {
                state.version = CURRENT_FIBER_VERSION;
                state.thread = thread_key;
                state.compiled_instruction = compiled_bytes;
                state.priority_fee = priority_fee;
                state.last_executed = 0;
                state.exec_count = 0;
                state.lookup_tables = lookup_tables;
                write_versioned(&fiber_info, &state)?;
            }
        }

        Ok(())
    }
}

/// Shared helper for manual fiber account initialization.
/// New writes always emit `FiberVersionedState` — legacy accounts are
/// never created post-PR.
pub fn initialize_fiber<'info>(
    fiber: &UncheckedAccount<'info>,
    system_program: &Program<'info, System>,
    thread_key: &Pubkey,
    fiber_index: u8,
    instruction: &Instruction,
    priority_fee: u64,
    lookup_tables: Vec<Pubkey>,
) -> Result<()> {
    require!(
        lookup_tables.len() <= MAX_LOOKUP_TABLES_PER_FIBER,
        AntegenFiberError::LookupTablesExceedMax
    );

    let fiber_info = fiber.to_account_info();

    let (expected_pda, bump) = Pubkey::find_program_address(
        &[SEED_THREAD_FIBER, thread_key.as_ref(), &[fiber_index]],
        &crate::ID,
    );
    require!(
        expected_pda.eq(&fiber.key()),
        AntegenFiberError::InvalidFiberPDA
    );

    let space = 8 + FiberVersionedState::INIT_SPACE;
    let rent = Rent::get()?;
    let min_lamports = rent.minimum_balance(space);
    require!(
        fiber_info.lamports().ge(&min_lamports),
        AntegenFiberError::InsufficientRent
    );

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

    invoke_signed(
        &system_instruction::assign(&fiber.key(), &crate::ID),
        &[fiber_info.clone(), system_program.to_account_info()],
        &[seeds],
    )?;

    let compiled = compile_instruction(instruction.clone())?;
    let compiled_bytes = borsh::to_vec(&compiled)?;

    let state = FiberVersionedState {
        version: CURRENT_FIBER_VERSION,
        thread: *thread_key,
        compiled_instruction: compiled_bytes,
        priority_fee,
        last_executed: 0,
        exec_count: 0,
        lookup_tables,
    };

    write_versioned(&fiber_info, &state)
}

pub(crate) fn write_versioned(fiber_info: &AccountInfo, state: &FiberVersionedState) -> Result<()> {
    let mut data = fiber_info.try_borrow_mut_data()?;
    data[..8].copy_from_slice(FiberVersionedState::DISCRIMINATOR);
    let state_bytes = borsh::to_vec(state)?;
    data[8..8 + state_bytes.len()].copy_from_slice(&state_bytes);
    Ok(())
}

pub(crate) fn write_legacy(fiber_info: &AccountInfo, state: &FiberState) -> Result<()> {
    let mut data = fiber_info.try_borrow_mut_data()?;
    data[..8].copy_from_slice(FiberState::DISCRIMINATOR);
    let state_bytes = borsh::to_vec(state)?;
    data[8..8 + state_bytes.len()].copy_from_slice(&state_bytes);
    Ok(())
}
