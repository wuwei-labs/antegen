//! Transaction assembly.
//!
//! One place where a transaction's *resource limits* — compute unit limit,
//! compute unit price, loaded-accounts data size, heap frame — turn into wire
//! format.
//!
//! Today those limits travel as ComputeBudget instructions prepended to the
//! instruction list. Under SIMD-0385 (transaction v1) they move into
//! fixed-offset fields in the message header, and ComputeBudget instructions
//! are *ignored for configuration* while still consuming compute units as
//! no-ops. A v1 message that carries them therefore runs with a compute limit
//! of zero — v1 defaults the limit to 0, not to 200k per instruction — and
//! burns compute on the instructions that were supposed to prevent exactly
//! that. Every execution fails, and nothing in the type system objects.
//!
//! That failure is silent, total, and depends only on which message type the
//! builder happened to produce. So the encoding decision lives here, once,
//! rather than at each of the dozen call sites that used to hand-roll it.
//!
//! Legacy and v0 are implemented; v1 waits on solana-sdk, whose message crate
//! at the pinned version carries `versions/{legacy, v0}` only — v1 arrives in
//! the 4.x line — while the SIMD itself sits in Review with no feature gate
//! activated. What *is* implemented for v1 is the decision about what it must
//! not contain, so the invariant is asserted by a test today rather than
//! discovered on mainnet later.

use serde::{Deserialize, Serialize};
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_hash::Hash;
use solana_instruction::Instruction;
use solana_message::{legacy::Message, v0, VersionedMessage};
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::versioned::VersionedTransaction;
use thiserror::Error;

/// Solana's per-transaction compute ceiling.
pub const MAX_COMPUTE_UNITS: u32 = 1_400_000;

/// `PACKET_DATA_SIZE` — the maximum serialized size of a legacy or v0
/// transaction.
pub const MAX_TRANSACTION_SIZE_LEGACY: usize = 1232;

/// The v1 ceiling from SIMD-0385. Roughly triples the room for batched fiber
/// execs, which is why the size limit is a function of version rather than the
/// constant it used to be.
pub const MAX_TRANSACTION_SIZE_V1: usize = 4096;

#[derive(Debug, Error)]
pub enum TxError {
    /// Selecting a format the build path cannot yet emit. Deliberately an
    /// error rather than a silent fallback to legacy: falling back would
    /// quietly undo whatever made the newer format worth selecting, and the
    /// operator would see normal-looking execution with none of the benefit.
    #[error("transaction {0} encoding is not implemented yet")]
    UnsupportedVersion(TxVersion),
    /// The instructions could not be compiled into a message — too many
    /// accounts for the format, or a lookup table that does not cover what the
    /// instructions reference.
    #[error("failed to compile message: {0}")]
    Compile(String),
    /// Signing failed, most often because a required signer was not supplied.
    #[error("failed to sign transaction: {0}")]
    Sign(String),
}

/// Which message format to encode into.
///
/// Operator-selectable (`[transaction] version` in the client config) so a
/// format change is a config flip and a restart rather than a release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TxVersion {
    #[default]
    Legacy,
    /// Address lookup tables, same 1232-byte ceiling. Compiled with no lookup
    /// tables today, which makes it legacy plus two bytes — what it buys is the
    /// versioned send path v1 also needs, and somewhere for lookup tables to go.
    V0,
    /// SIMD-0385: resource limits move into the message header and the ceiling
    /// rises to 4096. No lookup tables. Waits on the solana-message 4.x line and
    /// on the feature gate activating.
    V1,
}

impl std::fmt::Display for TxVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TxVersion::Legacy => write!(f, "legacy"),
            TxVersion::V0 => write!(f, "v0"),
            TxVersion::V1 => write!(f, "v1"),
        }
    }
}

impl TxVersion {
    /// Maximum serialized transaction size for this format.
    pub fn max_transaction_size(&self) -> usize {
        match self {
            TxVersion::Legacy | TxVersion::V0 => MAX_TRANSACTION_SIZE_LEGACY,
            TxVersion::V1 => MAX_TRANSACTION_SIZE_V1,
        }
    }

    /// Whether resource limits are encoded as ComputeBudget instructions.
    ///
    /// False for v1, where they are header fields and any ComputeBudget
    /// instruction is dead weight that still costs compute.
    pub fn limits_are_instructions(&self) -> bool {
        matches!(self, TxVersion::Legacy | TxVersion::V0)
    }

    /// Whether this build path can emit the format today.
    ///
    /// Config validation rejects the others at startup, so an operator who
    /// selects one gets a single clear error instead of every execution
    /// failing for a reason that looks unrelated.
    pub fn is_implemented(&self) -> bool {
        matches!(self, TxVersion::Legacy | TxVersion::V0)
    }
}

/// Resource limits requested from the runtime.
///
/// `None` means "do not request", which is not the same as zero and does not
/// mean the same thing in both formats — legacy falls back to 200k compute
/// units per instruction, v1 to zero. Callers that care must set the value;
/// this type only records the decision.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TxConfig {
    pub compute_unit_limit: Option<u32>,
    /// Micro-lamports per compute unit, as `SetComputeUnitPrice` takes it.
    ///
    /// v1 carries a flat lamport priority fee instead of a per-unit price, so
    /// this becomes `price * limit / 1_000_000` at v1 encoding time. Storing
    /// the price rather than the product keeps the conversion in one direction
    /// and keeps the legacy encoding lossless.
    pub compute_unit_price: Option<u64>,
    pub loaded_accounts_data_size_limit: Option<u32>,
    pub heap_size: Option<u32>,
}

impl TxConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_compute_unit_limit(mut self, limit: u32) -> Self {
        self.compute_unit_limit = Some(limit.min(MAX_COMPUTE_UNITS));
        self
    }

    /// Sets the price only when non-zero — a zero price is what "no priority
    /// fee" means, and emitting the instruction anyway spends bytes and, under
    /// SIMD-0553, cost units to say nothing.
    pub fn with_compute_unit_price(mut self, price: u64) -> Self {
        self.compute_unit_price = (price > 0).then_some(price);
        self
    }

    pub fn with_loaded_accounts_data_size_limit(mut self, bytes: u32) -> Self {
        self.loaded_accounts_data_size_limit = Some(bytes);
        self
    }

    pub fn with_heap_size(mut self, bytes: u32) -> Self {
        self.heap_size = Some(bytes);
        self
    }

    /// A config for *sizing* a transaction whose real limits are not known yet.
    ///
    /// Batching decides how many instructions fit before the compute estimate
    /// exists, so the byte cost of the limit instructions has to be reserved in
    /// advance. Only their *presence* affects size — `set_compute_unit_limit`
    /// encodes a fixed-width u32 whatever its value — so reserving with
    /// placeholder values is exact for every field that ends up emitted, and
    /// merely conservative for any that does not.
    ///
    /// Conservative is the safe direction here: over-reserving splits a batch
    /// one instruction early, while under-reserving builds a transaction that
    /// the runtime rejects for size after the compute budget is prepended.
    pub fn reserving_limits() -> Self {
        Self::new()
            .with_compute_unit_limit(MAX_COMPUTE_UNITS)
            .with_compute_unit_price(1)
            .with_loaded_accounts_data_size_limit(u32::MAX)
    }

    /// The ComputeBudget instructions this config encodes to.
    ///
    /// Empty for v1 — the values ride in the header there, and including the
    /// instructions anyway would leave the limits unset while still burning
    /// compute on them.
    pub fn budget_instructions(&self, version: TxVersion) -> Vec<Instruction> {
        if !version.limits_are_instructions() {
            return Vec::new();
        }

        let mut ixs = Vec::new();
        if let Some(limit) = self.compute_unit_limit {
            ixs.push(ComputeBudgetInstruction::set_compute_unit_limit(limit));
        }
        if let Some(price) = self.compute_unit_price {
            ixs.push(ComputeBudgetInstruction::set_compute_unit_price(price));
        }
        if let Some(bytes) = self.loaded_accounts_data_size_limit {
            ixs.push(ComputeBudgetInstruction::set_loaded_accounts_data_size_limit(bytes));
        }
        if let Some(bytes) = self.heap_size {
            ixs.push(ComputeBudgetInstruction::request_heap_frame(bytes));
        }
        ixs
    }
}

/// Compose the full instruction list: resource limits first, then the payload.
pub fn instructions_with_limits(
    version: TxVersion,
    config: &TxConfig,
    instructions: &[Instruction],
) -> Vec<Instruction> {
    let mut all = config.budget_instructions(version);
    all.extend_from_slice(instructions);
    all
}

/// Build an unsigned message carrying `blockhash`.
///
/// v0 is compiled with no address lookup tables. Without them it is legacy plus
/// a version byte, which is the point: nothing in the executor uses lookup
/// tables yet, and what v0 buys today is the versioned send path that v1 also
/// needs. Lookup table support becomes a change to this function alone.
pub fn build_message(
    version: TxVersion,
    payer: &Pubkey,
    instructions: &[Instruction],
    config: &TxConfig,
    blockhash: Hash,
) -> Result<VersionedMessage, TxError> {
    let instructions = instructions_with_limits(version, config, instructions);

    match version {
        TxVersion::Legacy => Ok(VersionedMessage::Legacy(Message::new_with_blockhash(
            &instructions,
            Some(payer),
            &blockhash,
        ))),
        TxVersion::V0 => v0::Message::try_compile(payer, &instructions, &[], blockhash)
            .map(VersionedMessage::V0)
            .map_err(|e| TxError::Compile(e.to_string())),
        other => Err(TxError::UnsupportedVersion(other)),
    }
}

/// Build and sign a transaction.
pub fn build_transaction(
    version: TxVersion,
    signers: &[&dyn Signer],
    payer: &Pubkey,
    instructions: &[Instruction],
    config: &TxConfig,
    blockhash: Hash,
) -> Result<VersionedTransaction, TxError> {
    let message = build_message(version, payer, instructions, config, blockhash)?;
    VersionedTransaction::try_new(message, &signers.to_vec())
        .map_err(|e| TxError::Sign(e.to_string()))
}

/// Serialized size of the transaction these inputs would produce.
///
/// Built from the same `TxConfig` the transaction itself is built from. The
/// previous estimator hardcoded a 1.4M limit and a 1M price regardless of what
/// the worker went on to request, and assumed a price instruction that the
/// worker only emits when the priority fee is non-zero — so it disagreed with
/// reality by an instruction's worth of bytes on the common path.
pub fn estimate_size(
    version: TxVersion,
    payer: &Pubkey,
    instructions: &[Instruction],
    config: &TxConfig,
) -> Result<usize, TxError> {
    // A default blockhash is exact for sizing: it occupies the same 32 bytes as
    // a real one, and nothing else in the message depends on its value.
    let message = build_message(version, payer, instructions, config, Hash::default())?;
    // +64 signature, +1 compact-u16 length prefix.
    Ok(bincode::serialized_size(&message).unwrap_or(0) as usize + 65)
}

/// Whether these instructions fit in one transaction of this version.
pub fn fits(
    version: TxVersion,
    payer: &Pubkey,
    instructions: &[Instruction],
    config: &TxConfig,
) -> bool {
    estimate_size(version, payer, instructions, config)
        .map(|size| size <= version.max_transaction_size())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_instruction::AccountMeta;

    const COMPUTE_BUDGET_ID: Pubkey =
        solana_pubkey::pubkey!("ComputeBudget111111111111111111111111111111");

    fn payload() -> Vec<Instruction> {
        vec![Instruction {
            program_id: Pubkey::new_unique(),
            accounts: vec![AccountMeta::new(Pubkey::new_unique(), false)],
            data: vec![1, 2, 3],
        }]
    }

    fn full_config() -> TxConfig {
        TxConfig::new()
            .with_compute_unit_limit(200_000)
            .with_compute_unit_price(1_000)
            .with_loaded_accounts_data_size_limit(256 * 1024)
            .with_heap_size(64 * 1024)
    }

    /// The invariant the whole module exists for.
    ///
    /// Under SIMD-0385 a ComputeBudget instruction in a v1 message is ignored
    /// for configuration but still consumes compute units. Since v1 defaults
    /// the compute limit to zero rather than 200k per instruction, emitting
    /// them there means every transaction fails on a limit that was never
    /// applied. This is asserted now, while v1 encoding is still unimplemented,
    /// because the failure mode is invisible until it is total.
    #[test]
    fn v1_never_carries_compute_budget_instructions() {
        let config = full_config();
        assert!(
            config.budget_instructions(TxVersion::V1).is_empty(),
            "v1 encodes resource limits in the header; instructions would be ignored yet still burn compute"
        );

        let composed = instructions_with_limits(TxVersion::V1, &config, &payload());
        assert!(
            composed.iter().all(|ix| ix.program_id != COMPUTE_BUDGET_ID),
            "no ComputeBudget instruction may reach a v1 message"
        );
        assert_eq!(composed.len(), 1, "only the payload survives");
    }

    #[test]
    fn legacy_encodes_every_limit_as_an_instruction() {
        let ixs = full_config().budget_instructions(TxVersion::Legacy);
        assert_eq!(ixs.len(), 4);
        assert!(ixs.iter().all(|ix| ix.program_id == COMPUTE_BUDGET_ID));
    }

    #[test]
    fn unset_limits_emit_nothing() {
        assert!(TxConfig::new()
            .budget_instructions(TxVersion::Legacy)
            .is_empty());
    }

    /// A zero priority fee is an absent one. The worker already skipped the
    /// instruction in that case while the size estimator counted it.
    #[test]
    fn zero_price_is_not_encoded() {
        let config = TxConfig::new().with_compute_unit_price(0);
        assert_eq!(config.compute_unit_price, None);
        assert!(config
            .budget_instructions(TxVersion::Legacy)
            .iter()
            .all(|ix| ix.program_id != COMPUTE_BUDGET_ID));
    }

    #[test]
    fn compute_unit_limit_is_clamped_to_the_ceiling() {
        let config = TxConfig::new().with_compute_unit_limit(u32::MAX);
        assert_eq!(config.compute_unit_limit, Some(MAX_COMPUTE_UNITS));
    }

    /// Size tracks the config it was given, rather than a fixed assumption
    /// about what the builder would later request.
    #[test]
    fn size_reflects_the_actual_config() {
        let payer = Pubkey::new_unique();
        let ixs = payload();

        let bare = estimate_size(TxVersion::Legacy, &payer, &ixs, &TxConfig::new()).unwrap();
        let budgeted = estimate_size(TxVersion::Legacy, &payer, &ixs, &full_config()).unwrap();

        assert!(
            budgeted > bare,
            "four budget instructions plus the ComputeBudget program key must cost bytes"
        );
    }

    #[test]
    fn v1_raises_the_size_ceiling() {
        assert_eq!(TxVersion::Legacy.max_transaction_size(), 1232);
        assert_eq!(TxVersion::V0.max_transaction_size(), 1232);
        assert_eq!(TxVersion::V1.max_transaction_size(), 4096);
    }

    /// v0 still carries resource limits as instructions — only v1 moves them
    /// into the header. Getting this backwards would strip the limits from a
    /// v0 message that needs them.
    #[test]
    fn v0_still_encodes_limits_as_instructions() {
        assert_eq!(full_config().budget_instructions(TxVersion::V0).len(), 4);
    }

    /// Selecting an unimplemented format fails loudly rather than silently
    /// degrading to legacy, which would undo whatever it was selected for.
    #[test]
    fn unimplemented_versions_are_refused_not_downgraded() {
        let payer = Pubkey::new_unique();

        assert!(!TxVersion::V1.is_implemented());
        assert!(matches!(
            build_message(
                TxVersion::V1,
                &payer,
                &payload(),
                &TxConfig::new(),
                Hash::default()
            ),
            Err(TxError::UnsupportedVersion(_))
        ));

        assert!(TxVersion::Legacy.is_implemented());
        assert!(TxVersion::V0.is_implemented());
    }

    /// Each version compiles to its own message variant. A v0 selection that
    /// quietly produced a legacy message would land successfully and teach the
    /// operator that the flip worked.
    #[test]
    fn each_version_compiles_to_its_own_variant() {
        let payer = Pubkey::new_unique();
        let config = TxConfig::new().with_compute_unit_limit(200_000);

        assert!(matches!(
            build_message(
                TxVersion::Legacy,
                &payer,
                &payload(),
                &config,
                Hash::default()
            ),
            Ok(VersionedMessage::Legacy(_))
        ));
        assert!(matches!(
            build_message(TxVersion::V0, &payer, &payload(), &config, Hash::default()),
            Ok(VersionedMessage::V0(_))
        ));
    }

    /// With no lookup tables, v0 costs two bytes over legacy: the version
    /// prefix, and the length prefix on an address-table-lookups vector that is
    /// empty.
    ///
    /// Worth pinning, because it is the honest statement of what v0 buys today.
    /// The gain is the versioned send path that v1 also needs, and the room for
    /// lookup tables later — not a smaller transaction.
    #[test]
    fn v0_without_lookup_tables_costs_two_bytes_over_legacy() {
        let payer = Pubkey::new_unique();
        let config = TxConfig::new().with_compute_unit_limit(200_000);

        let legacy = estimate_size(TxVersion::Legacy, &payer, &payload(), &config).unwrap();
        let v0 = estimate_size(TxVersion::V0, &payer, &payload(), &config).unwrap();

        assert_eq!(v0, legacy + 2);
        assert!(v0 <= TxVersion::V0.max_transaction_size());
    }

    /// The blockhash lives in the message for a versioned transaction, so
    /// building must carry it rather than applying it at signing time.
    #[test]
    fn the_message_carries_the_blockhash_it_was_built_with() {
        let payer = Pubkey::new_unique();
        let blockhash = Hash::new_from_array([3u8; 32]);

        for version in [TxVersion::Legacy, TxVersion::V0] {
            let message =
                build_message(version, &payer, &payload(), &TxConfig::new(), blockhash).unwrap();
            assert_eq!(
                *message.recent_blockhash(),
                blockhash,
                "{version} lost its blockhash"
            );
        }
    }

    #[test]
    fn version_round_trips_through_config_serialization() {
        for (version, encoded) in [
            (TxVersion::Legacy, "\"legacy\""),
            (TxVersion::V0, "\"v0\""),
            (TxVersion::V1, "\"v1\""),
        ] {
            assert_eq!(serde_json::to_string(&version).unwrap(), encoded);
            assert_eq!(serde_json::from_str::<TxVersion>(encoded).unwrap(), version);
        }
    }
}
