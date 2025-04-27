use std::sync::Arc;

use anchor_lang::{solana_program::instruction::Instruction, InstructionData, ToAccountMetas};
use antegen_network_program::state::*;
use log::info;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{signature::Keypair, signer::Signer, transaction::Transaction};

use crate::pool_position::PoolPosition;

pub async fn build_pool_rotation_tx<'a>(
    client: Arc<RpcClient>,
    keypair: &Keypair,
    pool_position: PoolPosition,
    builder_id: u32,
) -> Option<Transaction> {
    info!("current_position: {:?}", pool_position.current_position,);

    // Exit early if the worker is already in the pool.
    if pool_position.current_position.is_some() {
        return None;
    }

    let pool_id: u8 = 0 as u8;
    // Build rotation instruction to rotate the worker into pool 0.
    let ix = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::BuilderAdd {
            config: Config::pubkey(),
            pool: Pool::pubkey(pool_id),
            signatory: keypair.pubkey(),
            builder: Builder::pubkey(builder_id),
        }
        .to_account_metas(Some(false)),
        data: antegen_network_program::instruction::BuilderAdd {}.data(),
    };

    // Build and sign tx.
    let mut tx = Transaction::new_with_payer(&[ix.clone()], Some(&keypair.pubkey()));
    tx.sign(&[keypair], client.get_latest_blockhash().await.unwrap());
    return Some(tx);
}
