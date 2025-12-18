//! Safe RPC response types with custom deserialization
//!
//! Handles the u64::MAX -> float serialization issue where Solana RPC
//! serializes u64::MAX (18446744073709551615) as a float (1.8446744073709552e19)
//! in simulateTransaction responses.

use base64::prelude::*;
use serde::{Deserialize, Deserializer};
use std::io::Read;
use thiserror::Error;

// ============================================================================
// Account Data Decoding
// ============================================================================

/// Error type for account data decoding
#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("Base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("Zstd decompression error: {0}")]
    Decompression(String),
    #[error("Unsupported encoding: {0}")]
    UnsupportedEncoding(String),
}

/// Decode account data based on encoding type
///
/// Supports:
/// - `"base64"`: Standard base64 decoding
/// - `"base64+zstd"`: Base64 decode then zstd decompress
///
/// # Example
/// ```ignore
/// let bytes = decode_account_data(&account.data.0, &account.data.1)?;
/// ```
pub fn decode_account_data(data: &str, encoding: &str) -> Result<Vec<u8>, DecodeError> {
    let decoded = BASE64_STANDARD.decode(data)?;

    match encoding {
        "base64" => Ok(decoded),
        "base64+zstd" => {
            let mut decompressed = Vec::new();
            zstd::stream::read::Decoder::new(decoded.as_slice())
                .map_err(|e| DecodeError::Decompression(e.to_string()))?
                .read_to_end(&mut decompressed)
                .map_err(|e| DecodeError::Decompression(e.to_string()))?;
            Ok(decompressed)
        }
        other => Err(DecodeError::UnsupportedEncoding(other.to_string())),
    }
}

// ============================================================================
// Custom Deserializers
// ============================================================================

/// Deserializer that handles u64::MAX serialized as float
pub fn deserialize_u64_or_float<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum U64OrFloat {
        U64(u64),
        Float(f64),
    }

    match U64OrFloat::deserialize(deserializer)? {
        U64OrFloat::U64(v) => Ok(v),
        U64OrFloat::Float(v) => {
            // u64::MAX as float is ~1.8446744073709552e19
            // Use a threshold to detect it
            if v >= 1.8e19 {
                Ok(u64::MAX)
            } else if v >= 0.0 {
                Ok(v as u64)
            } else {
                Err(serde::de::Error::custom(format!(
                    "negative float {} cannot be converted to u64",
                    v
                )))
            }
        }
    }
}

/// Deserializer for Option<u64> that handles float serialization
fn deserialize_optional_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    match value {
        None => Ok(None),
        Some(v) => {
            if v.is_null() {
                Ok(None)
            } else if let Some(n) = v.as_u64() {
                Ok(Some(n))
            } else if let Some(f) = v.as_f64() {
                if f >= 1.8e19 {
                    Ok(Some(u64::MAX))
                } else if f >= 0.0 {
                    Ok(Some(f as u64))
                } else {
                    Err(serde::de::Error::custom("negative float for u64"))
                }
            } else {
                Err(serde::de::Error::custom("expected u64 or float"))
            }
        }
    }
}

/// Safe UiAccount that handles rentEpoch properly
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeUiAccount {
    pub lamports: u64,
    /// Account data as (base64_data, encoding)
    pub data: (String, String),
    pub owner: String,
    pub executable: bool,
    #[serde(deserialize_with = "deserialize_u64_or_float")]
    pub rent_epoch: u64,
}

impl SafeUiAccount {
    /// Decode the account data bytes
    pub fn decode_data(&self) -> Result<Vec<u8>, DecodeError> {
        decode_account_data(&self.data.0, &self.data.1)
    }

    /// Parse the owner as a Pubkey
    pub fn owner_pubkey(&self) -> Result<solana_sdk::pubkey::Pubkey, String> {
        self.owner.parse().map_err(|e| format!("Invalid owner pubkey: {}", e))
    }
}

/// Safe simulation result value
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeSimulationValue {
    pub err: Option<serde_json::Value>,
    pub logs: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_u64")]
    pub units_consumed: Option<u64>,
    pub accounts: Option<Vec<Option<SafeUiAccount>>>,
    pub return_data: Option<serde_json::Value>,
}

/// Safe simulation result wrapper
#[derive(Debug, Clone, Deserialize)]
pub struct SafeSimulationResult {
    pub value: SafeSimulationValue,
}

/// Generic RPC response wrapper
#[derive(Debug, Clone, Deserialize)]
pub struct RpcResponse<T> {
    pub result: T,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_u64_max_as_float() {
        let json = r#"{"lamports":1000,"data":["","base64"],"owner":"11111111111111111111111111111111","executable":false,"rentEpoch":1.8446744073709552e19}"#;
        let account: SafeUiAccount = serde_json::from_str(json).unwrap();
        assert_eq!(account.rent_epoch, u64::MAX);
    }

    #[test]
    fn test_u64_max_as_int() {
        let json = r#"{"lamports":1000,"data":["","base64"],"owner":"11111111111111111111111111111111","executable":false,"rentEpoch":18446744073709551615}"#;
        let account: SafeUiAccount = serde_json::from_str(json).unwrap();
        assert_eq!(account.rent_epoch, u64::MAX);
    }

    #[test]
    fn test_normal_u64() {
        let json = r#"{"lamports":1000,"data":["","base64"],"owner":"11111111111111111111111111111111","executable":false,"rentEpoch":12345}"#;
        let account: SafeUiAccount = serde_json::from_str(json).unwrap();
        assert_eq!(account.rent_epoch, 12345);
    }

    #[test]
    fn test_simulation_result() {
        let json = r#"{
            "value": {
                "err": null,
                "logs": ["Program log: test"],
                "unitsConsumed": 150,
                "accounts": [null],
                "returnData": null
            }
        }"#;
        let result: SafeSimulationResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.value.units_consumed, Some(150));
        assert!(result.value.err.is_none());
    }

    #[test]
    fn test_simulation_with_account() {
        let json = r#"{
            "value": {
                "err": null,
                "logs": [],
                "unitsConsumed": 100,
                "accounts": [{
                    "lamports": 1000,
                    "data": ["dGVzdA==", "base64"],
                    "owner": "11111111111111111111111111111111",
                    "executable": false,
                    "rentEpoch": 1.8446744073709552e19
                }],
                "returnData": null
            }
        }"#;
        let result: SafeSimulationResult = serde_json::from_str(json).unwrap();
        let accounts = result.value.accounts.unwrap();
        let account = accounts[0].as_ref().unwrap();
        assert_eq!(account.rent_epoch, u64::MAX);
    }
}
