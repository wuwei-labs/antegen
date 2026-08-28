use anchor_lang::prelude::*;

use crate::state::Signal;

/// Accounts for thread_memo - simple memo functionality for thread testing.
/// Only called via CPI from thread_exec, so authorization is verified by signer.
/// The thread signs via invoke_signed in thread_exec.
#[derive(Accounts)]
pub struct ThreadMemo<'info> {
    /// The thread account that signs this instruction via CPI.
    ///
    /// This instruction is only reachable through `thread_exec`, which signs
    /// as the thread PDA. Requiring the signer to be an account this program
    /// owns keeps it from being driven directly by an arbitrary wallet.
    #[account(
        constraint = signer.to_account_info().owner == &crate::ID,
    )]
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
