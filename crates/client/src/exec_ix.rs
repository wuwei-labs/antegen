//! Assembly of the `thread_exec` instruction.
//!
//! Pure and RPC-free on purpose. The node's executor and the CLI both need to
//! build this instruction, and they fetch their inputs very differently — the
//! executor from a pooled, cached RPC, the CLI from a single one-shot call.
//! Only the *assembly* is common, so only the assembly lives here.
//!
//! Keeping it in one place matters more than the line count saved. The account
//! order, the `PAYER_PUBKEY` substitution and the writability derivation all
//! have to agree exactly with what the program expects; two copies drifting
//! apart would produce an instruction that fails somewhere deep in a CPI, which
//! is precisely the kind of bug that is expensive to trace back to its cause.

use anchor_lang::{InstructionData, ToAccountMetas};
use antegen_thread_program::accounts::ThreadExec;
use antegen_thread_program::fiber::CompiledInstructionV0;
use antegen_thread_program::instruction::ExecThread;
use antegen_thread_program::state::Thread;
use antegen_thread_program::state::PAYER_PUBKEY;
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::sysvar;

/// Everything needed to assemble a `thread_exec`, already fetched.
pub struct ThreadExecParams<'a> {
    /// Thread program to invoke.
    pub program_id: Pubkey,
    /// Signer paying for and executing the transaction. Substituted wherever
    /// the compiled instruction refers to `PAYER_PUBKEY`.
    pub executor: Pubkey,
    pub thread_pubkey: Pubkey,
    pub thread: &'a Thread,
    /// Fiber holding the compiled instruction to run.
    pub fiber_pubkey: Pubkey,
    /// Decoded compiled instruction from that fiber.
    pub compiled: &'a CompiledInstructionV0,
    pub config_pubkey: Pubkey,
    /// `ThreadConfig::admin`, which receives the core-team fee.
    pub admin: Pubkey,
    pub fiber_cursor: u8,
    pub forgo_commission: bool,
}

/// Base accounts for `ThreadExec`, before the compiled instruction's own
/// accounts are appended.
///
/// Public because the close path builds a `thread_exec` too — it runs a
/// pre-compiled `close_fiber` and supplies its own remaining accounts, but the
/// base set must match this one exactly.
pub fn thread_exec_base_accounts(
    executor: Pubkey,
    thread_pubkey: Pubkey,
    thread: &Thread,
    fiber_pubkey: Pubkey,
    config_pubkey: Pubkey,
    admin: Pubkey,
) -> Vec<AccountMeta> {
    let has_nonce = thread.has_nonce_account();

    ThreadExec {
        executor,
        thread: thread_pubkey,
        fiber: fiber_pubkey,
        config: config_pubkey,
        admin,
        nonce_account: has_nonce.then_some(thread.nonce_account),
        recent_blockhashes: has_nonce.then_some(sysvar::recent_blockhashes::ID),
        system_program: solana_system_interface::program::ID,
    }
    .to_account_metas(Some(false))
}

/// Append the compiled instruction's accounts as remaining accounts.
///
/// Writability is positional: the compiled table is ordered rw-signers,
/// ro-signers, rw-non-signers, then ro-non-signers. Nothing is marked a signer
/// at transaction level — the thread signs for them via `invoke_signed`.
fn push_compiled_accounts(accounts: &mut Vec<AccountMeta>, p: &ThreadExecParams) {
    let c = p.compiled;
    for (i, pubkey) in c.accounts.iter().enumerate() {
        // Table order is: rw-signers, ro-signers, rw-non-signers, ro-non-signers.
        let idx = i as u8;
        let signers_end = c.num_rw_signers + c.num_ro_signers;
        let writable_end = signers_end + c.num_rw;
        let is_writable = idx < c.num_rw_signers || (idx >= signers_end && idx < writable_end);

        accounts.push(AccountMeta {
            pubkey: if pubkey.eq(&PAYER_PUBKEY) {
                p.executor
            } else {
                *pubkey
            },
            is_signer: false,
            is_writable,
        });
    }
}

/// Assemble a `thread_exec` instruction.
pub fn build_thread_exec_instruction(p: &ThreadExecParams) -> Instruction {
    let mut accounts = thread_exec_base_accounts(
        p.executor,
        p.thread_pubkey,
        p.thread,
        p.fiber_pubkey,
        p.config_pubkey,
        p.admin,
    );
    push_compiled_accounts(&mut accounts, p);

    Instruction {
        program_id: p.program_id,
        accounts,
        data: ExecThread {
            forgo_commission: p.forgo_commission,
            fiber_cursor: p.fiber_cursor,
        }
        .data(),
    }
}
