use std::sync::Arc;

use agave_geyser_plugin_interface::geyser_plugin_interface::{
    GeyserPluginError, Result as PluginResult,
};
use anchor_lang::{system_program, InstructionData, ToAccountMetas};
use antegen_network_program::{state::Builder, ANTEGEN_SQUADS};
use antegen_thread_program::state::{Thread, Trigger};
use antegen_utils::thread::PAYER_PUBKEY;
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
) -> PluginResult<Option<VersionedTransaction>> {
    let now = std::time::Instant::now();
    let signatory_pubkey = payer.pubkey();
    let builder_pubkey = Builder::pubkey(builder_id);

    let builder_account = match client.get_account(&builder_pubkey).await {
        Ok(account) => account,
        Err(err) => {
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
    let first_instruction = if thread.next_instruction.is_some() {
        build_exec_ix(
            thread.clone(),
            thread_pubkey,
            signatory_pubkey,
            builder_pubkey,
            builder.authority,
        )
    } else {
        build_kickoff_ix(
            thread.clone(),
            thread_pubkey,
            signatory_pubkey,
            builder_pubkey,
        )
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
        .map_err(|e| {
            GeyserPluginError::Custom(format!("Failed to compile message: {}", e).into())
        })?;

        let versioned_message = VersionedMessage::V0(message);
        // Create versioned transaction
        let sim_tx = VersionedTransaction::try_new(versioned_message, &[payer]).map_err(|e| {
            GeyserPluginError::Custom(format!("Failed to create transaction: {}", e).into())
        })?;

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
                        return Err(GeyserPluginError::Custom(
                            "RPC client has not reached min context slot".into(),
                        ));
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

                // Check if next instruction exists
                match sim_thread.next_instruction {
                    Some(_) => (),
                    None => {
                        break;
                    }
                };

                // Check execution context and rate limit
                let exec_context = match sim_thread.exec_context {
                    Some(context) => context,
                    None => {
                        info!("No exec context found, breaking");
                        break;
                    }
                };

                if !exec_context.execs_since_slot.lt(&sim_thread.rate_limit) {
                    info!("Rate limit reached, breaking");
                    break;
                }

                ixs.push(build_exec_ix(
                    sim_thread,
                    thread_pubkey,
                    signatory_pubkey,
                    builder_pubkey,
                    builder.authority,
                ));
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
            .map_err(|e| {
                GeyserPluginError::Custom(format!("Failed to compile final message: {}", e).into())
            })?;

    let versioned_message = VersionedMessage::V0(message);
    let signed_tx = VersionedTransaction::try_new(versioned_message, &[payer]).map_err(|e| {
        GeyserPluginError::Custom(format!("Failed to create final transaction: {}", e).into())
    })?;

    // @todo - this will be submitted to NATs to be submitted via executors.
    info!(
        "Successfully built transaction - slot: {:?} thread: {:?} sim_duration: {:?} instruction_count: {:?} compute_units: {:?} tx_sig: {:?}",
        slot,
        thread_pubkey,
        now.elapsed(),
        successful_ixs.len(),
        units_consumed,
        signed_tx.signatures[0]
    );

    let thread_builder_claim_ix: Instruction = build_thread_claim(
        signatory_pubkey,
        nonce_blockhash.to_string(),
        thread.nonce_account,
        thread_pubkey,
    );
    let blockhash = client.get_latest_blockhash().await.unwrap();
    let claim: v0::Message = v0::Message::try_compile(
        &signatory_pubkey,
        &[thread_builder_claim_ix],
        &[],
        blockhash,
    )
    .map_err(|e| {
        GeyserPluginError::Custom(format!("Failed to compile claim message: {}", e).into())
    })?;

    let versioned_claim = VersionedMessage::V0(claim);
    let tx = VersionedTransaction::try_new(versioned_claim, &[payer]).map_err(|e| {
        GeyserPluginError::Custom(format!("Failed to create claim transaction: {}", e).into())
    })?;

    Ok(Some(tx))
}

fn build_thread_claim(
    signatory: Pubkey,
    hash: String,
    nonce_account: Pubkey,
    thread: Pubkey,
) -> Instruction {
    return Instruction {
        program_id: antegen_thread_program::ID,
        accounts: antegen_thread_program::accounts::ThreadClaim {
            authority: signatory,
            signatory,
            nonce_account,
            thread,
            system_program: system_program::ID,
        }
        .to_account_metas(Some(false)),
        data: antegen_thread_program::instruction::ThreadClaim { hash }.data(),
    };
}

fn build_kickoff_ix(
    thread: Thread,
    thread_pubkey: Pubkey,
    signatory_pubkey: Pubkey,
    builder_pubkey: Pubkey,
) -> Instruction {
    let mut kickoff_ix = Instruction {
        program_id: antegen_thread_program::ID,
        accounts: antegen_thread_program::accounts::ThreadKickoff {
            signatory: signatory_pubkey,
            thread: thread_pubkey,
            builder: builder_pubkey,
            nonce_account: thread.nonce_account,
            system_program: system_program::ID,
        }
        .to_account_metas(Some(false)),
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
        Trigger::Pyth {
            price_feed,
            equality: _,
            limit: _,
        } => kickoff_ix.accounts.push(AccountMeta {
            pubkey: price_feed,
            is_signer: false,
            is_writable: false,
        }),
        _ => {}
    }

    kickoff_ix
}

fn build_exec_ix(
    thread: Thread,
    thread_pubkey: Pubkey,
    signatory_pubkey: Pubkey,
    builder_pubkey: Pubkey,
    authority_pubkey: Pubkey,
) -> Instruction {
    // Build the instruction.
    let mut exec_ix = Instruction {
        program_id: antegen_thread_program::ID,
        accounts: antegen_thread_program::accounts::ThreadExec {
            authority: authority_pubkey,
            signatory: signatory_pubkey,
            thread: thread_pubkey,
            builder: builder_pubkey,
            network_fee: ANTEGEN_SQUADS,
        }
        .to_account_metas(Some(true)),
        data: antegen_thread_program::instruction::ThreadExec {}.data(),
    };

    if let Some(next_instruction) = thread.next_instruction {
        // Inject the target program account.
        exec_ix.accounts.push(AccountMeta::new_readonly(
            next_instruction.program_id,
            false,
        ));

        // Inject the worker pubkey as the dynamic "payer" account.
        for acc in next_instruction.clone().accounts {
            let acc_pubkey = if acc.pubkey == PAYER_PUBKEY {
                signatory_pubkey
            } else {
                acc.pubkey
            };
            exec_ix.accounts.push(match acc.is_writable {
                true => AccountMeta::new(acc_pubkey, false),
                false => AccountMeta::new_readonly(acc_pubkey, false),
            })
        }
    }

    exec_ix
}
