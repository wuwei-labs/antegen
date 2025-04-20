use {solana_program::pubkey::Pubkey, std::fmt::Debug};

#[derive(Clone, Debug)]
pub struct PoolPosition {
    pub current_position: Option<u64>,
    pub builders: Vec<Pubkey>,
}

impl Default for PoolPosition {
    fn default() -> Self {
        PoolPosition {
            current_position: None,
            builders: vec![],
        }
    }
}
