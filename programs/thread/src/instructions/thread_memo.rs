use anchor_lang::prelude::*;

use crate::state::Signal;

/// Accounts for thread_memo - simple memo functionality for thread testing.
/// Only called via CPI from thread_exec, so authorization is verified by signer.
/// The thread signs via invoke_signed in thread_exec.
#[derive(Accounts)]
pub struct ThreadMemo<'info> {
    /// The account that signs this instruction.
    ///
    /// Deliberately unconstrained beyond the signature. This carried an
    /// `owner == crate::ID` constraint briefly, on the assumption that the
    /// signer is always the thread PDA; it is not. The signer is whichever
    /// account the fiber's compiled instruction names, and `thread_exec`
    /// substitutes the executor's wallet for the payer placeholder — a
    /// system-owned account. The constraint failed every exec path that
    /// routes through here.
    pub signer: Signer<'info>,
}

pub fn thread_memo(
    _ctx: Context<ThreadMemo>,
    memo: String,
    signal: Option<Signal>,
) -> Result<Signal> {
    msg!("Thread memo: {}", memo);

    if let Some(response) = signal {
        return Ok(response);
    }

    Ok(Signal::None)
}
