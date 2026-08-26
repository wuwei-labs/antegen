use crate::constants::*;
use crate::errors::AntegenFiberError;
use crate::state::Fiber;
use anchor_lang::prelude::*;

/// Accounts required by the `close_fiber` instruction.
/// Thread PDA is signer, receives rent back. Works for both legacy and V1
/// fiber accounts — discriminator is read manually via `Fiber`.
#[derive(Accounts)]
#[instruction(fiber_index: u8)]
pub struct Close<'info> {
    /// Thread PDA - signer, receives rent back
    #[account(mut)]
    pub thread: Signer<'info>,

    /// CHECK: shape-agnostic — bound to the signing thread and to
    /// `fiber_index` by seeds, then read manually via `Fiber` below.
    ///
    /// The seeds are what make closing safe. Deriving the account from the
    /// signer means a wallet cannot present someone else's fiber, and deriving
    /// it from the index means a caller cannot retire index N in its own
    /// bookkeeping while handing over index M's account — the desync that
    /// leaves a thread naming a fiber that no longer exists.
    #[account(
        mut,
        seeds = [SEED_THREAD_FIBER, thread.key().as_ref(), &[fiber_index]],
        bump,
    )]
    pub fiber: UncheckedAccount<'info>,
}

pub fn close(ctx: Context<Close>, _fiber_index: u8) -> Result<()> {
    let fiber_info = ctx.accounts.fiber.to_account_info();
    let thread_info = ctx.accounts.thread.to_account_info();

    // Redundant against the seeds above, and kept anyway: it is the one check
    // that reads the account's own record of who owns it, so a fiber whose
    // stored owner ever diverges from its address fails closed rather than
    // being swept. It is also the check an attacker satisfied on mainnet by
    // first rewriting `state.thread` through `create`'s unvalidated
    // update-in-place branch — which is why it is no longer the only one.
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
