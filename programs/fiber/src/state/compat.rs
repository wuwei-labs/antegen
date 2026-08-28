//! Wire compatibility for arguments appended to an existing instruction.

use anchor_lang::prelude::*;
use std::io::{Read, Write};

/// An instruction argument that callers compiled against an older IDL do not
/// send at all.
///
/// Borsh has no notion of an optional trailing field. A program that appends an
/// argument changes the wire format, and every existing caller — including
/// on-chain programs that CPI in and cannot be upgraded in the same transaction
/// — starts failing to deserialize, because their instruction data simply ends
/// early. That is not hypothetical: `create_fiber` and `update_fiber` gained
/// `lookup_tables`, and the mainnet contract program that CPIs into
/// `update_fiber` was compiled against the previous surface.
///
/// This reads the value when the bytes are there and yields `T::default()` when
/// the buffer is exhausted, so both wire formats decode against one instruction.
/// For `lookup_tables` the default is exactly right — an old caller means "no
/// lookup tables", which is what it meant before the argument existed.
///
/// Safe because Anchor dispatches instruction data with `deserialize` rather
/// than `try_from_slice`: trailing bytes are ignored and short data is the only
/// failure mode, which is the one this closes. Serialization is unconditional,
/// so anything this program writes stays a well-formed borsh value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Trailing<T>(pub T);

impl<T> Trailing<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> From<T> for Trailing<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

impl<T: AnchorSerialize> AnchorSerialize for Trailing<T> {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        self.0.serialize(writer)
    }
}

impl<T: AnchorDeserialize + Default> AnchorDeserialize for Trailing<T> {
    fn deserialize_reader<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        // Take one byte to distinguish "nothing left" from "a value follows".
        // A borsh value is never zero bytes — even `None` writes its tag — so an
        // empty read means the caller stopped before this argument.
        let mut first = [0u8; 1];
        match reader.read(&mut first)? {
            0 => Ok(Self(T::default())),
            _ => {
                // Put the byte back in front of the remaining input rather than
                // requiring a seekable reader.
                let mut replayed = first.chain(reader);
                T::deserialize_reader(&mut replayed).map(Self)
            }
        }
    }
}

/// Described in the IDL as its inner type.
///
/// `create_type` returns `None` because the wrapper introduces no new shape —
/// it serializes exactly as `T` — so the IDL should describe `T`. Implemented
/// concretely for the two instantiations the programs use, since `IdlBuild` is
/// not implemented for collections like `Vec<T>` and cannot be delegated
/// generically.
///
/// Gated because `IdlBuild` only exists under `idl-build`; a normal program
/// build does not link it.
#[cfg(feature = "idl-build")]
mod idl {
    use super::Trailing;
    use anchor_lang::idl::types::IdlTypeDef;
    use anchor_lang::idl::IdlBuild;
    use anchor_lang::prelude::Pubkey;
    use std::collections::BTreeMap;

    impl IdlBuild for Trailing<Vec<Pubkey>> {
        fn create_type() -> Option<IdlTypeDef> {
            None
        }
        fn insert_types(_types: &mut BTreeMap<String, IdlTypeDef>) {}
        fn get_full_path() -> String {
            "Vec<Pubkey>".to_string()
        }
    }

    impl IdlBuild for Trailing<Option<Vec<Pubkey>>> {
        fn create_type() -> Option<IdlTypeDef> {
            None
        }
        fn insert_types(_types: &mut BTreeMap<String, IdlTypeDef>) {}
        fn get_full_path() -> String {
            "Option<Vec<Pubkey>>".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// The case this type exists for: instruction data that stops before the
    /// appended argument, as sent by a caller built against the older IDL.
    #[test]
    fn absent_input_yields_the_default() {
        let mut empty: &[u8] = &[];
        let v = Trailing::<Option<Vec<Pubkey>>>::deserialize_reader(&mut empty).unwrap();
        assert_eq!(v.into_inner(), None);

        let mut empty: &[u8] = &[];
        let v = Trailing::<Vec<Pubkey>>::deserialize_reader(&mut empty).unwrap();
        assert!(v.into_inner().is_empty());
    }

    #[test]
    fn present_input_round_trips() {
        for value in [None, Some(vec![]), Some(vec![unique_pubkey(); 3])] {
            let wrapped = Trailing(value.clone());
            let mut bytes = Vec::new();
            wrapped.serialize(&mut bytes).unwrap();

            let mut cursor: &[u8] = &bytes;
            let decoded = Trailing::<Option<Vec<Pubkey>>>::deserialize_reader(&mut cursor).unwrap();
            assert_eq!(decoded.into_inner(), value);
        }
    }

    /// Serialization is unaffected, so a value this program writes is
    /// indistinguishable from a plain borsh one.
    #[test]
    fn serialization_matches_the_inner_type() {
        let value = Some(vec![unique_pubkey()]);
        let mut wrapped_bytes = Vec::new();
        Trailing(value.clone())
            .serialize(&mut wrapped_bytes)
            .unwrap();
        let mut plain_bytes = Vec::new();
        value.serialize(&mut plain_bytes).unwrap();
        assert_eq!(wrapped_bytes, plain_bytes);
    }

    /// The byte taken to test for EOF must be handed back, or every present
    /// value would decode against a buffer missing its first byte.
    #[test]
    fn the_probe_byte_is_not_consumed() {
        let value = Some(vec![unique_pubkey(), unique_pubkey()]);
        let mut bytes = Vec::new();
        value.serialize(&mut bytes).unwrap();

        // Trailing input after the value must be left for the next field.
        let mut padded = bytes.clone();
        padded.extend_from_slice(&[0xAA, 0xBB]);
        let mut cursor: &[u8] = &padded;
        let decoded = Trailing::<Option<Vec<Pubkey>>>::deserialize_reader(&mut cursor).unwrap();
        assert_eq!(decoded.into_inner(), value);
        assert_eq!(cursor, &[0xAA, 0xBB], "must not over-read");
    }
}
