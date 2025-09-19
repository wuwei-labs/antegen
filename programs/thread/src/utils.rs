use anchor_lang::prelude::*;
use chrono::{DateTime, Utc};
use solana_cron::Schedule;
use std::str::FromStr;

/// Calculate the next timestamp for a cron schedule
pub fn next_timestamp(after: i64, schedule: String) -> Option<i64> {
    Schedule::from_str(&schedule)
        .unwrap()
        .next_after(&DateTime::<Utc>::from_timestamp(after, 0).unwrap())
        .take()
        .map(|datetime| datetime.timestamp())
}

/// Safely transfer lamports from one account to another
pub fn transfer_lamports(from: &AccountInfo, to: &AccountInfo, amount: u64) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }
    
    // Deduct from source
    **from.try_borrow_mut_lamports()? = from
        .lamports()
        .checked_sub(amount)
        .ok_or(ProgramError::InsufficientFunds)?;
    
    // Add to destination
    **to.try_borrow_mut_lamports()? = to
        .lamports()
        .checked_add(amount)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    
    Ok(())
}