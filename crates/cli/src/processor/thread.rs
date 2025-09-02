use crate::{client::Client, errors::CliError};
use anchor_lang::{
    solana_program::{instruction::Instruction, system_program},
    InstructionData, ToAccountMetas,
};
use antegen_sdk::state::{SerializableInstruction, Thread, ThreadConfig, Trigger, TriggerContext};
use solana_sdk::{
    native_token::LAMPORTS_PER_SOL,
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    sysvar::{recent_blockhashes, rent},
};
use std::str::FromStr;

pub fn create(client: &Client, id: String, trigger: Trigger) -> Result<(), CliError> {
    let thread_pubkey = Thread::pubkey(client.payer_pubkey(), id.clone().into_bytes());
    let nonce_keypair = Keypair::new();

    let ix = Instruction {
        program_id: antegen_sdk::ID,
        accounts: antegen_sdk::accounts::ThreadCreate {
            authority: client.payer_pubkey(),
            payer: client.payer_pubkey(),
            thread: thread_pubkey,
            nonce_account: Some(nonce_keypair.pubkey()),
            recent_blockhashes: Some(recent_blockhashes::ID),
            rent: Some(rent::ID),
            system_program: system_program::ID,
        }
        .to_account_metas(Some(false)),
        data: antegen_sdk::instruction::CreateThread {
            amount: LAMPORTS_PER_SOL,
            id: id.into(),
            trigger,
        }
        .data(),
    };
    client
        .send_and_confirm(&[ix], &[client.payer(), &nonce_keypair])
        .unwrap();
    // Don't call get() here to avoid verbose output during creation
    Ok(())
}

pub fn delete(client: &Client, address: Pubkey) -> Result<(), CliError> {
    let ix = Instruction {
        program_id: antegen_sdk::ID,
        accounts: antegen_sdk::accounts::ThreadDelete {
            authority: client.payer_pubkey(),
            close_to: client.payer_pubkey(),
            thread: address,
        }
        .to_account_metas(Some(false)),
        data: antegen_sdk::instruction::DeleteThread {}.data(),
    };
    client.send_and_confirm(&[ix], &[client.payer()]).unwrap();
    Ok(())
}

pub fn get(client: &Client, address: Pubkey) -> Result<(), CliError> {
    let data = client.get_account_data(&address).unwrap();
    let thread = Thread::try_from(data).unwrap();

    // Print thread info in a clean format
    println!("\nThread Details:");
    println!("  Address: {}", address);
    println!("  Authority: {}", thread.authority);
    println!("  ID: {}", String::from_utf8_lossy(&thread.id));
    println!("  Paused: {}", thread.paused);

    // Print trigger info
    match thread.trigger {
        Trigger::Cron {
            schedule,
            skippable,
        } => {
            println!("  Trigger: Cron");
            println!("    Schedule: {}", schedule);
            println!("    Skippable: {}", skippable);
        }
        Trigger::Interval { seconds, skippable } => {
            println!("  Trigger: Interval");
            println!("    Every: {} seconds", seconds);
            println!("    Skippable: {}", skippable);
        }
        Trigger::Now => {
            println!("  Trigger: Immediate");
        }
        Trigger::Account {
            address,
            offset,
            size,
        } => {
            println!("  Trigger: Account");
            println!("    Address: {}", address);
            println!("    Offset: {}", offset);
            println!("    Size: {}", size);
        }
        Trigger::Timestamp { unix_ts } => {
            println!("  Trigger: Timestamp");
            println!("    At: {}", unix_ts);
        }
        Trigger::Slot { slot } => {
            println!("  Trigger: Slot");
            println!("    At: {}", slot);
        }
        Trigger::Epoch { epoch } => {
            println!("  Trigger: Epoch");
            println!("    At: {}", epoch);
        }
    }

    // Print execution info
    println!("  Execution:");
    println!("    Index: {}", thread.exec_index);
    println!("    Count: {}", thread.exec_count);

    // Print trigger context if available
    match thread.trigger_context {
        TriggerContext::Account { hash } => {
            println!("    Account Hash: {}", hash);
        }
        TriggerContext::Timestamp { prev, next } => {
            println!("    Previous: {}", prev);
            println!("    Next: {}", next);
        }
        TriggerContext::Block { prev, next } => {
            println!("    Previous Block: {}", prev);
            println!("    Next Block: {}", next);
        }
    }

    Ok(())
}

pub fn toggle(client: &Client, id: String) -> Result<(), CliError> {
    let thread_pubkey = Thread::pubkey(client.payer_pubkey(), id.into_bytes());
    let ix = Instruction {
        program_id: antegen_sdk::ID,
        accounts: antegen_sdk::accounts::ThreadToggle {
            authority: client.payer_pubkey(),
            thread: thread_pubkey,
        }
        .to_account_metas(Some(false)),
        data: antegen_sdk::instruction::ToggleThread {}.data(),
    };
    client.send_and_confirm(&[ix], &[client.payer()]).unwrap();
    get(client, thread_pubkey)?;
    Ok(())
}

pub fn update(client: &Client, id: String, schedule: Option<String>) -> Result<(), CliError> {
    let thread_pubkey = Thread::pubkey(client.payer_pubkey(), id.into_bytes());
    let new_trigger = if let Some(schedule) = schedule {
        Some(Trigger::Cron {
            schedule,
            skippable: true,
        })
    } else {
        None
    };
    let ix = Instruction {
        program_id: antegen_sdk::ID,
        accounts: antegen_sdk::accounts::ThreadUpdate {
            authority: client.payer_pubkey(),
            thread: thread_pubkey,
        }
        .to_account_metas(Some(false)),
        data: antegen_sdk::instruction::UpdateThread { new_trigger }.data(),
    };
    client.send_and_confirm(&[ix], &[client.payer()]).unwrap();
    get(client, thread_pubkey)?;
    Ok(())
}

pub fn init_config(client: &Client) -> Result<(), CliError> {
    let config_pubkey = ThreadConfig::pubkey();

    let ix = Instruction {
        program_id: antegen_sdk::ID,
        accounts: antegen_sdk::accounts::ConfigInit {
            admin: client.payer_pubkey(),
            config: config_pubkey,
            system_program: system_program::ID,
        }
        .to_account_metas(Some(false)),
        data: antegen_sdk::instruction::InitConfig {}.data(),
    };

    client.send_and_confirm(&[ix], &[client.payer()]).unwrap();
    // Silent initialization - will be shown in localnet status
    Ok(())
}

pub fn create_fiber(
    client: &Client,
    thread_id: String,
    index: u8,
    instruction: Instruction,
) -> Result<(), CliError> {
    let thread_pubkey = Thread::pubkey(client.payer_pubkey(), thread_id.into_bytes());

    // Convert standard Instruction to SerializableInstruction
    let serializable_instruction: SerializableInstruction = instruction.into();

    // Derive fiber PDA
    let fiber_pubkey = Pubkey::find_program_address(
        &[b"thread_fiber", thread_pubkey.as_ref(), &[index]],
        &antegen_sdk::ID,
    )
    .0;

    let ix = Instruction {
        program_id: antegen_sdk::ID,
        accounts: antegen_sdk::accounts::FiberCreate {
            authority: client.payer_pubkey(),
            payer: client.payer_pubkey(),
            thread: thread_pubkey,
            fiber: fiber_pubkey,
            system_program: system_program::ID,
        }
        .to_account_metas(Some(false)),
        data: antegen_sdk::instruction::CreateFiber {
            index,
            instruction: serializable_instruction,
            signer_seeds: vec![], // Empty for simple instructions
        }
        .data(),
    };

    client.send_and_confirm(&[ix], &[client.payer()]).unwrap();
    Ok(())
}

pub fn parse_pubkey_from_id_or_address(
    authority: Pubkey,
    id: Option<String>,
    address: Option<Pubkey>,
) -> Result<Pubkey, CliError> {
    let address_from_id = id.map(|str| Thread::pubkey(authority, str));
    address.or(address_from_id).ok_or(CliError::InvalidAddress)
}

pub fn stress_test(
    client: &Client,
    count: u32,
    interval: u64,
    jitter: u64,
    prefix: String,
    with_fibers: bool,
    batch_size: u32,
) -> Result<(), CliError> {
    use crate::print_status;
    use rand::Rng;

    println!("🚀 Creating {} test threads for stress testing", count);
    println!(
        "   Base interval: {} seconds, Jitter: ±{} seconds",
        interval, jitter
    );
    println!("   Thread ID prefix: '{}'", prefix);
    println!("   Batch size: {} threads per batch", batch_size);

    if with_fibers {
        println!("   Each thread will have a fiber with memo instruction");
    }

    let mut rng = rand::thread_rng();
    let mut created = 0;
    let mut failed = 0;

    // Process in batches to avoid rate limiting
    for batch_num in 0..(count as f32 / batch_size as f32).ceil() as u32 {
        let batch_start = batch_num * batch_size;
        let batch_end = ((batch_num + 1) * batch_size).min(count);
        let batch_count = batch_end - batch_start;

        println!(
            "\nBatch {} ({}/{}): Creating {} threads...",
            batch_num + 1,
            batch_start + 1,
            count,
            batch_count
        );

        for i in batch_start..batch_end {
            let thread_id = format!("{}-{:04}", prefix, i);

            // Calculate interval with jitter
            let jitter_amount = if jitter > 0 {
                rng.gen_range(0..=(jitter * 2)) as i64 - jitter as i64
            } else {
                0
            };
            let thread_interval = (interval as i64 + jitter_amount).max(1) as u64;

            // Create thread with interval trigger
            let trigger = Trigger::Interval {
                seconds: thread_interval as i64,
                skippable: true,
            };

            match create(client, thread_id.clone(), trigger) {
                Ok(_) => {
                    created += 1;

                    // Create fiber if requested
                    if with_fibers {
                        let thread_pubkey =
                            Thread::pubkey(client.payer_pubkey(), thread_id.clone().into_bytes());
                        let memo_program_id =
                            Pubkey::from_str("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr")
                                .map_err(|e| {
                                    CliError::BadParameter(format!(
                                        "Invalid memo program ID: {}",
                                        e
                                    ))
                                })?;
                        let memo_instruction = Instruction {
                            program_id: memo_program_id,
                            accounts: vec![],
                            data: format!("Stress test thread {} execution", thread_id)
                                .into_bytes(),
                        };

                        if let Err(e) = create_fiber(client, thread_id.clone(), 0, memo_instruction)
                        {
                            eprintln!("Failed to create fiber for {}: {}", thread_id, e);
                        }
                    }

                    // Show progress periodically
                    if created % 10 == 0 {
                        print!(".");
                        use std::io::Write;
                        std::io::stdout().flush().ok();
                    }
                }
                Err(e) => {
                    failed += 1;
                    eprintln!("\nFailed to create thread {}: {}", thread_id, e);
                }
            }
        }

        // Pause between batches to avoid rate limiting (except for last batch)
        if batch_num < (count as f32 / batch_size as f32).ceil() as u32 - 1 {
            println!("\nPausing 2 seconds before next batch...");
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    }

    println!("\n");
    print_status!(
        "Stress Test Complete",
        "Created {} threads, {} failed",
        created,
        failed
    );

    if created > 0 {
        println!("✅ Successfully created {} test threads", created);
        println!(
            "   Thread IDs: {}-0000 to {}-{:04}",
            prefix,
            prefix,
            created - 1
        );
        println!(
            "   Intervals range from {} to {} seconds",
            interval.saturating_sub(jitter),
            interval + jitter
        );
    }

    if failed > 0 {
        println!("⚠️  {} threads failed to create", failed);
    }

    Ok(())
}
