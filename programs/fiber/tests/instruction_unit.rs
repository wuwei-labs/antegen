use antegen_fiber_program::state::*;
use antegen_fiber_program::*;
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::pubkey::Pubkey;

// ============================================================================
// compile / decompile tests
// ============================================================================

#[test]
fn test_compile_empty_accounts() {
    let program_id = Pubkey::new_unique();
    let ix = Instruction {
        program_id,
        accounts: vec![],
        data: vec![1, 2, 3],
    };

    let compiled = compile_instruction(ix).unwrap();

    // Only the program_id should be in accounts
    assert_eq!(compiled.accounts.len(), 1);
    assert_eq!(compiled.accounts[0], program_id);
    assert_eq!(compiled.num_rw_signers, 0);
    assert_eq!(compiled.num_ro_signers, 0);
    assert_eq!(compiled.num_rw, 0);
    assert_eq!(compiled.instructions.len(), 1);
    assert_eq!(compiled.instructions[0].data, vec![1, 2, 3]);

    // Program ID is read-only, non-signer (index 0)
    assert_eq!(compiled.instructions[0].program_id_index, 0);
}

#[test]
fn test_compile_duplicate_accounts() {
    let program_id = Pubkey::new_unique();
    let dup_key = Pubkey::new_unique();

    // Same pubkey appears twice: once as readonly, once as writable+signer
    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(dup_key, false), // ro, non-signer
            AccountMeta::new(dup_key, true),           // rw, signer
        ],
        data: vec![],
    };

    let compiled = compile_instruction(ix).unwrap();

    // Deduplication: dup_key appears once with merged flags (rw + signer wins)
    // Accounts: dup_key (rw_signer), program_id (ro)
    assert_eq!(compiled.accounts.len(), 2);
    assert_eq!(compiled.num_rw_signers, 1);
    assert_eq!(compiled.num_ro_signers, 0);
    assert_eq!(compiled.num_rw, 0);

    // The rw_signer should be first (priority 0)
    assert_eq!(compiled.accounts[0], dup_key);
}

#[test]
fn test_compile_all_permission_types() {
    let program_id = Pubkey::new_unique();
    let rw_signer = Pubkey::new_unique();
    let ro_signer = Pubkey::new_unique();
    let rw = Pubkey::new_unique();
    let ro = Pubkey::new_unique();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(rw_signer, true),          // rw + signer (priority 0)
            AccountMeta::new_readonly(ro_signer, true),  // ro + signer (priority 1)
            AccountMeta::new(rw, false),                 // rw (priority 2)
            AccountMeta::new_readonly(ro, false),        // ro (priority 3)
        ],
        data: vec![],
    };

    let compiled = compile_instruction(ix).unwrap();

    // 5 accounts: rw_signer, ro_signer, rw, ro, program_id (ro)
    assert_eq!(compiled.accounts.len(), 5);
    assert_eq!(compiled.num_rw_signers, 1);
    assert_eq!(compiled.num_ro_signers, 1);
    assert_eq!(compiled.num_rw, 1);

    // Verify sorting: rw_signers first, then ro_signers, then rw, then ro
    assert_eq!(compiled.accounts[0], rw_signer);
    assert_eq!(compiled.accounts[1], ro_signer);
    assert_eq!(compiled.accounts[2], rw);
    // ro accounts: either ro or program_id (both are priority 3)
    let ro_accounts = &compiled.accounts[3..];
    assert!(ro_accounts.contains(&ro));
    assert!(ro_accounts.contains(&program_id));
}

#[test]
fn test_compile_decompile_roundtrip_permissions() {
    let program_id = Pubkey::new_unique();
    let rw_signer = Pubkey::new_unique();
    let ro_signer = Pubkey::new_unique();
    let rw = Pubkey::new_unique();
    let ro = Pubkey::new_unique();

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(rw_signer, true),
            AccountMeta::new_readonly(ro_signer, true),
            AccountMeta::new(rw, false),
            AccountMeta::new_readonly(ro, false),
        ],
        data: vec![42, 43, 44],
    };

    let compiled = compile_instruction(ix).unwrap();
    let decompiled = decompile_instruction(&compiled).unwrap();

    // Verify program_id preserved
    assert_eq!(decompiled.program_id, program_id);

    // Verify data preserved
    assert_eq!(decompiled.data, vec![42, 43, 44]);

    // Verify all accounts present with correct permissions
    assert_eq!(decompiled.accounts.len(), 4);

    let find_account = |key: &Pubkey| -> &AccountMeta {
        decompiled
            .accounts
            .iter()
            .find(|a| a.pubkey == *key)
            .unwrap()
    };

    let rw_signer_meta = find_account(&rw_signer);
    assert!(rw_signer_meta.is_writable);
    assert!(rw_signer_meta.is_signer);

    let ro_signer_meta = find_account(&ro_signer);
    assert!(!ro_signer_meta.is_writable);
    assert!(ro_signer_meta.is_signer);

    let rw_meta = find_account(&rw);
    assert!(rw_meta.is_writable);
    assert!(!rw_meta.is_signer);

    let ro_meta = find_account(&ro);
    assert!(!ro_meta.is_writable);
    assert!(!ro_meta.is_signer);
}

#[test]
fn test_decompile_empty_instructions_vec() {
    let compiled = CompiledInstructionV0 {
        num_ro_signers: 0,
        num_rw_signers: 0,
        num_rw: 0,
        instructions: vec![], // empty!
        accounts: vec![Pubkey::new_unique()],
    };

    let result = decompile_instruction(&compiled);
    assert!(result.is_err());
}

// ============================================================================
// sentinel permission pollution tests
// ============================================================================

#[test]
fn test_sentinel_does_not_promote_program_id() {
    let program_id = Pubkey::new_unique();
    let real_account = Pubkey::new_unique();

    // Simulate Anchor optional None: program_id used as sentinel with mut flag
    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(real_account, false),    // real rw account
            AccountMeta::new(program_id, true),       // sentinel: writable + signer
        ],
        data: vec![1],
    };

    let compiled = compile_instruction(ix).unwrap();

    // program_id must stay ro non-signer (priority 3), not get promoted
    assert_eq!(compiled.num_rw_signers, 0, "sentinel should not create rw_signer");
    assert_eq!(compiled.num_ro_signers, 0, "sentinel should not create ro_signer");
    assert_eq!(compiled.num_rw, 1, "only real_account should be rw");

    // Accounts: [real_account (rw), program_id (ro)]
    assert_eq!(compiled.accounts.len(), 2);
    assert_eq!(compiled.accounts[0], real_account);
    assert_eq!(compiled.accounts[1], program_id);

    // Roundtrip: real_account keeps its permissions
    let decompiled = decompile_instruction(&compiled).unwrap();
    let real_meta = decompiled.accounts.iter().find(|a| a.pubkey == real_account).unwrap();
    assert!(real_meta.is_writable);
    assert!(!real_meta.is_signer);
}

// ============================================================================
// get_instruction tests (PAYER_PUBKEY replacement)
// ============================================================================

#[test]
fn test_payer_pubkey_replacement() {
    let program_id = Pubkey::new_unique();
    let executor = Pubkey::new_unique();

    // Create instruction with PAYER_PUBKEY as an account
    let ix = Instruction {
        program_id,
        accounts: vec![AccountMeta::new_readonly(PAYER_PUBKEY, true)],
        data: vec![1],
    };

    let compiled = compile_instruction(ix).unwrap();
    let compiled_bytes = borsh::to_vec(&compiled).unwrap();

    let fiber = FiberState {
        thread: Pubkey::new_unique(),
        compiled_instruction: compiled_bytes,
        last_executed: 0,
        exec_count: 0,
        priority_fee: 0,
    };

    let result_ix = fiber.get_instruction(&executor).unwrap();

    // PAYER_PUBKEY should be replaced with executor
    assert_eq!(result_ix.accounts[0].pubkey, executor);
    assert_eq!(result_ix.program_id, program_id);
}

#[test]
fn test_payer_pubkey_no_match() {
    let program_id = Pubkey::new_unique();
    let executor = Pubkey::new_unique();
    let some_key = Pubkey::new_unique();

    // No PAYER_PUBKEY in accounts
    let ix = Instruction {
        program_id,
        accounts: vec![AccountMeta::new_readonly(some_key, false)],
        data: vec![1],
    };

    let compiled = compile_instruction(ix).unwrap();
    let compiled_bytes = borsh::to_vec(&compiled).unwrap();

    let fiber = FiberState {
        thread: Pubkey::new_unique(),
        compiled_instruction: compiled_bytes,
        last_executed: 0,
        exec_count: 0,
        priority_fee: 0,
    };

    let result_ix = fiber.get_instruction(&executor).unwrap();

    // Account should be unchanged
    assert_eq!(result_ix.accounts[0].pubkey, some_key);
}

// ============================================================================
// PDA derivation tests
// ============================================================================

#[test]
fn test_pubkey_derivation() {
    let thread = Pubkey::new_unique();
    let fiber_index: u8 = 3;

    let derived = FiberState::pubkey(thread, fiber_index);

    // Manual derivation should match
    let (expected, _) = Pubkey::find_program_address(
        &[SEED_THREAD_FIBER, thread.as_ref(), &[fiber_index]],
        &antegen_fiber_program::ID,
    );

    assert_eq!(derived, expected);
}

#[test]
fn test_pubkey_different_indices() {
    let thread = Pubkey::new_unique();

    let pda_0 = FiberState::pubkey(thread, 0);
    let pda_1 = FiberState::pubkey(thread, 1);
    let pda_255 = FiberState::pubkey(thread, 255);

    assert_ne!(pda_0, pda_1);
    assert_ne!(pda_0, pda_255);
    assert_ne!(pda_1, pda_255);
}

#[test]
fn test_pubkey_different_threads() {
    let thread_a = Pubkey::new_unique();
    let thread_b = Pubkey::new_unique();

    let pda_a = FiberState::pubkey(thread_a, 0);
    let pda_b = FiberState::pubkey(thread_b, 0);

    assert_ne!(pda_a, pda_b);
}
