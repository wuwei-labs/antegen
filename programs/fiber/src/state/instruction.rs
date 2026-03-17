use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use std::collections::HashMap;

/// Serializable version of Solana's Instruction for easier handling
#[derive(AnchorDeserialize, AnchorSerialize, Clone, Debug)]
pub struct SerializableInstruction {
    pub program_id: Pubkey,
    pub accounts: Vec<SerializableAccountMeta>,
    pub data: Vec<u8>,
}

/// Serializable version of AccountMeta
#[derive(AnchorDeserialize, AnchorSerialize, Clone, Debug)]
pub struct SerializableAccountMeta {
    pub pubkey: Pubkey,
    pub is_signer: bool,
    pub is_writable: bool,
}

impl From<Instruction> for SerializableInstruction {
    fn from(ix: Instruction) -> Self {
        SerializableInstruction {
            program_id: ix.program_id,
            accounts: ix
                .accounts
                .into_iter()
                .map(|acc| SerializableAccountMeta {
                    pubkey: acc.pubkey,
                    is_signer: acc.is_signer,
                    is_writable: acc.is_writable,
                })
                .collect(),
            data: ix.data,
        }
    }
}

impl From<SerializableInstruction> for Instruction {
    fn from(ix: SerializableInstruction) -> Self {
        Instruction {
            program_id: ix.program_id,
            accounts: ix
                .accounts
                .into_iter()
                .map(|acc| AccountMeta {
                    pubkey: acc.pubkey,
                    is_signer: acc.is_signer,
                    is_writable: acc.is_writable,
                })
                .collect(),
            data: ix.data,
        }
    }
}

/// Compiled instruction data for space-efficient storage
#[derive(AnchorDeserialize, AnchorSerialize, Clone, Debug)]
pub struct CompiledInstructionData {
    pub program_id_index: u8,
    pub accounts: Vec<u8>,
    pub data: Vec<u8>,
}

/// Compiled instruction containing deduplicated accounts
#[derive(AnchorDeserialize, AnchorSerialize, Clone, Debug)]
pub struct CompiledInstructionV0 {
    pub num_ro_signers: u8,
    pub num_rw_signers: u8,
    pub num_rw: u8,
    pub instructions: Vec<CompiledInstructionData>,
    pub accounts: Vec<Pubkey>,
}

/// Compile an instruction into a space-efficient format
pub fn compile_instruction(
    instruction: Instruction,
) -> Result<CompiledInstructionV0> {
    let mut pubkeys_to_metadata: HashMap<Pubkey, AccountMeta> = HashMap::new();

    // Add program ID
    pubkeys_to_metadata.insert(
        instruction.program_id,
        AccountMeta {
            pubkey: instruction.program_id,
            is_signer: false,
            is_writable: false,
        },
    );

    // Process accounts
    for acc in &instruction.accounts {
        let entry = pubkeys_to_metadata
            .entry(acc.pubkey)
            .or_insert(AccountMeta {
                pubkey: acc.pubkey,
                is_signer: false,
                is_writable: false,
            });
        entry.is_signer |= acc.is_signer;
        entry.is_writable |= acc.is_writable;
    }

    // Sort accounts by priority
    let mut sorted_accounts: Vec<Pubkey> = pubkeys_to_metadata.keys().cloned().collect();
    sorted_accounts.sort_by(|a, b| {
        let a_meta = &pubkeys_to_metadata[a];
        let b_meta = &pubkeys_to_metadata[b];

        fn get_priority(meta: &AccountMeta) -> u8 {
            match (meta.is_signer, meta.is_writable) {
                (true, true) => 0,
                (true, false) => 1,
                (false, true) => 2,
                (false, false) => 3,
            }
        }

        get_priority(a_meta).cmp(&get_priority(b_meta))
    });

    // Count account types
    let mut num_rw_signers = 0u8;
    let mut num_ro_signers = 0u8;
    let mut num_rw = 0u8;

    for pubkey in &sorted_accounts {
        let meta = &pubkeys_to_metadata[pubkey];
        if meta.is_signer && meta.is_writable {
            num_rw_signers += 1;
        } else if meta.is_signer && !meta.is_writable {
            num_ro_signers += 1;
        } else if meta.is_writable {
            num_rw += 1;
        }
    }

    // Create index mapping
    let accounts_to_index: HashMap<Pubkey, u8> = sorted_accounts
        .iter()
        .enumerate()
        .map(|(i, k)| (*k, i as u8))
        .collect();

    // Create compiled instruction
    let compiled_instruction = CompiledInstructionData {
        program_id_index: *accounts_to_index.get(&instruction.program_id).unwrap(),
        accounts: instruction
            .accounts
            .iter()
            .map(|acc| *accounts_to_index.get(&acc.pubkey).unwrap())
            .collect(),
        data: instruction.data,
    };

    Ok(CompiledInstructionV0 {
        num_ro_signers,
        num_rw_signers,
        num_rw,
        instructions: vec![compiled_instruction],
        accounts: sorted_accounts,
    })
}

/// Decompile a compiled instruction back to a regular instruction
pub fn decompile_instruction(compiled: &CompiledInstructionV0) -> Result<Instruction> {
    if compiled.instructions.is_empty() {
        return Err(ProgramError::InvalidInstructionData.into());
    }

    let ix = &compiled.instructions[0];
    let program_id = compiled.accounts[ix.program_id_index as usize];

    let accounts: Vec<AccountMeta> = ix
        .accounts
        .iter()
        .enumerate()
        .map(|(_i, &idx)| {
            let pubkey = compiled.accounts[idx as usize];
            let is_writable = if idx < compiled.num_rw_signers {
                true
            } else if idx < compiled.num_rw_signers + compiled.num_ro_signers {
                false
            } else if idx < compiled.num_rw_signers + compiled.num_ro_signers + compiled.num_rw {
                true
            } else {
                false
            };
            let is_signer = idx < compiled.num_rw_signers + compiled.num_ro_signers;

            AccountMeta {
                pubkey,
                is_signer,
                is_writable,
            }
        })
        .collect();

    Ok(Instruction {
        program_id,
        accounts,
        data: ix.data.clone(),
    })
}
