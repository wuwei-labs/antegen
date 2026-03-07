# Antegen: From Protocol Platform to Production-Ready Executor

A lot has changed since v2.2.9. Here's a high-level look at where Antegen is today.

## A New Foundation

Antegen started as a hard fork of Clockwork. Since then, the project has been completely restructured — new crate layout, new on-chain program architecture, and a modern dependency stack built on **Solana v3** and **Anchor 1.0**.

The old multi-program design (thread, network, webhook) has been replaced with a single, focused **Thread program** for scheduled transaction execution. Fewer moving parts, simpler mental model, easier to audit.

## One Binary, Fully Self-Managed

The CLI is now a single `antegen` binary that handles everything:

- **Install & update itself** — `antegen node update` pulls the latest release, swaps the binary atomically, and restarts the service.
- **Run as a system service** — `antegen node start` configures and launches a systemd/launchd service automatically. No manual unit files.
- **Fund & withdraw** — `antegen node fund` tops up your executor to the minimum balance. `antegen node withdraw` pulls out everything above it.
- **Observe** — `antegen node info` shows executor address, balance, service status, and version at a glance. `antegen node logs` streams live output.

## On-Chain: Simplified Thread Program

The thread program has been redesigned around **fibers** — individual instructions within a thread's execution sequence. Key changes:

- New instruction set: `fiber_create`, `fiber_delete`, `fiber_update` replace the old `thread_instruction_add/remove` pattern
  - Enabling on-chain "chaining" like n8n workflows
- Global configuration via `config_init` / `config_update`
- Fairer fee model — execution fees decay over time, so threads that run late don't pay full price (can be 0 if too late)
- Durable nonces for transaction reliability across all threads (not enabled for default threads)

## Under the Hood

- **Actor-based execution** via Ractor for concurrent thread processing
- **TPU direct submission** bypasses RPC for lower-latency transactions
- **RPC load balancing** with race condition handling and automatic requeueing
- **Workspace claims** via loa-core integration for the agent builder ecosystem

## Modern Stack

| Component | v2.2.9 | v4.8.2 |
|---|---|---|
| Solana | 1.x | 3.1.4 |
| Anchor | 0.29.x | 1.0.0-rc.1 |
| Rust | 1.81 | 1.82+ |
| Programs | 4 (thread, network, webhook, test) | 1 (thread) |
