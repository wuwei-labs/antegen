---
icon: home
order: 100
---

# Welcome

<div align="center">
  <p>
    <strong>Solana automation engine</strong>
  </p>

  <p>
    <a href="https://github.com/wuwei-labs/antegen/actions/workflows/build-status.yaml"><img alt="build status" src="https://github.com/wuwei-labs/antegen/actions/workflows/build-status.yaml/badge.svg"/></a>&nbsp;&nbsp;&nbsp;
    <a href="https://discord.com/channels/1328480150676836462"><img alt="Discord Chat" src="https://img.shields.io/discord/1328480150676836462?color=blueviolet" /></a>&nbsp;&nbsp;&nbsp;
    <a href="https://www.gnu.org/licenses/agpl-3.0.en.html"><img alt="License" src="https://img.shields.io/github/license/wuwei-labs/antegen?color=turquoise" /></a>
  </p>
</div>

## Why was this fork created?

Antegen is a **hard** fork from [Clockwork](https://github.com/clockwork-xyz/clockwork) at commit [44d2038](https://github.com/clockwork-xyz/clockwork/commit/44d2038931da60ba3e192a833096fabee0422d44). This was done to continue the development of the protocol in an Open Source environment. The rebranding from Clockwork to Antegen became necessary due to limitations in accessing and updating the crates.io packages and on-chain programs.

## Fork Details

Antegen maintains the core protocol design from Clockwork while modernizing the codebase for compatibility with the latest Solana ecosystem dependencies. This includes support for:

- Anchor 30.1 and above
- Solana 2.1.13 (Agave) and above
- Latest ecosystem dependencies and standards
- Modern development tooling

This open-source development path enables:

- Community-driven improvements
- Rapid security patches
- Continuous dependency updates
- Enhanced protocol stability

## Opinionated Changes

- Removed Clockwork ThreadV1 program
- Removed Clockwork Webhook program
- Updated Clockwork ThreadV2 to Antegen ThreadV1

## Alternatives

There's been work on a truly open source version of Clockwork via the [Open Clockwork](https://github.com/open-clockwork/clockwork) project. Which appears to take the approach of carrying the torch from where the original project left off ensure backwards compatibility.

> [!NOTE]
> Antegen is now under active development as its own project. While it shares historical roots with Clockwork, all interfaces and implementations are subject to change as the project evolves to meet its unique objectives.

## Deployments

| Program | Address| Devnet | Mainnet |
| ------- | ------ | ------ | ------- |
| Network | `AgNet6qmh75bjFULcS9RQijUoWwkCtSiSwXM1K3Ujn6Z` |  |  |
| Thread v1 | `AgThdyi1P5RkVeZD2rQahTvs8HePJoGFFxKtvok5s2J1` |  |  |

## SDKs

| Language | Description  | Lib  | Examples |
| ----------- | -------- | ---- | -------- |
| Anchor |  Anchor bindings for Solana programs.  | [crates.io](https://crates.io/crates/antegen-sdk) |  |
| Rust | Rust bindings for clients.  | [crates.io](https://crates.io/crates/antegen-client) |  |
| Typescript | Typescript bindings for clients and frontends.  |  |  |


## Local Development

---

#### 1. Install Rust

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

#### 2. Install Solana CLI

```sh
sh -c "$(curl -sSfL https://release.anza.xyz/v2.1.13/install)"
```

#### 3. Install Anchor (avm)

```sh
cargo install --git https://github.com/coral-xyz/anchor avm --locked --force
```

```sh
avm install latest
```

#### 4. Install antegen-cli

If you are on linux, you might need to run this:

```sh
sudo apt-get update && sudo apt-get upgrade && sudo apt-get install -y pkg-config build-essential libudev-dev libssl-dev
```

Install with cargo:

```sh
cargo build --workspace
cargo install --path cli
```

#### 5. Anchor Build

> <https://solana.stackexchange.com/questions/17777/unexpected-cfg-condition-value-solana>

```sh
anchor build
```

#### 6. Run a localnet node

```sh
antegen localnet --dev
```

#### 7. Stream program logs

```sh
tail -f validator.log
solana logs
```
