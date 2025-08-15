use anchor_lang::{system_program, InstructionData};
use antegen_network_program::state::Builder;
use antegen_thread_program::state::{Thread, Trigger};
use anyhow::{anyhow, Result};
use log::info;
use solana_account_decoder::UiAccountEncoding;
use solana_client::{
    nonblocking::rpc_client::RpcClient,
    rpc_config::{RpcSimulateTransactionAccountsConfig, RpcSimulateTransactionConfig},
    rpc_custom_error::JSON_RPC_SERVER_ERROR_MIN_CONTEXT_SLOT_NOT_REACHED,
};
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use solana_sdk::{
    account::Account,
    commitment_config::CommitmentConfig,
    compute_budget::ComputeBudgetInstruction,
    message::{v0, VersionedMessage},
    signature::Keypair,
    signer::Signer,
    system_instruction::advance_nonce_account,
    transaction::VersionedTransaction,
};
use std::cmp::min;
use std::sync::Arc;

/// Max byte size of a serialized transaction.
static TRANSACTION_MESSAGE_SIZE_LIMIT: usize = 1_232;

/// Max compute units that may be used by transaction.
static TRANSACTION_COMPUTE_UNIT_LIMIT: u32 = 1_400_000;

/// The buffer amount to add to transactions' compute units in case on-chain PDA derivations take more CUs than used in simulation.
static TRANSACTION_COMPUTE_UNIT_BUFFER: u32 = 100_000;

pub async fn build_thread_exec_tx(
    client: Arc<RpcClient>,
    payer: &Keypair,
    slot: u64,
    thread: Thread,
    thread_pubkey: Pubkey,
    builder_id: u32,
) -> Result<Option<VersionedTransaction>> {
    println!("DEBUG exec.rs: build_thread_exec_tx called for thread {}", thread_pubkey);
    let now = std::time::Instant::now();
    let signatory_pubkey = payer.pubkey();
    let builder_pubkey = Builder::pubkey(builder_id);

    let builder_account = match client.get_account(&builder_pubkey).await {
        Ok(account) => account,
        Err(err) => {
            println!("DEBUG exec.rs: Failed to get builder account {}: {:?}", builder_pubkey, err);
            info!(
                "Failed to get builder account {}: {:?}",
                builder_pubkey, err
            );
            return Ok(None);
        }
    };

    let builder: Builder = match Builder::try_from(builder_account.data.as_slice()) {
        Ok(builder) => builder,
        Err(err) => {
            info!(
                "Failed to parse uilder ccount ({}): {:?}",
                builder_pubkey.to_string(),
                err
            );
            return Ok(None);
        }
    };

    // Get nonce account and extract the blockhash
    let nonce_account = match client.get_account(&thread.nonce_account).await {
        Ok(account) => account,
        Err(err) => {
            info!("Failed to get nonce account: {:?}", err);
            return Ok(None);
        }
    };

    // Extract blockhash from nonce account using the method you provided
    let nonce_blockhash = match solana_rpc_client_nonce_utils::data_from_account(&nonce_account) {
        Ok(nonce_data) => {
            let hash = nonce_data.blockhash();
            info!("Extracted nonce blockhash: {}", hash);
            hash
        }
        Err(err) => {
            info!("Could not get nonce value: {:?}", err);
            return Ok(None);
        }
    };

    if nonce_blockhash.to_string() == thread.last_nonce {
        info!("Skipping, pending executor");
        return Ok(None);
    }

    // Build the first instruction
    // Since we don't have next_instruction anymore, always start with kickoff
    let first_instruction = {
        match client.get_balance(&signatory_pubkey).await {
            Ok(balance) => {
                info!("Signatory balance: {}", balance);
            }
            Err(err) => {
                info!("Failed to get signatory balance: {:?}", err);
            }
        }

        build_kickoff_ix(thread.clone(), thread_pubkey, signatory_pubkey)
    };

    // Initialize instructions vector
    let mut ixs: Vec<Instruction> = vec![
        ComputeBudgetInstruction::set_compute_unit_limit(TRANSACTION_COMPUTE_UNIT_LIMIT),
        first_instruction,
    ];
    let mut successful_ixs: Vec<Instruction> = vec![];
    let mut units_consumed: Option<u64> = None;

    loop {
        // Create versioned message
        let message = v0::Message::try_compile(
            &signatory_pubkey,
            &ixs,
            &[], // address table lookups
            nonce_blockhash,
        )
        .map_err(|e| anyhow!("Failed to compile message: {}", e))?;

        let versioned_message = VersionedMessage::V0(message);
        // Create versioned transaction
        let sim_tx = VersionedTransaction::try_new(versioned_message, &[payer])
            .map_err(|e| anyhow!("Failed to create transaction: {}", e))?;

        // Check transaction size limit
        let tx_size = sim_tx.message.serialize().len();
        if tx_size > TRANSACTION_MESSAGE_SIZE_LIMIT {
            info!("Transaction size exceeds limit, breaking");
            break;
        }

        // Run simulation
        match client
            .simulate_transaction_with_config(
                &sim_tx,
                RpcSimulateTransactionConfig {
                    sig_verify: false,
                    replace_recent_blockhash: true,
                    commitment: Some(CommitmentConfig::processed()),
                    accounts: Some(RpcSimulateTransactionAccountsConfig {
                        encoding: Some(UiAccountEncoding::Base64Zstd),
                        addresses: vec![thread_pubkey.to_string()],
                    }),
                    min_context_slot: Some(slot),
                    ..RpcSimulateTransactionConfig::default()
                },
            )
            .await
        {
            Err(err) => {
                info!("Simulation error encountered: {:?}", err);
                match err.kind {
                    solana_client::client_error::ClientErrorKind::RpcError(
                        solana_client::rpc_request::RpcError::RpcResponseError { code, .. },
                    ) if code == JSON_RPC_SERVER_ERROR_MIN_CONTEXT_SLOT_NOT_REACHED => {
                        return Err(anyhow!("RPC client has not reached min context slot"));
                    }
                    _ => break,
                }
            }

            Ok(response) => {
                if response.value.err.is_some() {
                    if successful_ixs.is_empty() {
                        info!(
                            "First simulation failed - slot: {} thread: {} error: {:?} logs: {:?}",
                            slot,
                            thread_pubkey,
                            response.value.err,
                            response.value.logs.unwrap_or_default()
                        );
                    }
                    break;
                }

                successful_ixs = ixs.clone();
                units_consumed = response.value.units_consumed;

                // Parse resulting thread account
                let ui_account = match response
                    .value
                    .accounts
                    .and_then(|a| a.get(0).cloned().flatten())
                {
                    Some(acc) => acc,
                    None => {
                        info!("No thread account found in response");
                        break;
                    }
                };

                let account = match ui_account.decode::<Account>() {
                    Some(acc) => acc,
                    None => {
                        info!("Failed to decode thread account");
                        break;
                    }
                };

                if account.data.len() < 8 {
                    info!("Thread account has insufficient data (likely closed), breaking");
                    break;
                }

                let sim_thread = match Thread::try_from(account.data) {
                    Ok(thread) => thread,
                    Err(e) => {
                        info!("Failed to parse thread state: {:?}", e);
                        break;
                    }
                };

                // Check if thread has more fibers to execute
                if sim_thread.exec_index >= sim_thread.fibers.len() as u8 {
                    info!("No more fibers to execute");
                    break;
                }

                // Add the next exec instruction
                match build_exec_ix(
                    client.clone(),
                    sim_thread,
                    thread_pubkey,
                    signatory_pubkey,
                    builder_pubkey,
                    builder.authority,
                ).await {
                    Ok(exec_ix) => ixs.push(exec_ix),
                    Err(e) => {
                        info!("Failed to build exec instruction: {:?}", e);
                        break;
                    }
                }
            }
        }
    }

    // Exit if no successful instructions
    if successful_ixs.is_empty() {
        info!("No successful instructions, returning None");
        return Ok(None);
    }

    // Update compute unit limit based on simulation
    if let Some(units_consumed) = units_consumed {
        let units_committed = min(
            (units_consumed as u32) + TRANSACTION_COMPUTE_UNIT_BUFFER,
            TRANSACTION_COMPUTE_UNIT_LIMIT,
        );
        successful_ixs[0] = ComputeBudgetInstruction::set_compute_unit_limit(units_committed);
    }

    successful_ixs[0] = advance_nonce_account(&thread.nonce_account, &signatory_pubkey);

    let message =
        v0::Message::try_compile(&signatory_pubkey, &successful_ixs, &[], nonce_blockhash)
            .map_err(|e| anyhow!("Failed to compile final message: {}", e))?;

    let versioned_message = VersionedMessage::V0(message);
    let signed_tx = VersionedTransaction::try_new(versioned_message, &[payer])
        .map_err(|e| anyhow!("Failed to create final transaction: {}", e))?;

    info!(
        "Successfully built transaction - slot: {:?} thread: {:?} sim_duration: {:?} instruction_count: {:?} compute_units: {:?} tx_sig: {:?}",
        slot,
        thread_pubkey,
        now.elapsed(),
        successful_ixs.len(),
        units_consumed,
        signed_tx.signatures[0]
    );

    Ok(Some(signed_tx))
}

pub fn build_thread_claim(
    signatory: Pubkey,
    thread: Pubkey,
    builder: Pubkey,
    registry: Pubkey,
) -> Instruction {
    Instruction {
        program_id: antegen_thread_program::ID,
        accounts: vec![
            AccountMeta::new(signatory, true),
            AccountMeta::new(thread, false),
            AccountMeta::new_readonly(builder, false),
            AccountMeta::new_readonly(registry, false),
        ],
        data: antegen_thread_program::instruction::ThreadClaim {}.data(),
    }
}

fn build_kickoff_ix(
    thread: Thread,
    thread_pubkey: Pubkey,
    signatory_pubkey: Pubkey,
) -> Instruction {
    info!("--- build_kickoff_ix input parameters ---");
    info!("thread: {:?}", thread);
    info!("thread_pubkey: {}", thread_pubkey);
    info!("signatory_pubkey: {}", signatory_pubkey);
    info!("--------------------------------------");

    let mut account_metas = vec![
        AccountMeta::new(signatory_pubkey, true),
        AccountMeta::new(thread_pubkey, false),
    ];

    if thread.has_nonce_account() {
        account_metas.push(AccountMeta::new(thread.nonce_account, false));
    }

    account_metas.push(AccountMeta::new_readonly(system_program::ID, false));

    let mut kickoff_ix = Instruction {
        program_id: antegen_thread_program::ID,
        accounts: account_metas,
        data: antegen_thread_program::instruction::ThreadKickoff {}.data(),
    };

    // If the thread's trigger is account-based, inject the triggering account.
    match thread.trigger {
        Trigger::Account {
            address,
            offset: _,
            size: _,
        } => kickoff_ix.accounts.push(AccountMeta {
            pubkey: address,
            is_signer: false,
            is_writable: false,
        }),
        _ => {}
    }

    kickoff_ix
}

async fn build_exec_ix(
    client: Arc<RpcClient>,
    thread: Thread,
    thread_pubkey: Pubkey,
    signatory_pubkey: Pubkey,
    _builder_pubkey: Pubkey,
    _authority_pubkey: Pubkey,
) -> Result<Instruction> {
    // Build the instruction for thread execution
    // We need to get the fiber PDA based on thread and exec_index
    let fiber_pubkey = Pubkey::find_program_address(
        &[
            b"thread_fiber",
            thread_pubkey.as_ref(),
            &[thread.exec_index],
        ],
        &antegen_thread_program::ID,
    )
    .0;

    // Fetch the fiber account to verify it exists
    match client.get_account(&fiber_pubkey).await {
        Ok(_fiber_account) => {
            info!("Fetched fiber account {} for thread {} at index {}", 
                  fiber_pubkey, thread_pubkey, thread.exec_index);
        }
        Err(err) => {
            info!("Failed to fetch fiber account {}: {:?}", fiber_pubkey, err);
            return Err(anyhow!("Fiber account not found: {}", fiber_pubkey));
        }
    }

    let exec_ix = Instruction {
        program_id: antegen_thread_program::ID,
        accounts: vec![
            AccountMeta::new(signatory_pubkey, true),
            AccountMeta::new(thread_pubkey, false),
            AccountMeta::new_readonly(fiber_pubkey, false),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
        data: antegen_thread_program::instruction::ThreadExec {}.data(),
    };

    Ok(exec_ix)
}
