use crate::{client::Client, errors::CliError};
use anchor_lang::{
    solana_program::{instruction::Instruction, system_program},
    InstructionData, ToAccountMetas,
};
use antegen_sdk::state::{SerializableInstruction, Thread, Trigger, TriggerContext};
use solana_sdk::{
    native_token::LAMPORTS_PER_SOL,
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    sysvar::{recent_blockhashes, rent},
};
use std::str::FromStr;

pub fn create(client: &Client, id: String, trigger: Trigger) -> Result<(), CliError> {
    // Create thread with nonce and default fiber in a single operation
    create_with_optional_nonce_and_fiber(client, id, trigger, true, true)?;
    Ok(())
}

pub fn create_with_optional_nonce_and_fiber(
    client: &Client,
    id: String,
    trigger: Trigger,
    use_nonce: bool,
    with_default_fiber: bool,
) -> Result<(), CliError> {
    let thread_pubkey = Thread::pubkey(client.payer_pubkey(), id.clone().into_bytes());

    // Prepare optional initial instruction
    let (initial_instruction, priority_fee) = if with_default_fiber {
        // Create a simple memo instruction as the default fiber
        let memo_program_id = Pubkey::from_str("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr")
            .map_err(|e| CliError::BadParameter(format!("Invalid memo program ID: {}", e)))?;

        let memo_instruction = Instruction {
            program_id: memo_program_id,
            accounts: vec![],
            data: format!("Thread {} default fiber", id).into_bytes(),
        };

        let serializable: SerializableInstruction = memo_instruction.into();
        (Some(serializable), Some(0u64))
    } else {
        (None, None)
    };

    // Derive fiber PDA if we're creating one
    let fiber_pubkey = if with_default_fiber {
        Some(
            Pubkey::find_program_address(
                &[b"thread_fiber", thread_pubkey.as_ref(), &[0]],
                &antegen_sdk::ID,
            )
            .0,
        )
    } else {
        None
    };

    if use_nonce {
        // Create with nonce account (durable thread)
        let nonce_keypair = Keypair::new();

        // Build account metas with explicit signer control
        let mut accounts = antegen_sdk::accounts::ThreadCreate {
            authority: client.payer_pubkey(),
            payer: client.payer_pubkey(),
            thread: thread_pubkey,
            fiber: fiber_pubkey,
            nonce_account: Some(nonce_keypair.pubkey()),
            recent_blockhashes: Some(recent_blockhashes::ID),
            rent: Some(rent::ID),
            system_program: system_program::ID,
        }
        .to_account_metas(Some(false));

        // Manually set signers
        for account in accounts.iter_mut() {
            if account.pubkey == client.payer_pubkey() {
                account.is_signer = true;
            } else if account.pubkey == nonce_keypair.pubkey() {
                account.is_signer = true;
            }
        }

        let ix = Instruction {
            program_id: antegen_sdk::ID,
            accounts,
            data: antegen_sdk::instruction::CreateThread {
                amount: LAMPORTS_PER_SOL,
                id: id.clone().into(),
                trigger: trigger.clone(),
                initial_instruction: initial_instruction.clone(),
                priority_fee,
            }
            .data(),
        };
        client
            .send_and_confirm(&[ix], &[client.payer(), &nonce_keypair])
            .unwrap();
    } else {
        // Create without nonce account (regular thread)
        let mut accounts = antegen_sdk::accounts::ThreadCreate {
            authority: client.payer_pubkey(),
            payer: client.payer_pubkey(),
            thread: thread_pubkey,
            fiber: fiber_pubkey,
            nonce_account: None,
            recent_blockhashes: None,
            rent: None,
            system_program: system_program::ID,
        }
        .to_account_metas(Some(false));

        // Manually set signers (authority and payer are the same in this case)
        for account in accounts.iter_mut() {
            if account.pubkey == client.payer_pubkey() {
                account.is_signer = true;
            }
        }

        let ix = Instruction {
            program_id: antegen_sdk::ID,
            accounts,
            data: antegen_sdk::instruction::CreateThread {
                amount: LAMPORTS_PER_SOL,
                id: id.into(),
                trigger,
                initial_instruction,
                priority_fee,
            }
            .data(),
        };
        client.send_and_confirm(&[ix], &[client.payer()]).unwrap();
    }

    // Don't call get() here to avoid verbose output during creation
    Ok(())
}

pub fn delete(client: &Client, address: Pubkey) -> Result<(), CliError> {
    let mut accounts = antegen_sdk::accounts::ThreadDelete {
        authority: client.payer_pubkey(),
        close_to: client.payer_pubkey(),
        thread: address,
    }
    .to_account_metas(Some(false));

    // Manually set authority as signer
    for account in accounts.iter_mut() {
        if account.pubkey == client.payer_pubkey() {
            account.is_signer = true;
            break;
        }
    }

    let ix = Instruction {
        program_id: antegen_sdk::ID,
        accounts,
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
    println!("  Last Executor: {}", thread.last_executor);
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
    let mut accounts = antegen_sdk::accounts::ThreadToggle {
        authority: client.payer_pubkey(),
        thread: thread_pubkey,
    }
    .to_account_metas(Some(false));

    // Manually set authority as signer
    for account in accounts.iter_mut() {
        if account.pubkey == client.payer_pubkey() {
            account.is_signer = true;
            break;
        }
    }

    let ix = Instruction {
        program_id: antegen_sdk::ID,
        accounts,
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
    let mut accounts = antegen_sdk::accounts::ThreadUpdate {
        authority: client.payer_pubkey(),
        thread: thread_pubkey,
    }
    .to_account_metas(Some(false));

    // Manually set authority as signer
    for account in accounts.iter_mut() {
        if account.pubkey == client.payer_pubkey() {
            account.is_signer = true;
            break;
        }
    }

    let ix = Instruction {
        program_id: antegen_sdk::ID,
        accounts,
        data: antegen_sdk::instruction::UpdateThread { new_trigger }.data(),
    };
    client.send_and_confirm(&[ix], &[client.payer()]).unwrap();
    get(client, thread_pubkey)?;
    Ok(())
}

pub fn create_fiber(
    client: &Client,
    thread_id: String,
    index: u8,
    instruction: Instruction,
    priority_fee: Option<u64>,
) -> Result<(), CliError> {
    let thread_pubkey = Thread::pubkey(client.payer_pubkey(), thread_id.into_bytes());

    // Derive fiber PDA
    let fiber_pubkey = Pubkey::find_program_address(
        &[b"thread_fiber", thread_pubkey.as_ref(), &[index]],
        &antegen_sdk::ID,
    )
    .0;

    // Check if fiber already exists
    match client.get_account_data(&fiber_pubkey) {
        Ok(_) => {
            // Fiber already exists, skip creation
            return Ok(());
        }
        Err(_) => {
            // Fiber doesn't exist, proceed with creation
        }
    }

    // Convert standard Instruction to SerializableInstruction
    let serializable_instruction: SerializableInstruction = instruction.into();

    let mut accounts = antegen_sdk::accounts::FiberCreate {
        authority: client.payer_pubkey(),
        payer: client.payer_pubkey(),
        thread: thread_pubkey,
        fiber: fiber_pubkey,
        system_program: system_program::ID,
    }
    .to_account_metas(Some(false));

    // Manually set authority and payer as signers
    for account in accounts.iter_mut() {
        if account.pubkey == client.payer_pubkey() {
            account.is_signer = true;
        }
    }

    let ix = Instruction {
        program_id: antegen_sdk::ID,
        accounts,
        data: antegen_sdk::instruction::CreateFiber {
            index,
            instruction: serializable_instruction,
            signer_seeds: vec![], // Empty for simple instructions
            priority_fee: priority_fee.unwrap_or(0),
        }
        .data(),
    };

    match client.send_and_confirm(&[ix], &[client.payer()]) {
        Ok(_) => Ok(()),
        Err(e) => {
            // If it fails because account already exists, that's ok
            if e.to_string().contains("already in use") {
                Ok(())
            } else {
                Err(CliError::BadParameter(format!(
                    "Failed to create fiber: {}",
                    e
                )))
            }
        }
    }
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
    _with_fibers: bool, // Kept for backward compatibility, min/max fiber counts now control fibers
    batch_size: u32,
    durable_ratio: u8,
    min_fiber_count: u8,
    max_fiber_count: u8,
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
    println!("   Durable thread ratio: {}%", durable_ratio);
    if min_fiber_count == max_fiber_count {
        println!("   Fibers per thread: {}", min_fiber_count);
    } else {
        println!(
            "   Fibers per thread: {}-{}",
            min_fiber_count, max_fiber_count
        );
    }

    let mut rng = rand::thread_rng();
    let mut created = 0;
    let mut failed = 0;
    let mut durable_count = 0;
    let mut regular_count = 0;
    let mut total_fibers_created = 0;
    let mut actual_min_fibers = max_fiber_count; // Track actual min used
    let mut actual_max_fibers = min_fiber_count; // Track actual max used

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

            // Determine if this thread should be durable based on the ratio
            let use_nonce = if durable_ratio == 100 {
                true // All durable
            } else if durable_ratio == 0 {
                false // All regular
            } else {
                // Use random selection based on ratio
                rng.gen_range(0..100) < durable_ratio
            };

            // Choose number of fibers for this thread
            let num_fibers = if min_fiber_count == max_fiber_count {
                // Exact count, no randomization needed
                min_fiber_count
            } else {
                // Random between min and max (inclusive)
                rng.gen_range(min_fiber_count..=max_fiber_count)
            };

            // Determine if we should create an initial fiber with the thread
            let with_initial_fiber = num_fibers > 0;

            match create_with_optional_nonce_and_fiber(
                client,
                thread_id.clone(),
                trigger,
                use_nonce,
                with_initial_fiber,
            ) {
                Ok(_) => {
                    created += 1;
                    if use_nonce {
                        durable_count += 1;
                    } else {
                        regular_count += 1;
                    }

                    // Track actual min/max fibers for statistics
                    actual_min_fibers = actual_min_fibers.min(num_fibers);
                    actual_max_fibers = actual_max_fibers.max(num_fibers);

                    // If initial fiber was created with thread, count it
                    if with_initial_fiber {
                        total_fibers_created += 1;
                    }

                    // Create additional fibers (starting from index 1 since index 0 was created with the thread)
                    if num_fibers > 1 {
                        let memo_program_id =
                            Pubkey::from_str("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr")
                                .map_err(|e| {
                                    CliError::BadParameter(format!(
                                        "Invalid memo program ID: {}",
                                        e
                                    ))
                                })?;

                        let mut fiber_creation_failed = false;
                        // Start from index 1 since index 0 was created with the thread
                        for fiber_index in 1..num_fibers {
                            let memo_data = format!(
                                "Thread {} fiber {} (durable: {}, total: {})",
                                thread_id, fiber_index, use_nonce, num_fibers
                            );

                            let memo_instruction = Instruction {
                                program_id: memo_program_id,
                                accounts: vec![],
                                data: memo_data.into_bytes(),
                            };

                            if let Err(e) = create_fiber(
                                client,
                                thread_id.clone(),
                                fiber_index,
                                memo_instruction,
                                None,
                            ) {
                                eprintln!(
                                    "Failed to create fiber {} for {}: {}",
                                    fiber_index, thread_id, e
                                );
                                fiber_creation_failed = true;
                                break;
                            }
                            total_fibers_created += 1;
                        }

                        if fiber_creation_failed {
                            failed += 1;
                            continue; // Skip to next thread if any fiber creation fails
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
            "   Durable threads (with nonce): {} ({}%)",
            durable_count,
            if created > 0 {
                (durable_count * 100) / created
            } else {
                0
            }
        );
        println!(
            "   Regular threads (no nonce): {} ({}%)",
            regular_count,
            if created > 0 {
                (regular_count * 100) / created
            } else {
                0
            }
        );
        println!(
            "   Intervals range from {} to {} seconds",
            interval.saturating_sub(jitter),
            interval + jitter
        );

        // Fiber statistics
        println!("\n📊 Fiber Statistics:");
        println!("   Total fibers created: {}", total_fibers_created);
        println!(
            "   Average fibers per thread: {:.2}",
            total_fibers_created as f64 / created as f64
        );
        if min_fiber_count == max_fiber_count {
            println!("   Fibers per thread: {} (exact)", min_fiber_count);
        } else {
            println!(
                "   Configured range: {}-{}",
                min_fiber_count, max_fiber_count
            );
            println!("   Actual min used: {}", actual_min_fibers);
            println!("   Actual max used: {}", actual_max_fibers);
        }
    }

    if failed > 0 {
        println!("⚠️  {} threads failed to create", failed);
    }

    Ok(())
}
