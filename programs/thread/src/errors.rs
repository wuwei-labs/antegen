//! Errors thrown by the program.

use anchor_lang::prelude::*;

/// Errors for the the Antegen thread program.
#[error_code]
pub enum AntegenThreadError {
    /// Thrown if a exec response has an invalid program ID or cannot be parsed.
    #[msg("The exec response could not be parsed")]
    InvalidThreadResponse,

    /// Thrown if a thread has an invalid state and cannot complete the operation.
    #[msg("The thread is in an invalid state")]
    InvalidThreadState,

    /// The provided trigger variant is invalid.
    #[msg("The trigger variant cannot be changed")]
    InvalidTriggerVariant,

    /// The provided nonce account is invalid.
    #[msg("The provided nonce account is invalid")]
    InvalidNonceAccount,

    /// Thrown if a exec instruction is invalid because the thread's trigger condition has not been met.
    #[msg("The trigger condition has not been activated")]
    TriggerConditionFailed,

    #[msg("This operation cannot be processed because the thread is currently busy")]
    ThreadBusy,

    /// Thrown if a request is invalid because the thread is currently paused.
    #[msg("The thread is currently paused")]
    ThreadPaused,

    /// Thrown if a exec instruction would cause a thread to exceed its rate limit.
    #[msg("The thread's rate limit has been reached")]
    RateLimitExeceeded,

    /// Thrown if a thread authority attempts to set a rate limit above the maximum allowed value.
    #[msg("Thread rate limits cannot exceed the maximum allowed value")]
    MaxRateLimitExceeded,

    /// Thrown if an inner instruction attempted to write to an unauthorized address.
    #[msg("Inner instruction attempted to write to an unauthorized address")]
    UnauthorizedWrite,

    /// Thrown if the user attempts to withdraw SOL that would put a thread below it's minimum rent threshold.
    #[msg("Withdrawing this amount would leave the thread with less than the minimum required SOL for rent exemption")]
    WithdrawalTooLarge,

    #[msg("Thread IDs are limited to 32 bytes")]
    ThreadIdTooLong,

    #[msg("InsufficientFunds")]
    InsufficientFunds,

    #[msg("MathOverflow")]
    MathOverflow,

    #[msg("Thread does not have a nonce account")]
    ThreadHasNoNonceAccount,

    #[msg("Thread is currently being observed by observers")]
    ThreadBeingObserved,

    #[msg("Observer has not claimed this thread")]
    ObserverNotClaimed,

    #[msg("Invalid thread authority")]
    InvalidThreadAuthority,

    #[msg("Invalid observer authority")]
    InvalidObserverAuthority,

    #[msg("Invalid registry admin")]
    InvalidRegistryAdmin,

    #[msg("Invalid instruction provided to thread_submit")]
    InvalidInstruction,

    #[msg("Invalid signatory for observer")]
    InvalidSignatory,

    #[msg("This instruction must be called via CPI")]
    MustBeCalledViaCPI,
    
    #[msg("Fiber already claimed by another observer")]
    AlreadyClaimed,
    
    #[msg("Wrong fiber index for current execution")]
    WrongFiberIndex,
    
    #[msg("Observer priority window is still active")]
    ObserverPriorityActive,
    
    #[msg("Trigger is not ready yet")]
    TriggerNotReady,
    
    #[msg("Nonce account is required for all threads")]
    NonceRequired,
    
    #[msg("Invalid observer account provided")]
    InvalidObserverAccount,
    
    #[msg("Invalid config admin")]
    InvalidConfigAdmin,
    
    #[msg("Global pause is active")]
    GlobalPauseActive,
    
    #[msg("Invalid authority for this operation")]
    InvalidAuthority,
    
    #[msg("Invalid fee percentage (must be 0-10000)")]
    InvalidFeePercentage,
    
    #[msg("Initial instruction provided but fiber account is missing")]
    MissingFiberAccount,

    #[msg("Invalid fiber index specified in ThreadResponse")]
    InvalidFiberIndex,

    #[msg("Thread has fibers that must be deleted before the thread can be deleted")]
    ThreadHasFibers,

    #[msg("Thread has no fibers to execute")]
    ThreadHasNoFibersToExecute,

    #[msg("Invalid execution index - fiber not found in thread")]
    InvalidExecIndex,

    #[msg("Only the last executor or no executor can report errors")]
    NotLastExecutor,

    #[msg("An error has already been reported for this thread")]
    ErrorAlreadyReported,

    #[msg("Thread is not sufficiently overdue to report an error")]
    ThreadNotSufficientlyOverdue,

    #[msg("Payment distribution failed")]
    PaymentFailed,

    #[msg("Fiber account is required for this execution")]
    FiberAccountRequired,

    #[msg("Invalid fiber cursor provided")]
    InvalidFiberCursor,

    #[msg("Invalid fiber account - does not belong to this thread or not in fiber_ids")]
    InvalidFiberAccount,

    #[msg("Missing fiber accounts - all external fibers must be provided for deletion")]
    MissingFiberAccounts,

    #[msg("Thread has not signaled close - fiber_signal must be Signal::Close")]
    CloseNotSignaled,

    #[msg("Chain signal must target the next consecutive fiber")]
    InvalidChainTarget,
}

/// Alias for AntegenThreadError
pub use AntegenThreadError as ThreadError;