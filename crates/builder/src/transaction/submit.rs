use anchor_lang::{AccountDeserialize, InstructionData};
use antegen_network_program::state::{Builder, Registry};
use antegen_thread_program::state::Thread;
use anyhow::{anyhow, Result};
use log::info;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use solana_sdk::{
    message::v0, signature::Keypair, signer::Signer, transaction::VersionedTransaction,
};
use std::sync::Arc;

/// Build a thread_submit wrapper transaction that includes fee distribution
pub async fn build_thread_submit_tx(
    client: Arc<RpcClient>,
    submitter: &Keypair,
    thread_pubkey: Pubkey,
    thread: Thread,
    builder_id: u32,
    thread_exec_ix_data: Vec<u8>,
    remaining_accounts: Vec<AccountMeta>,
) -> Result<VersionedTransaction> {
    let submitter_pubkey = submitter.pubkey();
    let builder_pubkey = Builder::pubkey(builder_id);
    let registry_pubkey = Registry::pubkey();

    // Get builder account to get authority
    let builder_account = client
        .get_account(&builder_pubkey)
        .await
        .map_err(|e| anyhow!("Failed to get builder account: {}", e))?;
    let builder = Builder::try_from(builder_account.data.as_slice())
        .map_err(|e| anyhow!("Failed to parse builder account: {}", e))?;

    // Get registry to get admin
    let registry_account = client
        .get_account(&registry_pubkey)
        .await
        .map_err(|e| anyhow!("Failed to get registry account: {}", e))?;
    let registry = Registry::try_deserialize(&mut registry_account.data.as_slice())
        .map_err(|e| anyhow!("Failed to parse registry account: {}", e))?;

    // Build thread_submit instruction
    let mut accounts = vec![
        AccountMeta::new(submitter_pubkey, true),
        AccountMeta::new(thread_pubkey, false),
        AccountMeta::new_readonly(builder_pubkey, false),
        AccountMeta::new_readonly(registry_pubkey, false),
        AccountMeta::new(thread.authority, false), // thread authority for fee
        AccountMeta::new(builder.authority, false), // builder authority for fee
        AccountMeta::new(registry.admin, false),   // registry admin for fee
        AccountMeta::new_readonly(antegen_thread_program::ID, false), // thread program for CPI
    ];

    // Add remaining accounts for the inner thread_exec instruction
    accounts.extend(remaining_accounts);

    let thread_submit_ix = Instruction {
        program_id: antegen_thread_program::ID,
        accounts,
        data: antegen_thread_program::instruction::ThreadSubmit {
            thread_exec_ix_data,
        }
        .data(),
    };

    // Get latest blockhash
    let blockhash = client
        .get_latest_blockhash()
        .await
        .map_err(|e| anyhow!("Failed to get latest blockhash: {}", e))?;

    // Build and sign transaction
    let message = v0::Message::try_compile(&submitter_pubkey, &[thread_submit_ix], &[], blockhash)
        .map_err(|e| anyhow!("Failed to compile message: {}", e))?;

    let tx = VersionedTransaction::try_new(
        solana_sdk::message::VersionedMessage::V0(message),
        &[submitter],
    )
    .map_err(|e| anyhow!("Failed to create transaction: {}", e))?;

    info!(
        "Built thread_submit transaction - thread: {} builder: {} submitter: {}",
        thread_pubkey, builder_id, submitter_pubkey
    );

    Ok(tx)
}
