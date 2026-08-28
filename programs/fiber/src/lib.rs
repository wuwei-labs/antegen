pub mod constants;
pub mod errors;
pub mod instructions;
pub mod state;

pub use constants::*;
use instructions::*;
use state::*;

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::Instruction;

declare_id!("AgFv5afjW9DmSPkiEvJ1er5bAAmRUqaBeTB6Cr8e1hKx");

// On-chain security contact, read by explorers from the deployed `.so`.
//
// Gated on `not(no-entrypoint)` for the same reason the entrypoint is: the
// macro emits a `#[no_mangle]` static into the binary, and this crate is
// depended on with the `cpi` feature (which implies `no-entrypoint`) by the
// thread program, which publishes its own. Two of these in one binary is a
// link-time symbol collision.
#[cfg(not(feature = "no-entrypoint"))]
solana_security_txt::security_txt! {
    name: "Antegen Fiber Program",
    project_url: "https://antegen.xyz/",
    contacts: "email:anthony@wuwei.dev",
    policy: "https://github.com/wuwei-labs/antegen/blob/main/SECURITY.md",
    preferred_languages: "en",
    source_code: "https://github.com/wuwei-labs/antegen/tree/main/programs/fiber",
    source_release: concat!("antegen-fiber-program-v", env!("CARGO_PKG_VERSION")),
    auditors: "None"
}

#[program]
pub mod antegen_fiber {
    use super::*;

    /// Creates a fiber (instruction account) for a thread.
    /// Thread PDA must be signer and payer.
    /// `lookup_tables` is capped at 4 (Solana v0 transaction limit).
    ///
    /// `lookup_tables` is `Trailing` because it was appended after callers
    /// already existed — see the type's docs. Programs that CPI straight into
    /// this program, rather than through the thread program's `create_fiber`,
    /// send instruction data that ends before this argument.
    pub fn create(
        ctx: Context<Create>,
        fiber_index: u8,
        instruction: SerializableInstruction,
        priority_fee: u64,
        lookup_tables: Trailing<Vec<Pubkey>>,
    ) -> Result<()> {
        let instruction: Instruction = instruction.into();
        instructions::create::create(
            ctx,
            fiber_index,
            instruction,
            priority_fee,
            lookup_tables.into_inner(),
        )
    }

    /// Updates a fiber's instruction content (or initializes if it doesn't exist).
    /// Thread PDA must be signer and payer. Resets execution stats.
    /// Pass `None` for `instruction` to wipe the compiled instruction (idle fiber).
    /// Pass `None` for `lookup_tables` to leave them unchanged; `Some(vec)`
    /// atomically replaces. Legacy fibers reject non-empty lookup_tables.
    ///
    /// `lookup_tables` is `Trailing` for the same reason as `create`. 5.2.1
    /// wrapped the thread program's `create_fiber`/`update_fiber`, but this
    /// program's own entrypoints were left bare — and an on-chain program
    /// whose crank path CPIs directly here (`fiber::cpi::update`, as SRSLY's
    /// `stage_close_fiber` and `wipe_fiber` do when the thread is the signer)
    /// bypasses the thread program entirely. Those callers hit
    /// `InstructionDidNotDeserialize` on every execution until this was
    /// wrapped too.
    pub fn update(
        ctx: Context<Update>,
        fiber_index: u8,
        instruction: Option<SerializableInstruction>,
        priority_fee: Option<u64>,
        lookup_tables: Trailing<Option<Vec<Pubkey>>>,
    ) -> Result<()> {
        let instruction = instruction.map(|i| i.into());
        instructions::update::update(
            ctx,
            fiber_index,
            instruction,
            priority_fee,
            lookup_tables.into_inner(),
        )
    }

    /// Closes a fiber account, returns rent to thread PDA.
    ///
    /// `fiber_index` is **required**, deliberately unlike the `Trailing`
    /// arguments on `create` and `update`. Those exist so a caller compiled
    /// against an older IDL keeps working; here that leniency is the
    /// vulnerability. An absent index can only mean "derive nothing", which is
    /// how a caller retires one index while closing another account, and a
    /// default of 0 would be worse — it would name a real, wrong fiber.
    ///
    /// This is a wire break for any program that CPIs straight into `close`.
    /// They must be rebuilt against this IDL. Failing loudly on old data is the
    /// intent: the alternative silently accepts the unbound close.
    pub fn close(ctx: Context<Close>, fiber_index: u8) -> Result<()> {
        instructions::close::close(ctx, fiber_index)
    }

    /// Copies source fiber's instruction into target, closes source.
    /// Target keeps its PDA/index, source is deleted.
    pub fn swap(ctx: Context<Swap>) -> Result<()> {
        instructions::swap::swap(ctx)
    }
}

#[cfg(test)]
mod wire_compat_tests {
    use super::*;
    use anchor_lang::AnchorDeserialize;

    /// Deterministic stand-in for `Pubkey::new_unique()`.
    ///
    /// Still distinct on every call, but reproducible from run to run, so a
    /// failing assertion reports the same addresses each time.
    fn unique_pubkey() -> Pubkey {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let mut bytes = [0u8; 32];
        bytes[..8].copy_from_slice(&NEXT.fetch_add(1, Ordering::Relaxed).to_le_bytes());
        Pubkey::new_from_array(bytes)
    }

    /// Instruction data exactly as a caller built against the pre-`lookup_tables`
    /// IDL sends it: everything through `priority_fee`, and then nothing.
    ///
    /// 5.2.1 wrapped the thread program's `create_fiber`/`update_fiber` but left
    /// this program's own entrypoints bare, so a program CPI'ing directly into
    /// `fiber::cpi::update` — which is what a thread-signed crank path does —
    /// still failed to decode. That took down every fiber operation for those
    /// callers until they were rebuilt and redeployed.
    #[test]
    fn update_decodes_data_without_the_appended_argument() {
        let mut old_format = Vec::new();
        1u8.serialize(&mut old_format).unwrap(); // fiber_index
        None::<SerializableInstruction>
            .serialize(&mut old_format)
            .unwrap(); // instruction
        None::<u64>.serialize(&mut old_format).unwrap(); // priority_fee
                                                         // no lookup_tables — the caller does not know the argument exists

        let ix = instruction::Update::deserialize(&mut &old_format[..])
            .expect("data from a pre-lookup_tables caller must still decode");

        assert_eq!(ix.fiber_index, 1);
        assert_eq!(
            ix.lookup_tables.into_inner(),
            None,
            "an absent argument means leave lookup tables unchanged, as before it existed"
        );
    }

    /// The new format must still decode, or the program cannot read its own
    /// clients.
    #[test]
    fn update_decodes_data_with_the_appended_argument() {
        let tables = vec![unique_pubkey(), unique_pubkey()];
        let mut new_format = Vec::new();
        1u8.serialize(&mut new_format).unwrap();
        None::<SerializableInstruction>
            .serialize(&mut new_format)
            .unwrap();
        None::<u64>.serialize(&mut new_format).unwrap();
        Some(tables.clone()).serialize(&mut new_format).unwrap();

        let ix = instruction::Update::deserialize(&mut &new_format[..]).unwrap();
        assert_eq!(ix.lookup_tables.into_inner(), Some(tables));
    }

    /// `close` deliberately does NOT get the `Trailing` treatment.
    ///
    /// Old callers sent no arguments at all. Accepting that again would mean
    /// closing a fiber with nothing binding the account to an index, which is
    /// the whole hole. Rejecting the old format is the intended behaviour, and
    /// it is why bumping this program is a breaking change for anything that
    /// CPIs into `close`.
    #[test]
    fn close_rejects_data_without_the_required_index() {
        let empty: Vec<u8> = Vec::new();
        assert!(
            instruction::Close::deserialize(&mut &empty[..]).is_err(),
            "an unbound close must not decode"
        );

        let mut with_index = Vec::new();
        3u8.serialize(&mut with_index).unwrap();
        assert_eq!(
            instruction::Close::deserialize(&mut &with_index[..])
                .unwrap()
                .fiber_index,
            3
        );
    }

    #[test]
    fn create_decodes_both_wire_formats() {
        let ix_data = SerializableInstruction {
            program_id: unique_pubkey(),
            accounts: vec![],
            data: vec![],
        };

        let mut old_format = Vec::new();
        2u8.serialize(&mut old_format).unwrap();
        ix_data.serialize(&mut old_format).unwrap();
        0u64.serialize(&mut old_format).unwrap();

        let old = instruction::Create::deserialize(&mut &old_format[..]).unwrap();
        assert!(old.lookup_tables.into_inner().is_empty());

        let tables = vec![unique_pubkey()];
        let mut new_format = old_format.clone();
        tables.serialize(&mut new_format).unwrap();

        let new = instruction::Create::deserialize(&mut &new_format[..]).unwrap();
        assert_eq!(new.lookup_tables.into_inner(), tables);
    }
}
