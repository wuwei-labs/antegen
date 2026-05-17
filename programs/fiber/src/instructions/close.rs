use crate::errors::AntegenFiberError;
use crate::state::Fiber;
use anchor_lang::prelude::*;

/// Accounts required by the `close_fiber` instruction.
/// Thread PDA is signer, receives rent back. Works for both legacy and V1
/// fiber accounts — discriminator is read manually via `Fiber`.
#[derive(Accounts)]
pub struct Close<'info> {
    /// Thread PDA - signer, receives rent back
    #[account(mut)]
    pub thread: Signer<'info>,

    /// CHECK: shape-agnostic fiber account — validated manually below.
    #[account(mut)]
    pub fiber: UncheckedAccount<'info>,
}

pub fn close(ctx: Context<Close>) -> Result<()> {
    let fiber_info = ctx.accounts.fiber.to_account_info();
    let thread_info = ctx.accounts.thread.to_account_info();

    let read = {
        let data = fiber_info.try_borrow_data()?;
        Fiber::try_deserialize(&mut &data[..])?
    };
    require!(
        read.thread() == thread_info.key(),
        AntegenFiberError::InvalidFiberPDA
    );

    sweep_fiber_lamports(&fiber_info, &thread_info)?;
    Ok(())
}

/// Drains all lamports from `fiber` into `thread` and zeros the data buffer.
/// Sets the discriminator bytes to Anchor's `CLOSED_ACCOUNT_DISCRIMINATOR`
/// sentinel so future reads recognize the slot as closed.
pub(crate) fn sweep_fiber_lamports<'info>(
    fiber: &AccountInfo<'info>,
    thread: &AccountInfo<'info>,
) -> Result<()> {
    let fiber_lamports = fiber.lamports();
    **thread.try_borrow_mut_lamports()? = thread
        .lamports()
        .checked_add(fiber_lamports)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    **fiber.try_borrow_mut_lamports()? = 0;

    let mut data = fiber.try_borrow_mut_data()?;
    for byte in data.iter_mut() {
        *byte = 0;
    }
    // Anchor closed-account sentinel: [0xff; 8] in the first 8 bytes — tells
    // downstream readers the slot is closed (cf. anchor's `close` constraint).
    if data.len() >= 8 {
        data[..8].copy_from_slice(&[0xff; 8]);
    }
    Ok(())
}
