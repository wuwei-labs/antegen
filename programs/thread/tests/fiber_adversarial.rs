//! Adversarial review of the fiber program's rent-bearing paths.
//!
//! Every lamport-decreasing path in the fiber program routes through
//! `sweep_fiber_lamports(fiber -> thread)`, and `thread` is always the *signer*.
//! So an attacker profits only by making a victim fiber's `state.thread` equal
//! a key they control. These tests pin down each writer of that field, plus the
//! index/account desyncs that leave a thread pointing at a fiber that is gone.

use anchor_lang::{InstructionData, ToAccountMetas};
use solana_sdk::{
    instruction::Instruction,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};

mod common;
use common::*;

fn setup_thread(
    svm: &mut litesvm::LiteSVM,
    authority: &Keypair,
    payer: &Keypair,
    id: &str,
) -> Pubkey {
    let thread_id = ThreadId::Bytes(id.as_bytes().to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), id.as_bytes());
    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        30_000_000,
        thread_id,
        Trigger::Immediate { jitter: 0 },
        None,
        None,
        None,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[payer, authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();
    thread_pubkey
}

fn create_fiber(
    svm: &mut litesvm::LiteSVM,
    authority: &Keypair,
    payer: &Keypair,
    thread: &Pubkey,
    fiber_index: u8,
) -> Pubkey {
    let (fiber_pubkey, _) = fiber_pda(thread, fiber_index);
    let memo_ix = make_memo_instruction("adversarial", None);
    let ix = build_create_fiber(
        &authority.pubkey(),
        thread,
        &fiber_pubkey,
        fiber_index,
        make_serializable_instruction(&memo_ix),
        0,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[payer, authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();
    fiber_pubkey
}

fn send_as(
    svm: &mut litesvm::LiteSVM,
    signer: &Keypair,
    ix: Instruction,
) -> Result<(), litesvm::types::FailedTransactionMetadata> {
    let blockhash = svm.latest_blockhash();
    let tx =
        Transaction::new_signed_with_payer(&[ix], Some(&signer.pubkey()), &[signer], blockhash);
    svm.send_transaction(tx).map(|_| ())
}

// ---------------------------------------------------------------------------
// Writers of `state.thread` — the only field that decides who may sweep rent.
// ---------------------------------------------------------------------------

/// `update` is bound by an Anchor `seeds` constraint, so a stranger cannot use
/// it to re-point a fiber at themselves the way `create` once allowed.
#[test]
fn stranger_cannot_claim_a_fiber_via_update() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    let thread = setup_thread(&mut svm, &authority, &payer, "adv-upd");
    let fiber = create_fiber(&mut svm, &authority, &payer, &thread, 0);

    let attacker = Keypair::new();
    svm.airdrop(&attacker.pubkey(), DEFAULT_AIRDROP).unwrap();

    let memo_ix = make_memo_instruction("stolen", None);
    let ix = Instruction {
        program_id: FIBER_PROGRAM_ID,
        accounts: antegen_fiber_program::accounts::Update {
            thread: attacker.pubkey(),
            fiber,
            system_program: solana_system_interface::program::ID,
        }
        .to_account_metas(None),
        data: antegen_fiber_program::instruction::Update {
            fiber_index: 0,
            instruction: Some(make_serializable_instruction(&memo_ix)),
            priority_fee: Some(0),
            lookup_tables: antegen_fiber_program::state::Trailing(None),
        }
        .data(),
    };
    assert!(send_as(&mut svm, &attacker, ix).is_err());
    assert_eq!(deserialize_fiber_any(&svm, &fiber).thread(), thread);
}

/// A thread the attacker legitimately owns does not help: the fiber PDA is
/// derived from the signing thread, so their own thread cannot reach a fiber
/// belonging to someone else's.
#[test]
fn attackers_own_thread_cannot_reach_a_foreign_fiber() {
    let (mut svm, _admin, payer) = create_test_env();
    let victim_auth = Keypair::new();
    svm.airdrop(&victim_auth.pubkey(), DEFAULT_AIRDROP).unwrap();
    let victim_thread = setup_thread(&mut svm, &victim_auth, &payer, "adv-victim");
    let victim_fiber = create_fiber(&mut svm, &victim_auth, &payer, &victim_thread, 0);
    let rent_before = svm.get_account(&victim_fiber).unwrap().lamports;

    let attacker = Keypair::new();
    svm.airdrop(&attacker.pubkey(), DEFAULT_AIRDROP).unwrap();
    let attacker_thread = setup_thread(&mut svm, &attacker, &payer, "adv-attacker");

    // Ask the attacker's own thread to close the victim's fiber. The thread
    // program will happily forward it — the fiber program must be the one to
    // say no.
    let ix = build_close_fiber(
        &attacker.pubkey(),
        &attacker_thread,
        &victim_fiber,
        0, // an index the attacker's thread really does track
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &attacker],
        blockhash,
    );
    assert!(
        svm.send_transaction(tx).is_err(),
        "one thread must not be able to close another thread's fiber"
    );
    assert_eq!(
        svm.get_account(&victim_fiber).unwrap().lamports,
        rent_before
    );
}

/// `swap` sweeps `source`, so it is a rent path too. Both sides are checked
/// against the signing thread; a cross-thread swap must not sweep anything.
#[test]
fn swap_cannot_sweep_a_foreign_source() {
    let (mut svm, _admin, payer) = create_test_env();
    let victim_auth = Keypair::new();
    svm.airdrop(&victim_auth.pubkey(), DEFAULT_AIRDROP).unwrap();
    let victim_thread = setup_thread(&mut svm, &victim_auth, &payer, "adv-sv");
    let victim_fiber = create_fiber(&mut svm, &victim_auth, &payer, &victim_thread, 0);
    let rent_before = svm.get_account(&victim_fiber).unwrap().lamports;

    let attacker = Keypair::new();
    svm.airdrop(&attacker.pubkey(), DEFAULT_AIRDROP).unwrap();
    let attacker_thread = setup_thread(&mut svm, &attacker, &payer, "adv-sa");
    let attacker_fiber = create_fiber(&mut svm, &attacker, &payer, &attacker_thread, 0);

    // target = attacker's own fiber, source = the victim's.
    let ix = build_swap_fiber(
        &attacker.pubkey(),
        &attacker_thread,
        &attacker_fiber,
        &victim_fiber,
        0,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &attacker],
        blockhash,
    );
    assert!(svm.send_transaction(tx).is_err());
    assert_eq!(
        svm.get_account(&victim_fiber).unwrap().lamports,
        rent_before
    );
}

/// Direct `close` against a fiber the signer does not own.
#[test]
fn stranger_cannot_close_a_fiber_directly() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    let thread = setup_thread(&mut svm, &authority, &payer, "adv-close");
    let fiber = create_fiber(&mut svm, &authority, &payer, &thread, 0);
    let rent_before = svm.get_account(&fiber).unwrap().lamports;

    let attacker = Keypair::new();
    svm.airdrop(&attacker.pubkey(), DEFAULT_AIRDROP).unwrap();

    let ix = Instruction {
        program_id: FIBER_PROGRAM_ID,
        accounts: antegen_fiber_program::accounts::Close {
            thread: attacker.pubkey(),
            fiber,
        }
        .to_account_metas(None),
        data: antegen_fiber_program::instruction::Close { fiber_index: 0 }.data(),
    };
    assert!(send_as(&mut svm, &attacker, ix).is_err());
    assert_eq!(svm.get_account(&fiber).unwrap().lamports, rent_before);
}

// ---------------------------------------------------------------------------
// Index/account desync — the state that broke mainnet threads, reachable
// without an attacker.
// ---------------------------------------------------------------------------

/// `fiber_close` must reject a `fiber_index` that does not match the account.
///
/// The thread program removes `fiber_index` from `fiber_ids` while the fiber
/// program closes whatever account was passed. Before these were bound
/// together, a mismatched pair left the thread listing an index whose account
/// was gone and stranded the other one's rent in an account nothing tracked —
/// the same shape as the mainnet breakage, reachable without an attacker.
#[test]
fn fiber_close_rejects_an_index_that_does_not_match_the_account() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    let thread = setup_thread(&mut svm, &authority, &payer, "adv-desync");
    let fiber0 = create_fiber(&mut svm, &authority, &payer, &thread, 0);
    let fiber1 = create_fiber(&mut svm, &authority, &payer, &thread, 1);

    // Say index 0, hand over fiber 1's account.
    let ix = build_close_fiber(&authority.pubkey(), &thread, &fiber1, 0);
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    assert!(
        svm.send_transaction(tx).is_err(),
        "closing fiber 1's account while retiring index 0 must be rejected"
    );

    // Nothing moved: both accounts alive, both still tracked.
    let tracked = deserialize_thread(&svm, &thread).fiber_ids;
    assert!(
        tracked.contains(&0) && tracked.contains(&1),
        "got {tracked:?}"
    );
    assert!(svm.get_account(&fiber0).unwrap().lamports > 0);
    assert!(svm.get_account(&fiber1).unwrap().lamports > 0);

    // The matching pair still works.
    let ix = build_close_fiber(&authority.pubkey(), &thread, &fiber1, 1);
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx)
        .expect("a matching index/account pair must still close");
    let tracked = deserialize_thread(&svm, &thread).fiber_ids;
    assert_eq!(tracked, vec![0]);
    assert!(svm.get_account(&fiber1).map(|a| a.lamports).unwrap_or(0) == 0);
}

/// Same mismatch on the swap path, which also sweeps an account.
#[test]
fn fiber_swap_rejects_a_source_index_that_does_not_match_the_account() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    let thread = setup_thread(&mut svm, &authority, &payer, "adv-swapdesync");
    let fiber0 = create_fiber(&mut svm, &authority, &payer, &thread, 0);
    let fiber1 = create_fiber(&mut svm, &authority, &payer, &thread, 1);
    let fiber2 = create_fiber(&mut svm, &authority, &payer, &thread, 2);

    // Sweep fiber 2's account while telling the thread that index 1 went away.
    let ix = build_swap_fiber(&authority.pubkey(), &thread, &fiber0, &fiber2, 1);
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    assert!(
        svm.send_transaction(tx).is_err(),
        "sweeping fiber 2's account while retiring index 1 must be rejected"
    );

    let tracked = deserialize_thread(&svm, &thread).fiber_ids;
    assert!(
        tracked.contains(&1) && tracked.contains(&2),
        "got {tracked:?}"
    );
    assert!(svm.get_account(&fiber1).unwrap().lamports > 0);
    assert!(svm.get_account(&fiber2).unwrap().lamports > 0);

    // The matching pair still works: source index 2 with fiber 2's account.
    let ix = build_swap_fiber(&authority.pubkey(), &thread, &fiber0, &fiber2, 2);
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx)
        .expect("a matching source index/account pair must still swap");
    let tracked = deserialize_thread(&svm, &thread).fiber_ids;
    assert_eq!(tracked, vec![0, 1]);
    assert!(svm.get_account(&fiber2).map(|a| a.lamports).unwrap_or(0) == 0);
    assert!(svm.get_account(&fiber0).unwrap().lamports > 0);
}
