//! `antegen thread doctor` — diagnose threads that cannot execute, and plan
//! their repair.
//!
//! Diagnosis is the durable part and is meant to grow: any condition that
//! leaves a thread scheduled but unable to build belongs here. `--plan` is the
//! remedy side, and is deliberately named for what it produces rather than
//! what it would like to do — the CLI cannot apply it. Should a check ever
//! have a remedy this binary can sign for, `--apply` is the flag to add, and
//! the reason it is still free.
//!
//! A thread carries `fiber_ids`, a list of indices whose PDAs hold the
//! instructions it runs. Nothing on chain keeps that list and those accounts in
//! agreement: an account can disappear while the id that names it stays. When
//! that happens the thread is intact, still scheduled, and permanently unable
//! to build a transaction — executors report `Fiber <pubkey> not found` and
//! retry against an address that will never resolve.
//!
//! That is the state 358 mainnet threads were left in on 2026-08-26, when a
//! missing address check in `fiber::create` let their fiber accounts be claimed
//! and closed for their rent. See `post-mortems/2026-08-26-fiber-rent-sweep.md`.
//!
//! Diagnosing is a read. Reconstruction is also a read: this command never
//! signs anything. Writing a fiber requires the owning *thread's* authority,
//! which belongs to whoever created the thread — often a program PDA with no
//! private key — so the output is a manifest for that owner to replay, not a
//! transaction.

use anchor_lang::{AccountDeserialize, AnchorDeserialize, Discriminator};
use antegen_client::rpc::decode_account_data;
use antegen_client::rpc::{RpcPool, SignatureRecord};
use antegen_thread_program::fiber::Fiber;
use antegen_thread_program::state::Thread;
use antegen_thread_program::state::{SerializableInstruction, SEED_THREAD_FIBER};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr;

use crate::commands::get_rpc_url;

/// How the command was asked to behave. Diagnosis always runs; the rest are
/// additive.
pub(crate) struct DoctorOpts {
    pub(crate) json: bool,
    /// Work out what repairing each problem would take, and emit it as a plan.
    pub(crate) plan: bool,
    /// Write the report to this path as JSON instead of stdout.
    pub(crate) output: Option<PathBuf>,
    /// Prove the reconstruction against fibers that still exist.
    pub(crate) verify: bool,
    /// After replaying a manifest, diff what is on chain against it.
    pub(crate) confirm: Option<PathBuf>,
    /// Cap how many missing fibers reach the manifest.
    pub(crate) limit: Option<usize>,
    /// Restrict to a single fiber index.
    pub(crate) fiber_index: Option<u8>,
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct AccountMetaJson {
    pub(crate) pubkey: String,
    pub(crate) is_signer: bool,
    pub(crate) is_writable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct InstructionJson {
    pub(crate) program_id: String,
    pub(crate) accounts: Vec<AccountMetaJson>,
    /// Base64 so the manifest stays readable and diffable.
    pub(crate) data_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FiberState {
    pub(crate) instruction: Option<InstructionJson>,
    pub(crate) priority_fee: u64,
    pub(crate) lookup_tables: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RecoveredFiber {
    pub(crate) fiber_pda: String,
    pub(crate) writes_replayed: usize,
    pub(crate) recovered: Option<FiberState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ThreadReport {
    pub(crate) thread: String,
    pub(crate) authority: String,
    pub(crate) fiber_ids: Vec<u8>,
    pub(crate) fiber_cursor: u8,
    pub(crate) missing: Vec<u8>,
    pub(crate) signatures_scanned: usize,
    pub(crate) fibers: BTreeMap<u8, RecoveredFiber>,
    pub(crate) warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub(crate) thread_program: String,
    pub(crate) fiber_program: String,
    pub(crate) threads_scanned: usize,
    pub(crate) threads_unhealthy: usize,
    pub(crate) fibers_missing: usize,
    pub(crate) fibers_reconstructed: usize,
    pub(crate) unresolved: Vec<String>,
    pub(crate) results: Vec<ThreadReport>,
}

// ---------------------------------------------------------------------------
// Instruction decoding
// ---------------------------------------------------------------------------

/// One fiber write, normalised across the six instructions that can produce
/// one. `None` means "this instruction did not touch that field", which is what
/// lets an update fold onto a create without clobbering what it never mentioned.
#[derive(Debug, Clone)]
struct Delta {
    index: u8,
    op: Op,
}

#[derive(Debug, Clone)]
enum Op {
    Create {
        instruction: SerializableInstruction,
        priority_fee: u64,
        lookup_tables: Vec<Pubkey>,
    },
    Update {
        /// `Some(None)` is a deliberate wipe; `None` is "not mentioned".
        instruction: Option<Option<SerializableInstruction>>,
        priority_fee: Option<u64>,
        lookup_tables: Option<Vec<Pubkey>>,
    },
    Close,
}

/// Decode one instruction's data into a fiber write, if it is one.
///
/// Dispatches on the Anchor discriminator and then deserialises with the
/// program's own generated argument struct, so this cannot drift from the
/// programs the way a hand-written layout would.
fn decode_write(program: &Pubkey, data: &[u8]) -> Option<Delta> {
    use antegen_fiber_program::instruction as fx;
    use antegen_thread_program::instruction as tx;

    if data.len() < 8 {
        return None;
    }
    let (disc, mut args) = data.split_at(8);

    let thread_program = antegen_thread_program::ID;
    let fiber_program = antegen_fiber_program::ID;

    if *program == thread_program {
        if disc == tx::CreateThread::DISCRIMINATOR {
            let ix = tx::CreateThread::deserialize(&mut args).ok()?;
            // A thread created without a default instruction has no fiber 0.
            let instruction = ix.instruction?;
            return Some(Delta {
                index: 0,
                op: Op::Create {
                    instruction,
                    priority_fee: ix.priority_fee.unwrap_or(0),
                    lookup_tables: ix.lookup_tables.into_inner(),
                },
            });
        }
        if disc == tx::CreateFiber::DISCRIMINATOR {
            let ix = tx::CreateFiber::deserialize(&mut args).ok()?;
            return Some(Delta {
                index: ix.fiber_index,
                op: Op::Create {
                    instruction: ix.instruction,
                    priority_fee: ix.priority_fee,
                    lookup_tables: ix.lookup_tables.into_inner(),
                },
            });
        }
        if disc == tx::UpdateFiber::DISCRIMINATOR {
            let ix = tx::UpdateFiber::deserialize(&mut args).ok()?;
            return Some(Delta {
                index: ix.fiber_index,
                op: Op::Update {
                    instruction: Some(ix.instruction),
                    priority_fee: ix.priority_fee,
                    lookup_tables: ix.lookup_tables.into_inner(),
                },
            });
        }
        if disc == tx::CloseFiber::DISCRIMINATOR {
            let ix = tx::CloseFiber::deserialize(&mut args).ok()?;
            return Some(Delta {
                index: ix.fiber_index,
                op: Op::Close,
            });
        }
        return None;
    }

    if *program == fiber_program {
        if disc == fx::Create::DISCRIMINATOR {
            let ix = fx::Create::deserialize(&mut args).ok()?;
            return Some(Delta {
                index: ix.fiber_index,
                op: Op::Create {
                    instruction: ix.instruction,
                    priority_fee: ix.priority_fee,
                    lookup_tables: ix.lookup_tables.into_inner(),
                },
            });
        }
        if disc == fx::Update::DISCRIMINATOR {
            let ix = fx::Update::deserialize(&mut args).ok()?;
            return Some(Delta {
                index: ix.fiber_index,
                op: Op::Update {
                    instruction: Some(ix.instruction),
                    priority_fee: ix.priority_fee,
                    lookup_tables: ix.lookup_tables.into_inner(),
                },
            });
        }
        if disc == fx::Close::DISCRIMINATOR {
            let ix = fx::Close::deserialize(&mut args).ok()?;
            return Some(Delta {
                index: ix.fiber_index,
                op: Op::Close,
            });
        }
    }

    None
}

/// Replay a fiber's writes into its final state.
///
/// Order is what makes this correct. Fibers are routinely rewritten long after
/// creation, so the creation transaction alone reconstructs stale content.
fn fold(deltas: &[Delta]) -> Option<FiberState> {
    let mut state: Option<FiberState> = None;

    for d in deltas {
        match &d.op {
            Op::Close => state = None,
            Op::Create {
                instruction,
                priority_fee,
                lookup_tables,
            } => {
                state = Some(FiberState {
                    instruction: Some(to_json(instruction)),
                    priority_fee: *priority_fee,
                    lookup_tables: lookup_tables.iter().map(|k| k.to_string()).collect(),
                });
            }
            Op::Update {
                instruction,
                priority_fee,
                lookup_tables,
            } => {
                // `update` initialises when the account does not exist, so it
                // can legitimately be the first write a fiber ever sees.
                let cur = state.get_or_insert(FiberState {
                    instruction: None,
                    priority_fee: 0,
                    lookup_tables: Vec::new(),
                });
                if let Some(ix) = instruction {
                    cur.instruction = ix.as_ref().map(to_json);
                }
                if let Some(fee) = priority_fee {
                    cur.priority_fee = *fee;
                }
                if let Some(alts) = lookup_tables {
                    cur.lookup_tables = alts.iter().map(|k| k.to_string()).collect();
                }
            }
        }
    }

    state
}

fn to_json(ix: &SerializableInstruction) -> InstructionJson {
    use base64::Engine;
    InstructionJson {
        program_id: ix.program_id.to_string(),
        accounts: ix
            .accounts
            .iter()
            .map(|a| AccountMetaJson {
                pubkey: a.pubkey.to_string(),
                is_signer: a.is_signer,
                is_writable: a.is_writable,
            })
            .collect(),
        data_b64: base64::engine::general_purpose::STANDARD.encode(&ix.data),
    }
}

pub(crate) fn fiber_pda(thread: &Pubkey, index: u8) -> Pubkey {
    Pubkey::find_program_address(
        &[SEED_THREAD_FIBER, thread.as_ref(), &[index]],
        &antegen_fiber_program::ID,
    )
    .0
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

/// Every instruction in a transaction, top level and CPI'd, paired with the
/// program that ran it.
///
/// Reading only the top level misses most fiber writes: they usually arrive as
/// inner instructions when an integrator's program CPIs in.
fn instructions_of(tx: &serde_json::Value) -> Vec<(Pubkey, Vec<u8>, Vec<Pubkey>)> {
    let msg = &tx["transaction"]["message"];
    let mut keys: Vec<String> = msg["accountKeys"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|k| {
                    k.as_str()
                        .map(str::to_owned)
                        .or_else(|| k["pubkey"].as_str().map(str::to_owned))
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default();

    for field in ["writable", "readonly"] {
        if let Some(extra) = tx["meta"]["loadedAddresses"][field].as_array() {
            keys.extend(extra.iter().filter_map(|k| k.as_str().map(str::to_owned)));
        }
    }

    let mut out = Vec::new();
    let mut push = |ix: &serde_json::Value| {
        let program = ix["programIdIndex"]
            .as_u64()
            .and_then(|i| keys.get(i as usize))
            .and_then(|k| Pubkey::from_str(k).ok());
        let (Some(program), Some(data)) = (program, ix["data"].as_str()) else {
            return;
        };
        let accounts: Vec<Pubkey> = ix["accounts"]
            .as_array()
            .map(|idxs| {
                idxs.iter()
                    .filter_map(|i| i.as_u64())
                    .filter_map(|i| keys.get(i as usize))
                    .filter_map(|k| Pubkey::from_str(k).ok())
                    .collect()
            })
            .unwrap_or_default();
        if let Ok(bytes) = bs58::decode(data).into_vec() {
            out.push((program, bytes, accounts));
        }
    };

    if let Some(ixs) = msg["instructions"].as_array() {
        ixs.iter().for_each(&mut push);
    }
    if let Some(groups) = tx["meta"]["innerInstructions"].as_array() {
        for g in groups {
            if let Some(ixs) = g["instructions"].as_array() {
                ixs.iter().for_each(&mut push);
            }
        }
    }
    out
}

/// Collect every write to the given fiber indices of one thread, in slot order.
///
/// History is gathered from the thread *and* from each fiber address: writes
/// that went straight to the fiber program never touched the thread account and
/// would be invisible otherwise.
async fn history(
    rpc: &RpcPool,
    thread: &Pubkey,
    indices: &[u8],
) -> Result<(BTreeMap<u8, Vec<Delta>>, usize, Vec<String>)> {
    let mut sigs: Vec<SignatureRecord> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let mut addresses = vec![*thread];
    addresses.extend(indices.iter().map(|i| fiber_pda(thread, *i)));

    for addr in addresses {
        for s in rpc.get_signatures_for_address(&addr).await? {
            if s.err.is_none() && seen.insert(s.signature.clone()) {
                sigs.push(s);
            }
        }
    }
    sigs.sort_by_key(|s| (s.slot, s.block_time.unwrap_or(0)));

    let mut per_index: BTreeMap<u8, Vec<Delta>> =
        indices.iter().map(|i| (*i, Vec::new())).collect();
    let mut warnings = Vec::new();

    for s in &sigs {
        let Some(tx) = rpc.get_transaction(&s.signature).await? else {
            warnings.push(format!(
                "{}: not returned by the endpoint — history gap, reconstruction \
                 may be incomplete",
                s.signature
            ));
            continue;
        };
        for (program, data, accounts) in instructions_of(&tx) {
            let Some(delta) = decode_write(&program, &data) else {
                continue;
            };
            let Some(slot) = per_index.get_mut(&delta.index) else {
                continue;
            };
            // Two things have to hold for a write to belong to this fiber's
            // history, and neither is implied by the instruction's index.
            //
            // It must name the account being rebuilt: a transaction reaches
            // this loop because it mentioned our thread, but it may also write
            // a different thread's fiber at the same index.
            //
            // It must also name the owning thread. Before `fiber::create`
            // checked the PDA it derived, anyone could call it against another
            // thread's fiber with their own wallet in the `thread` position and
            // overwrite the payload — which is exactly what happened on
            // 2026-08-26, and those writes are in this history. They pass the
            // first check, because they do name the fiber. They fail this one,
            // because the real thread is nowhere in their accounts.
            //
            // Folding one in would rebuild the fiber from what the attacker
            // supplied rather than from what the thread last legitimately ran.
            let target = fiber_pda(thread, delta.index);
            if accounts.contains(&target) && accounts.contains(thread) {
                slot.push(delta);
            }
        }
    }

    Ok((per_index, sigs.len(), warnings))
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub(crate) async fn doctor(
    address: Option<String>,
    rpc_url: Option<String>,
    opts: DoctorOpts,
) -> Result<()> {
    let rpc_url = get_rpc_url(rpc_url)?;
    let rpc =
        RpcPool::with_url(&rpc_url).map_err(|e| anyhow!("Failed to create RPC client: {}", e))?;

    if let Some(path) = &opts.confirm {
        return confirm(&rpc, path).await;
    }
    if opts.verify {
        return verify(&rpc).await;
    }

    let threads = load_threads(&rpc, address.as_deref()).await?;
    let scanned = threads.len();

    let mut results: Vec<ThreadReport> = Vec::new();
    let mut budget = opts.limit;

    for (pubkey, thread) in threads {
        let tracked: Vec<u8> = thread
            .fiber_ids
            .iter()
            .copied()
            .filter(|i| opts.fiber_index.is_none_or(|want| want == *i))
            .collect();
        if tracked.is_empty() {
            continue;
        }

        let pdas: Vec<Pubkey> = tracked.iter().map(|i| fiber_pda(&pubkey, *i)).collect();
        let accounts = rpc
            .get_multiple_accounts(&pdas)
            .await
            .map_err(|e| anyhow!("failed to fetch fiber accounts: {}", e))?;

        let mut missing: Vec<u8> = tracked
            .iter()
            .zip(accounts.iter())
            .filter(|(_, a)| a.is_none())
            .map(|(i, _)| *i)
            .collect();
        if missing.is_empty() {
            continue;
        }
        if let Some(room) = budget {
            if room == 0 {
                break;
            }
            missing.truncate(room);
            budget = Some(room - missing.len());
        }

        let mut report = ThreadReport {
            thread: pubkey.to_string(),
            authority: thread.authority.to_string(),
            fiber_ids: thread.fiber_ids.clone(),
            fiber_cursor: thread.fiber_cursor,
            missing: missing.clone(),
            signatures_scanned: 0,
            fibers: BTreeMap::new(),
            warnings: Vec::new(),
        };

        if opts.plan {
            let (per_index, scanned_sigs, warnings) = history(&rpc, &pubkey, &missing).await?;
            report.signatures_scanned = scanned_sigs;
            report.warnings = warnings;
            for idx in &missing {
                let deltas = per_index.get(idx).cloned().unwrap_or_default();
                report.fibers.insert(
                    *idx,
                    RecoveredFiber {
                        fiber_pda: fiber_pda(&pubkey, *idx).to_string(),
                        writes_replayed: deltas.len(),
                        recovered: fold(&deltas),
                    },
                );
            }
        } else {
            for idx in &missing {
                report.fibers.insert(
                    *idx,
                    RecoveredFiber {
                        fiber_pda: fiber_pda(&pubkey, *idx).to_string(),
                        writes_replayed: 0,
                        recovered: None,
                    },
                );
            }
        }

        results.push(report);
    }

    let fibers_missing: usize = results.iter().map(|r| r.missing.len()).sum();
    let fibers_reconstructed = results
        .iter()
        .flat_map(|r| r.fibers.values())
        .filter(|f| {
            f.recovered
                .as_ref()
                .is_some_and(|s| s.instruction.is_some())
        })
        .count();
    let unresolved: Vec<String> = results
        .iter()
        .flat_map(|r| {
            r.fibers
                .iter()
                .filter(|(_, f)| f.recovered.as_ref().is_none_or(|s| s.instruction.is_none()))
                .map(move |(i, _)| format!("{} idx={}", r.thread, i))
        })
        .collect();

    let manifest = Manifest {
        thread_program: antegen_thread_program::ID.to_string(),
        fiber_program: antegen_fiber_program::ID.to_string(),
        threads_scanned: scanned,
        threads_unhealthy: results.len(),
        fibers_missing,
        fibers_reconstructed,
        unresolved: unresolved.clone(),
        results,
    };

    if let Some(path) = &opts.output {
        std::fs::write(path, serde_json::to_string_pretty(&manifest)?)
            .with_context(|| format!("writing report to {}", path.display()))?;
    } else if opts.json {
        println!("{}", serde_json::to_string_pretty(&manifest)?);
        return Ok(());
    }

    render(&manifest, opts.plan, opts.output.as_deref());

    // A thread that cannot build is a failure state, not a clean bill of
    // health — exit non-zero so this is usable in a check.
    if manifest.threads_unhealthy > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn render(m: &Manifest, planned: bool, output: Option<&std::path::Path>) {
    if m.threads_unhealthy == 0 {
        println!(
            "healthy — {} thread(s) scanned, every tracked fiber account exists",
            m.threads_scanned
        );
        return;
    }

    for r in &m.results {
        println!(
            "\x1b[31mUNHEALTHY\x1b[0m {}  fiber_ids={:?} cursor={}",
            r.thread, r.fiber_ids, r.fiber_cursor
        );
        for idx in &r.missing {
            let f = &r.fibers[idx];
            let state = match &f.recovered {
                Some(s) if s.instruction.is_some() => {
                    format!("reconstructed from {} write(s)", f.writes_replayed)
                }
                Some(_) => "reconstructed as idle (no instruction)".to_string(),
                None if planned => "NOT RECOVERABLE from history".to_string(),
                None => "missing".to_string(),
            };
            println!("    fiber[{}] {}  {}", idx, f.fiber_pda, state);
        }
        for w in &r.warnings {
            println!("    \x1b[33mwarning\x1b[0m {}", w);
        }
    }

    println!();
    println!("threads scanned      {}", m.threads_scanned);
    println!("threads unhealthy    {}", m.threads_unhealthy);
    println!("fibers missing       {}", m.fibers_missing);
    if planned {
        println!("fibers reconstructed {}", m.fibers_reconstructed);
        println!("unresolved           {}", m.unresolved.len());
    }
    if let Some(p) = output {
        println!("\nreport written to {}", p.display());
    }
    if planned {
        println!(
            "\nThis command cannot carry the plan out — writing a fiber needs \
             the thread's authority to sign, and for these threads that is a \
             program, not a keypair."
        );
    } else {
        println!("\nRe-run with --plan to work out what repairing these would take.");
    }
}

async fn load_threads(rpc: &RpcPool, address: Option<&str>) -> Result<Vec<(Pubkey, Thread)>> {
    if let Some(addr) = address {
        let pubkey = Pubkey::from_str(addr).map_err(|_| anyhow!("invalid pubkey: {}", addr))?;
        let account = rpc
            .get_account(&pubkey)
            .await
            .map_err(|e| anyhow!("failed to fetch {}: {}", pubkey, e))?
            .ok_or_else(|| anyhow!("thread {} does not exist", pubkey))?;
        let data = decode_account(&account)?;
        let thread = Thread::try_deserialize(&mut data.as_slice())
            .map_err(|e| anyhow!("{} is not a thread account: {}", pubkey, e))?;
        return Ok(vec![(pubkey, thread)]);
    }

    let (_, accounts) = rpc
        .get_program_accounts(&antegen_thread_program::ID, None)
        .await
        .map_err(|e| anyhow!("failed to list threads: {}", e))?;

    Ok(accounts
        .into_iter()
        .filter_map(|(pk, acct)| {
            let data = decode_account(&acct).ok()?;
            Thread::try_deserialize(&mut data.as_slice())
                .ok()
                .map(|t| (pk, t))
        })
        .collect())
}

fn decode_account(account: &antegen_client::rpc::SafeUiAccount) -> Result<Vec<u8>> {
    decode_account_data(&account.data.0, &account.data.1)
        .map_err(|e| anyhow!("failed to decode account data: {}", e))
}

// ---------------------------------------------------------------------------
// Proofs
// ---------------------------------------------------------------------------

/// Replay history for fibers that still exist and diff against the chain.
///
/// Worth running before replaying anything, but note what it cannot show. A
/// surviving fiber is one nobody hijacked, so its history contains no forged
/// write — this pass stayed green through a bug that was rebuilding destroyed
/// fibers from the attacker's payload. It demonstrates the fold and the decode
/// are right; it cannot demonstrate that the writes selected for a *destroyed*
/// fiber were the legitimate ones, because there is no ground truth left for
/// those. That part is carried by the filtering in `history`.
async fn verify(rpc: &RpcPool) -> Result<()> {
    let (_, live) = rpc
        .get_program_accounts(&antegen_fiber_program::ID, None)
        .await
        .map_err(|e| anyhow!("failed to list fibers: {}", e))?;

    if live.is_empty() {
        println!("no surviving fiber accounts to verify against");
        return Ok(());
    }

    let threads: BTreeMap<Pubkey, Thread> = load_threads(rpc, None).await?.into_iter().collect();

    let mut failures = 0usize;
    let mut checked = 0usize;

    for (pda, account) in live {
        let data = decode_account(&account)?;
        let Ok(on_chain) = Fiber::try_deserialize(&mut data.as_slice()) else {
            println!("[SKIP] {} unreadable on chain", pda);
            continue;
        };
        let thread_key = on_chain.thread();
        let Some(thread) = threads.get(&thread_key) else {
            println!("[SKIP] {} owning thread {} not found", pda, thread_key);
            continue;
        };
        let Some(index) = thread
            .fiber_ids
            .iter()
            .copied()
            .find(|i| fiber_pda(&thread_key, *i) == pda)
        else {
            println!("[SKIP] {} not tracked by its thread", pda);
            continue;
        };

        let (per_index, _, _) = history(rpc, &thread_key, &[index]).await?;
        let deltas = per_index.get(&index).cloned().unwrap_or_default();
        let rebuilt = fold(&deltas);

        checked += 1;
        let diffs = compare_to_chain(&on_chain, rebuilt.as_ref());
        if diffs.is_empty() {
            println!(
                "[ ok ] {} idx={} matches on-chain state ({} write(s) replayed)",
                pda,
                index,
                deltas.len()
            );
        } else {
            failures += 1;
            println!("[FAIL] {} idx={} thread={}", pda, index, thread_key);
            for d in diffs {
                println!("         {}", d);
            }
        }
    }

    println!(
        "\nverified {} surviving fiber(s), {} mismatch(es)",
        checked, failures
    );
    if failures > 0 {
        println!("DO NOT REPLAY: reconstruction disagrees with ground truth.");
        std::process::exit(1);
    }
    Ok(())
}

/// After replaying, diff what is on chain against the manifest that was
/// replayed.
///
/// Deliberately compares against the manifest rather than re-deriving from
/// history: once a fiber has been replayed, the replay transaction is itself
/// part of that fiber's history, so a history-based check would fold in the
/// write it is meant to be checking and agree with itself. `--verify` is the
/// proof before, this is the proof after, and they are not interchangeable.
async fn confirm(rpc: &RpcPool, path: &std::path::Path) -> Result<()> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading manifest {}", path.display()))?;
    let manifest: Manifest = serde_json::from_str(&raw)
        .with_context(|| format!("parsing manifest {}", path.display()))?;

    let (mut restored, mut pending, mut mismatched) = (0usize, 0usize, 0usize);

    for report in &manifest.results {
        for (index, fiber) in &report.fibers {
            let Some(expected) = fiber.recovered.as_ref().filter(|s| s.instruction.is_some())
            else {
                continue;
            };
            let pda = Pubkey::from_str(&fiber.fiber_pda)
                .map_err(|_| anyhow!("bad pubkey in manifest: {}", fiber.fiber_pda))?;

            let Some(account) = rpc
                .get_account(&pda)
                .await
                .map_err(|e| anyhow!("failed to fetch {}: {}", pda, e))?
            else {
                pending += 1;
                println!("[----] {} idx={} not replayed yet", pda, index);
                continue;
            };

            let data = decode_account(&account)?;
            let Ok(on_chain) = Fiber::try_deserialize(&mut data.as_slice()) else {
                mismatched += 1;
                println!("[FAIL] {} idx={} unreadable on chain", pda, index);
                continue;
            };

            let diffs = compare_to_chain(&on_chain, Some(expected));
            if diffs.is_empty() {
                restored += 1;
                println!("[ ok ] {} idx={} matches the manifest", pda, index);
            } else {
                mismatched += 1;
                println!("[FAIL] {} idx={} thread={}", pda, index, report.thread);
                for d in diffs {
                    println!("         {}", d);
                }
            }
        }
    }

    println!(
        "\nrestored {}, not yet replayed {}, mismatched {}",
        restored, pending, mismatched
    );
    if mismatched > 0 {
        println!("DO NOT CONTINUE: what landed differs from the manifest.");
        std::process::exit(1);
    }
    Ok(())
}

/// Differences that would change what a fiber executes.
fn compare_to_chain(on_chain: &Fiber, rebuilt: Option<&FiberState>) -> Vec<String> {
    use base64::Engine;

    let Some(rebuilt) = rebuilt else {
        return vec!["history yielded nothing to compare".to_string()];
    };
    let Some(expected) = &rebuilt.instruction else {
        return vec!["reconstruction has no instruction".to_string()];
    };

    let compiled = on_chain.compiled_instruction();
    if compiled.is_empty() {
        return vec!["on-chain fiber is idle (empty instruction)".to_string()];
    }
    let decoded =
        match antegen_thread_program::fiber::CompiledInstructionV0::try_from_slice(compiled)
            .and_then(|c| {
                antegen_thread_program::fiber::decompile_instruction(&c)
                    .map_err(|_| std::io::Error::other("decompile failed"))
            }) {
            Ok(ix) => ix,
            Err(e) => return vec![format!("on-chain instruction is unreadable: {}", e)],
        };

    let mut diffs = Vec::new();
    if decoded.program_id.to_string() != expected.program_id {
        diffs.push(format!(
            "program_id {} on chain vs {} rebuilt",
            decoded.program_id, expected.program_id
        ));
    }
    let on_data = base64::engine::general_purpose::STANDARD.encode(&decoded.data);
    if on_data != expected.data_b64 {
        diffs.push("instruction data differs".to_string());
    }
    if decoded.accounts.len() != expected.accounts.len() {
        diffs.push(format!(
            "account count differs: {} on chain vs {} rebuilt",
            decoded.accounts.len(),
            expected.accounts.len()
        ));
    } else {
        // Same length, so name the positions that moved — a count says nothing
        // when both sides are the same size.
        for (i, (a, b)) in decoded.accounts.iter().zip(&expected.accounts).enumerate() {
            if a.pubkey.to_string() != b.pubkey {
                diffs.push(format!(
                    "account[{}] {} on chain vs {} rebuilt",
                    i, a.pubkey, b.pubkey
                ));
            }
        }
    }
    if on_chain.priority_fee() != rebuilt.priority_fee {
        diffs.push(format!(
            "priority_fee {} on chain vs {} rebuilt",
            on_chain.priority_fee(),
            rebuilt.priority_fee
        ));
    }
    diffs
}
