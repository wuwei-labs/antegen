use anchor_lang::InstructionData;
use antegen_thread_program::fiber;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};

mod common;
use common::*;

/// Build an instruction that calls test_processor::cpi_update_fiber.
/// This instruction will be stored in a fiber and invoked during thread_exec.
fn make_cpi_update_fiber_instruction(
    thread: &Pubkey,
    target_fiber: &Pubkey,
    fiber_index: u8,
) -> Instruction {
    Instruction {
        program_id: TEST_PROCESSOR_ID,
        accounts: vec![
            AccountMeta::new(*thread, true),                    // thread as signer
            AccountMeta::new(*target_fiber, false),              // fiber to update
            AccountMeta::new_readonly(fiber::ID, false),         // fiber program
            AccountMeta::new_readonly(solana_system_interface::program::ID, false), // system program
        ],
        data: antegen_reentrance_test::instruction::CpiUpdateFiber { fiber_index }.data(),
    }
}

/// Build an instruction that calls test_processor::cpi_close_fiber.
fn make_cpi_close_fiber_instruction(
    thread: &Pubkey,
    target_fiber: &Pubkey,
) -> Instruction {
    Instruction {
        program_id: TEST_PROCESSOR_ID,
        accounts: vec![
            AccountMeta::new(*thread, true),                    // thread as signer
            AccountMeta::new(*target_fiber, false),              // fiber to close
            AccountMeta::new_readonly(fiber::ID, false),         // fiber program
        ],
        data: antegen_reentrance_test::instruction::CpiCloseFiber {}.data(),
    }
}

/// Build an instruction that calls test_processor::cpi_swap_fiber.
fn make_cpi_swap_fiber_instruction(
    thread: &Pubkey,
    target_fiber: &Pubkey,
    source_fiber: &Pubkey,
) -> Instruction {
    Instruction {
        program_id: TEST_PROCESSOR_ID,
        accounts: vec![
            AccountMeta::new(*thread, true),                    // thread as signer
            AccountMeta::new(*target_fiber, false),              // target fiber
            AccountMeta::new(*source_fiber, false),              // source fiber
            AccountMeta::new_readonly(fiber::ID, false),         // fiber program
        ],
        data: antegen_reentrance_test::instruction::CpiSwapFiber {}.data(),
    }
}

/// Build remaining_accounts for thread_exec when calling a test_processor instruction.
/// These accounts must be present in the transaction for invoke_signed to work.
fn build_test_processor_remaining_accounts(
    thread: &Pubkey,
    inner_accounts: &[AccountMeta],
) -> Vec<AccountMeta> {
    let mut remaining = vec![
        AccountMeta::new_readonly(TEST_PROCESSOR_ID, false), // program being called
    ];
    // Add the thread as writable (needed for inner CPI)
    remaining.push(AccountMeta::new(*thread, false));
    // Add all other accounts from the inner instruction (skip thread which we already added)
    for acc in inner_accounts {
        if acc.pubkey != *thread {
            remaining.push(acc.clone());
        }
    }
    // Add fiber program as account (needed for nested CPI)
    if !remaining.iter().any(|a| a.pubkey == fiber::ID) {
        remaining.push(AccountMeta::new_readonly(fiber::ID, false));
    }
    remaining
}

#[test]
fn test_reentrancy_update_fiber_during_exec() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    // 1. Create thread with Interval trigger
    let thread_id = ThreadId::Bytes(b"reentrant-upd".to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), b"reentrant-upd");
    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        10_000_000, // enough SOL for rent + fees
        thread_id,
        Trigger::Interval {
            seconds: 10,
            skippable: false,
            jitter: 0,
        },
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    // 2. Create fiber_1 first (target: simple memo)
    let (fiber_1_pubkey, _) = fiber_pda(&thread_pubkey, 0);
    let memo_ix = make_memo_instruction("target-memo", None);
    let ser_memo = make_serializable_instruction(&memo_ix);
    let ix = build_create_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber_1_pubkey,
        0,
        ser_memo,
        0,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    // 3. Create fiber_0 (source: calls test_processor::cpi_update_fiber targeting fiber_1)
    let (fiber_0_pubkey, _) = fiber_pda(&thread_pubkey, 1);
    let cpi_ix = make_cpi_update_fiber_instruction(&thread_pubkey, &fiber_1_pubkey, 0);
    let ser_cpi = make_serializable_instruction(&cpi_ix);
    let ix = build_create_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber_0_pubkey,
        1,
        ser_cpi,
        0,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    // Record fiber_1's compiled instruction before exec
    let fiber_1_before = deserialize_fiber(&svm, &fiber_1_pubkey);
    let compiled_before = fiber_1_before.compiled_instruction.clone();

    // 4. Advance clock past interval
    advance_clock(&mut svm, 15);

    // 5. Execute thread_exec with fiber_cursor=1 (the CPI fiber)
    let (config_pubkey, _) = config_pda();
    let inner_accounts = vec![
        AccountMeta::new(thread_pubkey, true),
        AccountMeta::new(fiber_1_pubkey, false),
        AccountMeta::new_readonly(fiber::ID, false),
        AccountMeta::new_readonly(solana_system_interface::program::ID, false),
    ];
    let remaining = build_test_processor_remaining_accounts(&thread_pubkey, &inner_accounts);

    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        &fiber_0_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        false,
        1, // fiber_cursor = fiber_0's index
        &remaining,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&executor.pubkey()),
        &[&executor],
        blockhash,
    );

    // This is the key assertion: no ReentrancyNotAllowed error
    svm.send_transaction(tx)
        .expect("thread_exec with inner CPI to fiber_program should succeed (no reentrancy error)");

    // 6. Verify fiber_1 was updated by the inner CPI
    let fiber_1_after = deserialize_fiber(&svm, &fiber_1_pubkey);
    assert_ne!(
        fiber_1_after.compiled_instruction, compiled_before,
        "fiber_1's compiled instruction should be changed by the inner CPI update"
    );
    assert_eq!(fiber_1_after.priority_fee, 42, "priority_fee should be set to 42 by test_processor");
    assert_eq!(fiber_1_after.last_executed, 0, "execution stats should be reset");
    assert_eq!(fiber_1_after.exec_count, 0, "execution stats should be reset");

    // Verify thread exec_count incremented
    let thread = deserialize_thread(&svm, &thread_pubkey);
    assert_eq!(thread.exec_count, 1);
}

#[test]
fn test_reentrancy_close_fiber_during_exec() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    // 1. Create thread with Interval trigger
    let thread_id = ThreadId::Bytes(b"reentrant-cls".to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), b"reentrant-cls");
    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        10_000_000,
        thread_id,
        Trigger::Interval {
            seconds: 10,
            skippable: false,
            jitter: 0,
        },
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    // 2. Create fiber_0 (target to close)
    let (fiber_0_pubkey, _) = fiber_pda(&thread_pubkey, 0);
    let memo_ix = make_memo_instruction("will-be-closed", None);
    let ser_memo = make_serializable_instruction(&memo_ix);
    let ix = build_create_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber_0_pubkey,
        0,
        ser_memo,
        0,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    // 3. Create fiber_1 (CPI: close fiber_0)
    let (fiber_1_pubkey, _) = fiber_pda(&thread_pubkey, 1);
    let cpi_ix = make_cpi_close_fiber_instruction(&thread_pubkey, &fiber_0_pubkey);
    let ser_cpi = make_serializable_instruction(&cpi_ix);
    let ix = build_create_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber_1_pubkey,
        1,
        ser_cpi,
        0,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    // Verify fiber_0 exists before exec
    assert!(account_exists(&svm, &fiber_0_pubkey));

    // 4. Advance clock past interval
    advance_clock(&mut svm, 15);

    // 5. Execute thread_exec with fiber_cursor=1 (the CPI close fiber)
    let (config_pubkey, _) = config_pda();
    let inner_accounts = vec![
        AccountMeta::new(thread_pubkey, true),
        AccountMeta::new(fiber_0_pubkey, false),
        AccountMeta::new_readonly(fiber::ID, false),
    ];
    let remaining = build_test_processor_remaining_accounts(&thread_pubkey, &inner_accounts);

    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        &fiber_1_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        false,
        1,
        &remaining,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&executor.pubkey()),
        &[&executor],
        blockhash,
    );

    // Key assertion: no ReentrancyNotAllowed
    svm.send_transaction(tx)
        .expect("thread_exec with inner CPI close_fiber should succeed (no reentrancy error)");

    // 6. Verify fiber_0 was closed by the inner CPI
    assert!(
        !account_exists(&svm, &fiber_0_pubkey),
        "fiber_0 should be closed by the inner CPI"
    );
}

#[test]
fn test_reentrancy_swap_fiber_during_exec() {
    let (mut svm, admin, payer) = create_test_env();
    let authority = Keypair::new();
    let executor = Keypair::new();
    svm.airdrop(&authority.pubkey(), DEFAULT_AIRDROP).unwrap();
    svm.airdrop(&executor.pubkey(), DEFAULT_AIRDROP).unwrap();

    // 1. Create thread with Interval trigger
    let thread_id = ThreadId::Bytes(b"reentrant-swp".to_vec());
    let (thread_pubkey, _) = thread_pda(&authority.pubkey(), b"reentrant-swp");
    let ix = build_create_thread(
        &authority.pubkey(),
        &payer.pubkey(),
        &thread_pubkey,
        10_000_000,
        thread_id,
        Trigger::Interval {
            seconds: 10,
            skippable: false,
            jitter: 0,
        },
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    // 2. Create 3 fibers: target (0), source (1), executor (2 = CPI swap)
    // fiber_0: target (will receive source's instruction)
    let (fiber_0_pubkey, _) = fiber_pda(&thread_pubkey, 0);
    let memo_ix_0 = make_memo_instruction("target", None);
    let ser_0 = make_serializable_instruction(&memo_ix_0);
    let ix = build_create_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber_0_pubkey,
        0,
        ser_0,
        0,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    // fiber_1: source (will be closed by swap)
    let (fiber_1_pubkey, _) = fiber_pda(&thread_pubkey, 1);
    let memo_ix_1 = make_memo_instruction("source-to-swap", None);
    let ser_1 = make_serializable_instruction(&memo_ix_1);
    let ix = build_create_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber_1_pubkey,
        1,
        ser_1,
        7777,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    // fiber_2: CPI instruction that swaps fiber_1 -> fiber_0
    let (fiber_2_pubkey, _) = fiber_pda(&thread_pubkey, 2);
    let cpi_ix =
        make_cpi_swap_fiber_instruction(&thread_pubkey, &fiber_0_pubkey, &fiber_1_pubkey);
    let ser_cpi = make_serializable_instruction(&cpi_ix);
    let ix = build_create_fiber(
        &authority.pubkey(),
        &thread_pubkey,
        &fiber_2_pubkey,
        2,
        ser_cpi,
        0,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer, &authority],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    // Record source fiber's compiled instruction before swap
    let source_before = deserialize_fiber(&svm, &fiber_1_pubkey);
    let source_compiled = source_before.compiled_instruction.clone();

    // 4. Advance clock past interval
    advance_clock(&mut svm, 15);

    // 5. Execute thread_exec with fiber_cursor=2 (the CPI swap fiber)
    let (config_pubkey, _) = config_pda();
    let inner_accounts = vec![
        AccountMeta::new(thread_pubkey, true),
        AccountMeta::new(fiber_0_pubkey, false),
        AccountMeta::new(fiber_1_pubkey, false),
        AccountMeta::new_readonly(fiber::ID, false),
    ];
    let remaining = build_test_processor_remaining_accounts(&thread_pubkey, &inner_accounts);

    let ix = build_exec_thread(
        &executor.pubkey(),
        &thread_pubkey,
        &fiber_2_pubkey,
        &config_pubkey,
        &admin.pubkey(),
        false,
        2,
        &remaining,
    );
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&executor.pubkey()),
        &[&executor],
        blockhash,
    );

    // Key assertion: no ReentrancyNotAllowed
    svm.send_transaction(tx)
        .expect("thread_exec with inner CPI swap_fiber should succeed (no reentrancy error)");

    // 6. Verify: target now has source's instruction, source is closed
    let target_after = deserialize_fiber(&svm, &fiber_0_pubkey);
    assert_eq!(
        target_after.compiled_instruction, source_compiled,
        "target should have source's compiled instruction after swap"
    );
    assert_eq!(target_after.priority_fee, 7777);
    assert!(
        !account_exists(&svm, &fiber_1_pubkey),
        "source fiber should be closed after swap"
    );
}
