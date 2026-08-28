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

    /// CHECK: The fiber account to create — bound to the signing thread and
    /// `fiber_index` by seeds, and re-derived explicitly in the handler.
    #[account(
        mut,
        seeds = [SEED_THREAD_FIBER, thread.key().as_ref(), &[fiber_index]],
        bump,
        constraint = fiber.to_account_info().owner == &crate::ID
            || fiber.to_account_info().owner == &System::id()
            @ AntegenFiberError::InvalidAccountOwner,
    )]
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

    // Bind the fiber account to the signing thread before touching it. The
    // update-in-place branch below rewrites `state.thread` to the signer, so
    // without this any wallet could sign as `thread`, claim someone else's
    // fiber, and then `close` it to sweep the rent — which is exactly what was
    // done on mainnet against live threads.
    require_keys_eq!(
        Pubkey::find_program_address(
            &[SEED_THREAD_FIBER, thread_key.as_ref(), &[fiber_index]],
            &crate::ID,
        )
        .0,
        fiber_info.key(),
        AntegenFiberError::InvalidFiberPDA
    );

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

        let fiber_read = Fiber::try_from(&fiber_info)?;

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
    // The seeds below are handed to `invoke_signed` as this program's
    // signature. Proving the account they derive matches the account we were
    // given keeps that signature from being lent to any other address.
    require_keys_eq!(
        expected_pda,
        fiber.key(),
        AntegenFiberError::InvalidFiberPDA
    );

    let space = FIBER_ACCOUNT_SPACE;
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

/// Writes `discriminator || borsh(state)` into the account.
///
/// Both length checks replace slice indexing that panicked rather than
/// returned: an update that grew the encoded state past the space the account
/// was allocated with aborted the transaction with no usable error, and a
/// buffer shorter than the discriminator did the same.
fn write_state(fiber_info: &AccountInfo, discriminator: &[u8], state_bytes: &[u8]) -> Result<()> {
    /// An Anchor discriminator is exactly 8 bytes; anything else is a caller
    /// bug and would otherwise panic inside `copy_from_slice`.
    fn try_from_slice(discriminator: &[u8]) -> Result<[u8; 8]> {
        discriminator
            .try_into()
            .map_err(|_| error!(AntegenFiberError::InvalidFiberData))
    }

    let disc = try_from_slice(discriminator)?;

    let mut data = fiber_info.try_borrow_mut_data()?;
    let end = disc
        .len()
        .checked_add(state_bytes.len())
        .ok_or(AntegenFiberError::FiberAccountTooSmall)?;
    require!(
        end <= data.len(),
        AntegenFiberError::FiberAccountTooSmall
    );

    data[..8].copy_from_slice(&disc);
    data[8..end].copy_from_slice(state_bytes);
    Ok(())
}

pub(crate) fn write_versioned(fiber_info: &AccountInfo, state: &FiberVersionedState) -> Result<()> {
    write_state(
        fiber_info,
        FiberVersionedState::DISCRIMINATOR,
        &borsh::to_vec(state)?,
    )
}

pub(crate) fn write_legacy(fiber_info: &AccountInfo, state: &FiberState) -> Result<()> {
    write_state(fiber_info, FiberState::DISCRIMINATOR, &borsh::to_vec(state)?)
}
