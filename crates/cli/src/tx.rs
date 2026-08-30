//! Transaction submission for CLI commands.
//!
//! Every command that writes to the chain did the same three things inline:
//! fetch a blockhash, build a legacy `Message`, sign it. Thirteen copies of it,
//! each hardcoding the message format at the call site.
//!
//! That is the same duplication `antegen_client::tx` exists to remove for the
//! executor, and it has the same consequence: when the format changes under
//! SIMD-0385, thirteen places have to agree about it, and a place that
//! disagrees produces a transaction that fails for reasons that look nothing
//! like a format problem. So the CLI builds through the same seam.

use antegen_client::rpc::RpcPool;
use antegen_client::tx::{self, TxConfig, TxVersion};
use anyhow::{anyhow, Result};
use solana_instruction::Instruction;
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_signer::Signer;
use solana_transaction::versioned::VersionedTransaction;

/// Message format CLI commands emit.
///
/// A constant rather than a configurable, unlike the node's `[transaction]
/// version`. CLI transactions are one-off and human-initiated, so there is no
/// throughput argument for a newer format here — this follows the node once its
/// encoders land, rather than being rolled forward independently.
const CLI_TX_VERSION: TxVersion = TxVersion::Legacy;

/// Resource limits CLI transactions request.
///
/// Deliberately none. CLI commands submit a handful of transactions in a human
/// lifetime, so the compute they over-reserve costs little even once the
/// SIMD-0553 resource fee ramps in, and requesting a tight limit would mean a
/// simulation round trip — plus a new way for `antegen thread create` to fail —
/// on every command. The executor's hot path is where that trade goes the other
/// way.
fn cli_tx_config() -> TxConfig {
    TxConfig::new()
}

/// Build and sign a transaction against a fresh blockhash.
///
/// Separate from [`send`] for commands that simulate before submitting.
pub(crate) async fn build(
    client: &RpcPool,
    instructions: &[Instruction],
    payer: &Pubkey,
    signers: &[&dyn Signer],
) -> Result<VersionedTransaction> {
    let (blockhash, _) = client
        .get_latest_blockhash()
        .await
        .map_err(|e| anyhow!("Failed to fetch blockhash: {}", e))?;

    tx::build_transaction(
        CLI_TX_VERSION,
        signers,
        payer,
        instructions,
        &cli_tx_config(),
        blockhash,
    )
    .map_err(|e| anyhow!("Failed to build transaction: {}", e))
}

/// Build, sign, submit, and await confirmation.
///
/// `action` names what was being attempted, for the error message — "create
/// thread", "withdraw from thread". It is interpolated as `Failed to {action}`.
pub(crate) async fn send(
    client: &RpcPool,
    instructions: &[Instruction],
    payer: &Pubkey,
    signers: &[&dyn Signer],
    action: &str,
) -> Result<Signature> {
    let transaction = build(client, instructions, payer, signers).await?;

    client
        .send_and_confirm_transaction(&transaction)
        .await
        .map_err(|e| anyhow!("Failed to {}: {}", action, e))
}
