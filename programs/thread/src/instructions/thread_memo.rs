use anchor_lang::prelude::*;

use crate::state::Signal;

/// Accounts for thread_memo - simple memo functionality for thread testing.
/// Only called via CPI from thread_exec, so authorization is verified by signer.
/// The thread signs via invoke_signed in thread_exec.
#[derive(Accounts)]
pub struct ThreadMemo<'info> {
    /// The thread account that signs this instruction via CPI
    pub signer: Signer<'info>,
}

pub fn thread_memo(
    _ctx: Context<ThreadMemo>,
    memo: String,
    signal: Option<Signal>,
) -> Result<Signal> {
    msg!("Thread memo: {}", memo);

    if signal.is_some() {
        let response: Signal = signal.unwrap();
        return Ok(response);
    }

    Ok(Signal::None)
}
