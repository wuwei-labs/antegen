use std::sync::Arc;

use anchor_lang::{
    solana_program::instruction::Instruction,
    InstructionData, ToAccountMetas
};
use antegen_network_program::state::{Config, Pool, Registry, Worker};
use log::info;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{signature::Keypair, signer::Signer, transaction::Transaction};

use crate::pool_position::PoolPosition;

pub async fn build_pool_rotation_tx<'a>(
    client: Arc<RpcClient>,
    keypair: &Keypair,
    pool_position: PoolPosition,
    registry: Registry,
    worker_id: u64,
) -> Option<Transaction> {
    info!("current_position: {:?}",
        pool_position.current_position,
    );

    // Exit early if the rotator is not intialized
    if registry.nonce == 0 {
        return None;
    }

    // Exit early if the worker is already in the pool.
    if pool_position.current_position.is_some() {
        return None;
    }

    // Build rotation instruction to rotate the worker into pool 0.
    let ix = Instruction {
        program_id: antegen_network_program::ID,
        accounts: antegen_network_program::accounts::PoolRotate {
            config: Config::pubkey(),
            pool: Pool::pubkey(0),
            signatory: keypair.pubkey(),
            worker: Worker::pubkey(worker_id),
        }.to_account_metas(Some(false)),
        data: antegen_network_program::instruction::PoolRotate {}.data(),
    };

    // Build and sign tx.
    let mut tx = Transaction::new_with_payer(&[ix.clone()], Some(&keypair.pubkey()));
    tx.sign(&[keypair], client.get_latest_blockhash().await.unwrap());
    return Some(tx);
}
