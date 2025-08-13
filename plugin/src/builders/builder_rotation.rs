use std::sync::Arc;

use anchor_lang::{solana_program::instruction::Instruction, InstructionData, ToAccountMetas};
use antegen_network_program::state::*;
use log::info;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{signature::Keypair, signer::Signer, transaction::Transaction};

pub async fn build_builder_rotation_tx<'a>(
    client: Arc<RpcClient>,
    keypair: &Keypair,
    builder_id: u32,
) -> Option<Transaction> {
    // Get the builder's current state
    let builder_pubkey = Builder::pubkey(builder_id);
    let builder = match client.get_account(&builder_pubkey).await {
        Ok(account) => {
            match Builder::try_from(account.data.as_slice()) {
                Ok(b) => b,
                Err(e) => {
                    info!("Failed to deserialize builder account: {:?}", e);
                    return None;
                }
            }
        }
        Err(e) => {
            info!("Failed to get builder account: {:?}", e);
            return None;
        }
    };

    info!("Builder {} is_active: {}", builder_id, builder.is_active);

    // Exit early if the builder is already active
    if builder.is_active {
        return None;
    }

    // Build activation instruction to activate the builder
    let ix = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::BuilderActivate {
            signatory: keypair.pubkey(),
            builder: builder_pubkey,
        }
        .to_account_metas(Some(false)),
        data: antegen_network_program::instruction::BuilderActivate {}.data(),
    };

    // Build and sign tx
    let mut tx = Transaction::new_with_payer(&[ix.clone()], Some(&keypair.pubkey()));
    tx.sign(&[keypair], client.get_latest_blockhash().await.unwrap());
    return Some(tx);
}