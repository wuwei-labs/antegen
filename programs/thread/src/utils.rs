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