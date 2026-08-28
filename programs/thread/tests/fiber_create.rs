use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};

mod common;
use common::*;


/// Deterministic stand-in for `Pubkey::new_unique()`.
///
/// Still distinct on every call, but reproducible from run to run, so a
/// failing assertion reports the same addresses each time.
fn unique_pubkey() -> Pubkey {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(&NEXT.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    Pubkey::new_from_array(bytes)
}


/// Helper to create a thread with no fibers and return thread_pubkey.
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
        30_000_000, // enough to fund up to 3 fibers
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

/// Helper to send create_fiber.
fn send_create_fiber(
    svm: &mut litesvm::LiteSVM,
    authority: &Keypair,
    payer: &Keypair,
    thread: &Pubkey,
    fiber_index: u8,
    priority_fee: u64,
) -> Result<Pubkey, litesvm::types::FailedTransactionMetadata> {
    let (fiber_pubkey, _) = fiber_pda(thread, fiber_index);
    let memo_ix = make_memo_instruction("fiber-test", None);
    let serializable = make_serializable_instruction(&memo_ix);

    let ix = build_create_fiber(
        &authority.pubkey(),
        thread,
        &fiber_pubkey,
        fiber_index,
        serializable,
        priority_fee,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[payer, authority],
        blockhash,
    );
    svm.send_transaction(tx).map(|_| fiber_pubkey)
}

#[test]
fn test_fiber_create_success() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-1");
    let fiber_pubkey =
        send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 0, 100).unwrap();

    let fiber = deserialize_fiber(&svm, &fiber_pubkey);
    assert_eq!(fiber.thread, thread_pubkey);
    assert_eq!(fiber.priority_fee, 100);
    assert_eq!(fiber.last_executed, 0);
    assert_eq!(fiber.exec_count, 0);
    assert!(!fiber.compiled_instruction.is_empty());
}

#[test]
fn test_fiber_create_sequential_index() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-seq");

    send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 0, 0).unwrap();
    send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 1, 0).unwrap();
    send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 2, 0).unwrap();

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.fiber_ids, vec![0, 1, 2]);
    assert_eq!(thread.fiber_next_id, 3);
}

#[test]
fn test_fiber_create_non_sequential_index() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-nonseq");

    // fiber_next_id is 0, create index 1 directly — should succeed (relaxed constraint)
    send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 1, 0).unwrap();

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.fiber_ids, vec![1]);
    assert_eq!(thread.fiber_next_id, 2); // bumped past index 1
}

#[test]
fn test_fiber_create_skip_index() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-skip");
    send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 0, 0).unwrap();

    // Skip index 1 and create index 2 — should succeed (relaxed constraint)
    send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 2, 0).unwrap();

    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.fiber_ids, vec![0, 2]);
    assert_eq!(thread.fiber_next_id, 3); // bumped past index 2
}

#[test]
fn test_fiber_create_authority_check() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    let bad_authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&bad_authority.pubkey(), DEFAULT_AIRDROP)
        .unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-auth");

    // Use wrong authority
    let result = send_create_fiber(&mut svm, &bad_authority, &payer, &thread_pubkey, 0, 0);
    assert!(result.is_err());
}

#[test]
fn test_fiber_create_prevents_delete_thread() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-del");
    let (config_pubkey, _) = config_pda();

    // Build a delete_thread instruction as the fiber content
    let delete_ix = build_delete_thread(&authority.pubkey(), &config_pubkey, &thread_pubkey);
    let serializable = make_serializable_instruction(&delete_ix);

    let (fiber_pubkey, _) = fiber_pda(&thread_pubkey, 0);
    let ix = build_create_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber_pubkey,
        0,
        serializable,
        0,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    let result = svm.send_transaction(tx);
    assert!(result.is_err());
}

#[test]
fn test_fiber_create_with_priority_fee() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-pf");
    let fiber_pubkey =
        send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 0, 50000).unwrap();

    let fiber = deserialize_fiber(&svm, &fiber_pubkey);
    assert_eq!(fiber.priority_fee, 50000);
}

#[test]
fn test_fiber_create_pda_derivation() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-pda");
    let fiber_pubkey =
        send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 0, 0).unwrap();

    let expected = antegen_fiber_program::state::FiberState::pubkey(thread_pubkey, 0);
    assert_eq!(fiber_pubkey, expected);
}

#[test]
fn test_fiber_create_updates_thread() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-upd");
    let thread_before = deserialize_thread(&svm, &thread_pubkey);
    assert!(thread_before.fiber_ids.is_empty());
    assert_eq!(thread_before.fiber_next_id, 0);

    send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 0, 0).unwrap();

    let thread_after = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread_after.fiber_ids, vec![0]);
    assert_eq!(thread_after.fiber_next_id, 1);
}

// ============================================================================
// lookup_tables (ALT support) tests
// ============================================================================

/// Send a create_fiber with an explicit lookup_tables list.
fn send_create_fiber_with_alts(
    svm: &mut litesvm::LiteSVM,
    authority: &Keypair,
    payer: &Keypair,
    thread: &Pubkey,
    fiber_index: u8,
    lookup_tables: Vec<Pubkey>,
) -> Result<Pubkey, litesvm::types::FailedTransactionMetadata> {
    let (fiber_pubkey, _) = fiber_pda(thread, fiber_index);
    let memo_ix = make_memo_instruction("fiber-alt", None);
    let serializable = make_serializable_instruction(&memo_ix);

    let ix = build_create_fiber_with_alts(
        &authority.pubkey(),
        thread,
        &fiber_pubkey,
        fiber_index,
        serializable,
        0,
        lookup_tables,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[payer, authority],
        blockhash,
    );
    svm.send_transaction(tx).map(|_| fiber_pubkey)
}

#[test]
fn test_fiber_create_stores_lookup_tables() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-alt-store");
    let alt_a = unique_pubkey();
    let alt_b = unique_pubkey();
    let fiber_pubkey = send_create_fiber_with_alts(
        &mut svm,
        &authority,
        &payer,
        &thread_pubkey,
        0,
        vec![alt_a, alt_b],
    )
    .unwrap();

    let read = deserialize_fiber_any(&svm, &fiber_pubkey);
    assert!(!read.is_legacy(), "new fiber should be V1");
    assert_eq!(read.lookup_tables(), &[alt_a, alt_b]);
}

#[test]
fn test_fiber_create_at_alt_boundary_succeeds() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-alt-4");
    let four_alts: Vec<Pubkey> = (0..4).map(|_| unique_pubkey()).collect();
    let fiber_pubkey = send_create_fiber_with_alts(
        &mut svm,
        &authority,
        &payer,
        &thread_pubkey,
        0,
        four_alts.clone(),
    )
    .unwrap();

    let read = deserialize_fiber_any(&svm, &fiber_pubkey);
    assert_eq!(read.lookup_tables(), four_alts.as_slice());
}

#[test]
fn test_fiber_create_rejects_more_than_four_alts() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-alt-5");
    let five_alts: Vec<Pubkey> = (0..5).map(|_| unique_pubkey()).collect();
    let result =
        send_create_fiber_with_alts(&mut svm, &authority, &payer, &thread_pubkey, 0, five_alts);
    assert!(
        result.is_err(),
        "creating a fiber with 5 ALTs must be rejected (max 4)"
    );
}

#[test]
fn test_fiber_create_empty_lookup_tables_default() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-alt-empty");
    let fiber_pubkey =
        send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 0, 0).unwrap();

    let read = deserialize_fiber_any(&svm, &fiber_pubkey);
    // V1 fiber with empty lookup_tables (default path).
    assert!(!read.is_legacy());
    assert!(read.lookup_tables().is_empty());
}

#[test]
fn test_fiber_create_compiled_roundtrip() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-rt");
    let fiber_pubkey =
        send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 0, 0).unwrap();

    let fiber = deserialize_fiber(&svm, &fiber_pubkey);
    // Compiled bytes should be deserializable
    let compiled = borsh::from_slice::<antegen_fiber_program::state::CompiledInstructionV0>(
        &fiber.compiled_instruction,
    );
    assert!(compiled.is_ok());
}

/// A wallet that is not the owning thread must not be able to claim an existing
/// fiber by calling the fiber program directly.
///
/// Reproduces a live mainnet drain: the attacker sent `fiber::create` with their
/// own wallet as `thread` and a victim thread's fiber PDA as `fiber`. The
/// already-initialized branch rewrote `state.thread` to the signer without
/// checking that the account derives from them, after which `fiber::close`
/// passed its `read.thread() == signer` check and swept the rent. The victim
/// thread was left listing fiber ids whose accounts no longer exist, so every
/// executor failed to build a transaction for it from then on.
#[test]
fn test_fiber_create_rejects_foreign_fiber_account() {
    use anchor_lang::{InstructionData, ToAccountMetas};
    use solana_sdk::instruction::Instruction;

    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-steal");
    let fiber_pubkey =
        send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 0, 100).unwrap();
    let rent_before = svm.get_account(&fiber_pubkey).unwrap().lamports;
    assert!(rent_before > 0);

    let attacker = Keypair::new();
    svm.airdrop(&attacker.pubkey(), DEFAULT_AIRDROP).unwrap();

    let memo_ix = make_memo_instruction("stolen", None);
    let hijack = Instruction {
        program_id: FIBER_PROGRAM_ID,
        accounts: antegen_fiber_program::accounts::Create {
            thread: attacker.pubkey(),
            fiber: fiber_pubkey,
            system_program: solana_system_interface::program::ID,
        }
        .to_account_metas(None),
        data: antegen_fiber_program::instruction::Create {
            fiber_index: 0,
            instruction: make_serializable_instruction(&memo_ix),
            priority_fee: 0,
            lookup_tables: antegen_fiber_program::state::Trailing(Vec::new()),
        }
        .data(),
    };
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[hijack],
        Some(&attacker.pubkey()),
        &[&attacker],
        blockhash,
    );
    assert!(
        svm.send_transaction(tx).is_err(),
        "a wallet that does not derive the fiber PDA must not be able to claim it"
    );

    // The fiber still belongs to its thread, so the follow-up close cannot pass
    // the fiber program's ownership check either.
    let read = deserialize_fiber_any(&svm, &fiber_pubkey);
    assert_eq!(read.thread(), thread_pubkey);

    let steal_rent = Instruction {
        program_id: FIBER_PROGRAM_ID,
        accounts: antegen_fiber_program::accounts::Close {
            thread: attacker.pubkey(),
            fiber: fiber_pubkey,
        }
        .to_account_metas(None),
        data: antegen_fiber_program::instruction::Close { fiber_index: 0 }.data(),
    };
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[steal_rent],
        Some(&attacker.pubkey()),
        &[&attacker],
        blockhash,
    );
    assert!(
        svm.send_transaction(tx).is_err(),
        "rent must not be sweepable by a stranger"
    );
    assert_eq!(
        svm.get_account(&fiber_pubkey).unwrap().lamports,
        rent_before
    );
}

/// Re-creating an existing fiber must still work through the legitimate path.
///
/// `create` is idempotent by design: the already-initialized branch overwrites
/// the instruction in place rather than failing, and both `fiber_create` and
/// `thread_create` rely on that (they skip pre-funding when the account already
/// has data). The PDA check that closes the hijack sits in front of that
/// branch, so this is the case that proves it did not close the front door too.
#[test]
fn test_fiber_create_overwrites_existing_fiber_in_place() {
    let (mut svm, _admin, payer) = create_test_env();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();

    let thread_pubkey = setup_thread(&mut svm, &authority, &payer, "fc-again");
    let fiber_pubkey =
        send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 0, 100).unwrap();
    let rent_after_first = svm.get_account(&fiber_pubkey).unwrap().lamports;

    let again = send_create_fiber(&mut svm, &authority, &payer, &thread_pubkey, 0, 777)
        .expect("re-creating an owned fiber must succeed");
    assert_eq!(again, fiber_pubkey);

    let read = deserialize_fiber_any(&svm, &fiber_pubkey);
    assert_eq!(read.thread(), thread_pubkey);
    assert_eq!(read.priority_fee(), 777, "in-place overwrite should apply");
    assert_eq!(
        svm.get_account(&fiber_pubkey).unwrap().lamports,
        rent_after_first,
        "an initialized fiber must not be pre-funded a second time"
    );

    // The thread still tracks the index exactly once.
    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.fiber_ids.iter().filter(|&&i| i == 0).count(), 1);
}
