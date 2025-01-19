use thiserror::Error;
use solana_client::client_error::ClientError;

#[derive(Debug, Error)]
pub enum CliError {
    #[error("Account not found: {0}")]
    AccountNotFound(String),
    #[error("Account data could not be parsed: {0}")]
    AccountDataNotParsable(String),
    #[error("Bad client: {0}")]
    BadClient(String),
    #[error("Bad parameter: {0}")]
    BadParameter(String),
    #[error("This codepath hasn't been implemented yet")]
    NotImplemented,
    #[error("Command not recognized: {0}")]
    CommandNotRecognized(String),
    #[error("Transaction failed with error: {0}")]
    FailedTransaction(String),
    #[error("Failed to start localnet with error: {0}")]
    FailedLocalnet(String),
    #[error("Invalid address")]
    InvalidAddress,
    #[error("Program file does not exist")]
    InvalidProgramFile,
    #[error("No default signer found in {0}, \
     run `solana-keygen new`, or `solana config set —keypair <FILEPATH>`")]
    KeypairNotFound(String),
    #[error("Failed to kill validator process: {0}")]
    ValidatorCleanup(String),
    #[error("{0}")]
    Custom(String),
}

impl From<anyhow::Error> for CliError {
    fn from(err: anyhow::Error) -> Self {
        CliError::FailedLocalnet(err.to_string())
    }
}

// For errors that should become FailedTransaction
impl From<ClientError> for CliError {
    fn from(err: ClientError) -> Self {
        CliError::FailedTransaction(err.to_string())
    }
}

impl From<std::io::Error> for CliError {
    fn from(err: std::io::Error) -> Self {
        CliError::ValidatorCleanup(err.to_string())
    }
}
