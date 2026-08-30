//! Verifying a transaction format before the node commits to it.
//!
//! `[transaction] version` decides how every execution is encoded. A format the
//! cluster will not accept therefore fails *every* thread, and it fails them
//! with rejections that look like transport trouble rather than like a
//! configuration mistake — which is a bad way to find out, on a node that was
//! executing fine ten seconds earlier.
//!
//! So the format is proved once at startup, on a transaction of the node's own,
//! before any thread depends on it.
//!
//! Two questions, because they fail differently:
//!
//! 1. Does the RPC understand the encoding? A simulation answers that for free.
//! 2. Does the cluster accept it on the wire? Only actually submitting answers
//!    that — a leader's ingest path is not the RPC's deserializer, and a format
//!    can pass one and fail the other.
//!
//! Wire acceptance is a property of the *format*, not of any particular
//! transaction, so the probe is deliberately the smallest transaction the node
//! can construct. There is no need to make it resemble a thread execution, and
//! good reason not to: a probe that touches thread state can fail for reasons
//! that have nothing to do with encoding.
//!
//! Note what counts as success. The probe asks whether the cluster *accepted
//! and processed* the transaction, not whether the transaction did anything
//! useful. One that lands and then fails on-chain has still answered the
//! question — the bytes were understood. Only a rejection of the encoding
//! itself is a failure here.

use crate::rpc::RpcPool;
use crate::tx::{self, TxConfig, TxVersion};
use parking_lot::RwLock;
use solana_keypair::Keypair;
use solana_signer::Signer;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use thiserror::Error;

/// Format rejections in a row before the node gives up on its configured format
/// and falls back to legacy.
///
/// More than one, because a single rejection can come from one unhealthy member
/// of the RPC pool rather than from the cluster. Few, because every one of them
/// is a thread that did not execute.
pub const FORMAT_REJECTIONS_BEFORE_REVERT: u32 = 3;

/// The message format currently in use, and the standing guard on it.
///
/// The startup probe proves a format *once*. This covers everything the probe
/// cannot: a cluster that stops accepting the format mid-run, an RPC pool whose
/// members disagree, a probe that passed against one endpoint while the hot path
/// uses another. Without it, a cluster turning against the format silently stops
/// every thread on the node.
///
/// Shared rather than copied, because every worker holds a clone of the executor
/// and a fallback that applied only to the clone which saw the rejection would
/// leave the rest of the node emitting a format already known to be rejected.
#[derive(Debug, Clone)]
pub struct FormatGuard {
    version: Arc<RwLock<TxVersion>>,
    rejections: Arc<AtomicU32>,
}

impl FormatGuard {
    pub fn new(version: TxVersion) -> Self {
        Self {
            version: Arc::new(RwLock::new(version)),
            rejections: Arc::new(AtomicU32::new(0)),
        }
    }

    pub fn version(&self) -> TxVersion {
        *self.version.read()
    }

    /// A transaction reached the cluster, so no rejection streak is in progress.
    pub fn record_accepted(&self) {
        self.rejections.store(0, Ordering::Relaxed);
    }

    /// Account for a submission failure. Returns the format to use from now on.
    ///
    /// Only encoding rejections count toward the streak. Reverting the node's
    /// format because a fiber was broken or a blockhash expired would be a
    /// worse failure than the one being guarded against.
    pub fn record_failure(&self, error: &str) -> TxVersion {
        if !is_unsupported_format_error(error) {
            return self.version();
        }

        let current = self.version();
        if current == TxVersion::Legacy {
            return current;
        }

        let seen = self.rejections.fetch_add(1, Ordering::Relaxed) + 1;
        if seen < FORMAT_REJECTIONS_BEFORE_REVERT {
            log::warn!(
                "Cluster rejected a {} transaction ({} of {} before falling back): {}",
                current,
                seen,
                FORMAT_REJECTIONS_BEFORE_REVERT,
                error
            );
            return current;
        }

        let mut version = self.version.write();
        if *version != TxVersion::Legacy {
            log::error!(
                "Cluster rejected {} transactions {} times in a row; falling back to legacy. \
                 Investigate before selecting it again, and set `[transaction] version = \
                 \"legacy\"` to stop probing for it at startup.",
                *version,
                seen
            );
            *version = TxVersion::Legacy;
        }
        self.rejections.store(0, Ordering::Relaxed);
        *version
    }
}

#[derive(Debug, Error)]
pub enum ProbeError {
    #[error("could not build a {version} transaction: {source}")]
    Build {
        version: TxVersion,
        source: tx::TxError,
    },
    #[error("the RPC rejected a {version} transaction: {reason}")]
    Rejected { version: TxVersion, reason: String },
    #[error("could not reach the cluster to probe {version}: {reason}")]
    Unreachable { version: TxVersion, reason: String },
}

/// Whether a failure says the cluster does not understand the message format.
///
/// Distinguished from ordinary failures because it is the only kind that
/// justifies abandoning the configured format. Reverting on a thread's fiber
/// being broken, or on a blockhash expiring, would swap the node's encoding for
/// reasons that have nothing to do with encoding.
pub fn is_unsupported_format_error(error: &str) -> bool {
    const MARKERS: &[&str] = &[
        "unsupported transaction version",
        "UnsupportedVersion",
        "VersionNotSupported",
        "failed to deserialize",
        "invalid transaction",
        "SanitizeFailure",
        "DeserializationError",
    ];
    let lowered = error.to_ascii_lowercase();
    MARKERS
        .iter()
        .any(|marker| lowered.contains(&marker.to_ascii_lowercase()))
}

/// Prove that the cluster accepts this message format.
///
/// Legacy is not probed. It is what every node has always emitted, and spending
/// a transaction on every startup to confirm the unchanged default would be
/// pure cost.
pub async fn probe_format(
    version: TxVersion,
    rpc: &RpcPool,
    keypair: &Keypair,
) -> Result<(), ProbeError> {
    if version == TxVersion::Legacy {
        return Ok(());
    }

    log::info!(
        "Probing {} transaction support before switching...",
        version
    );

    let payer = keypair.pubkey();
    // Zero lamports to the incinerator: a real instruction that succeeds, moves
    // nothing, and needs no account to be created. A self-transfer would be
    // smaller still, but the same account appearing as both source and
    // destination fails at execution, and a probe that always errors on-chain
    // reads like a problem every time a node starts.
    let instruction =
        solana_system_interface::instruction::transfer(&payer, &solana_sdk_ids::incinerator::ID, 0);

    let (blockhash, _) = rpc
        .get_latest_blockhash()
        .await
        .map_err(|e| ProbeError::Unreachable {
            version,
            reason: e.to_string(),
        })?;

    let transaction = tx::build_transaction(
        version,
        &[keypair],
        &payer,
        std::slice::from_ref(&instruction),
        &TxConfig::new().with_compute_unit_limit(1_000),
        blockhash,
    )
    .map_err(|source| ProbeError::Build { version, source })?;

    // 1. Encoding — does the RPC understand these bytes at all?
    if let Err(e) = rpc.simulate_transaction(&transaction, &[]).await {
        let reason = e.to_string();
        return Err(if is_unsupported_format_error(&reason) {
            ProbeError::Rejected { version, reason }
        } else {
            // A simulation that fails for some other reason has still proved
            // the encoding was understood, which is all this step was for.
            log::debug!("{} probe simulated with an error ({}), continuing to the wire check — the encoding was understood", version, reason);
            return wire_check(version, rpc, &transaction).await;
        });
    }

    // 2. The wire — does a leader accept it?
    wire_check(version, rpc, &transaction).await
}

/// Submit the probe and wait for the cluster to process it.
async fn wire_check(
    version: TxVersion,
    rpc: &RpcPool,
    transaction: &solana_transaction::versioned::VersionedTransaction,
) -> Result<(), ProbeError> {
    match rpc.send_and_confirm_transaction(transaction).await {
        Ok(signature) => {
            log::info!("{} accepted by the cluster ({})", version, signature);
            Ok(())
        }
        Err(e) => {
            let reason = e.to_string();
            if is_unsupported_format_error(&reason) {
                Err(ProbeError::Rejected { version, reason })
            } else if reason.contains("Transaction failed:") {
                // It landed. The bytes were understood well enough to execute,
                // which is the only thing this step was asking.
                log::info!(
                    "{} accepted by the cluster, though the probe itself errored: {}",
                    version,
                    reason
                );
                Ok(())
            } else {
                Err(ProbeError::Unreachable { version, reason })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only encoding rejections may cost the node its configured format.
    #[test]
    fn ordinary_failures_are_not_format_failures() {
        for ordinary in [
            "BlockhashNotFound",
            "InstructionError(0, Custom(6004))",
            "AccountInUse",
            "ComputationalBudgetExceeded",
            "connection refused",
        ] {
            assert!(
                !is_unsupported_format_error(ordinary),
                "{ordinary} must not trigger a format revert"
            );
        }
    }

    const REJECTION: &str = "unsupported transaction version";

    /// A streak short of the threshold leaves the format alone: one rejection
    /// can come from a single unhealthy endpoint rather than from the cluster.
    #[test]
    fn a_short_streak_does_not_cost_the_format() {
        let guard = FormatGuard::new(TxVersion::V0);
        for _ in 0..FORMAT_REJECTIONS_BEFORE_REVERT - 1 {
            assert_eq!(guard.record_failure(REJECTION), TxVersion::V0);
        }
        assert_eq!(guard.version(), TxVersion::V0);
    }

    #[test]
    fn a_settled_rejection_reverts_to_legacy() {
        let guard = FormatGuard::new(TxVersion::V0);
        for _ in 0..FORMAT_REJECTIONS_BEFORE_REVERT {
            guard.record_failure(REJECTION);
        }
        assert_eq!(guard.version(), TxVersion::Legacy);
    }

    /// The streak is consecutive. A transaction that reaches the cluster in
    /// between says the format still works, so earlier rejections must not
    /// accumulate toward a revert.
    #[test]
    fn an_accepted_transaction_clears_the_streak() {
        let guard = FormatGuard::new(TxVersion::V0);
        for _ in 0..100 {
            for _ in 0..FORMAT_REJECTIONS_BEFORE_REVERT - 1 {
                guard.record_failure(REJECTION);
            }
            guard.record_accepted();
        }
        assert_eq!(guard.version(), TxVersion::V0);
    }

    /// The guard exists to react to encoding rejections and nothing else.
    #[test]
    fn ordinary_failures_never_revert_the_format() {
        let guard = FormatGuard::new(TxVersion::V0);
        for _ in 0..1_000 {
            guard.record_failure("InstructionError(0, Custom(6004))");
            guard.record_failure("BlockhashNotFound");
        }
        assert_eq!(guard.version(), TxVersion::V0);
    }

    /// Every worker holds a clone; a revert one of them observes has to reach
    /// the rest, or the node keeps emitting a rejected format.
    #[test]
    fn a_revert_is_visible_through_every_clone() {
        let guard = FormatGuard::new(TxVersion::V0);
        let clone = guard.clone();

        for _ in 0..FORMAT_REJECTIONS_BEFORE_REVERT {
            clone.record_failure(REJECTION);
        }

        assert_eq!(guard.version(), TxVersion::Legacy);
        assert_eq!(clone.version(), TxVersion::Legacy);
    }

    #[test]
    fn encoding_rejections_are_recognised() {
        for rejection in [
            "Transaction version (0) is not supported: unsupported transaction version",
            "failed to deserialize solana_transaction::versioned::VersionedTransaction",
            "SanitizeFailure",
        ] {
            assert!(is_unsupported_format_error(rejection), "{rejection}");
        }
    }
}
