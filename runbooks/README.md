# antegen Runbooks

[![Surfpool](https://img.shields.io/badge/Operated%20with-Surfpool-gree?labelColor=gray)](https://surfpool.run)

## Available Runbooks

### deployment
Deploy programs

## Getting Started

This repository is using [Surfpool](https://surfpool.run) as a part of its development workflow.

Surfpool provides three major upgrades to the Solana development experience:

- **Surfnet**: A local validator that runs on your machine, allowing you fork mainnet on the fly so that you always use the latest chain data when testing your programs.
- **Runbooks**: Bringing the devops best practice of `infrastructure as code` to Solana, Runbooks allow you to have secure, reproducible, and composable scripts for managing on-chain operations & deployments.
- **Surfpool Studio**: An all-local Web UI that gives new levels of introspection into your transactions.

### Installation

Install pre-built binaries:

```console
# macOS (Homebrew)
brew install txtx/taps/surfpool

# Updating surfpool for Homebrew users
brew tap txtx/taps
brew reinstall surfpool

# Linux (Snap Store)
snap install surfpool
```

Install from source:

```console
# Clone repo
git clone https://github.com/txtx/surfpool.git

# Set repo as current directory
cd surfpool

# Build
cargo surfpool-install
```

### Start a Surfnet

```console
$ surfpool start
```

## Resources

Access tutorials and documentation at [docs.surfpool.run](https://docs.surfpool.run) to understand Surfnets and the Runbook syntax, and to discover the powerful features of surfpool.

Additionally, the [Visual Studio Code extension](https://marketplace.visualstudio.com/items?itemName=txtx.txtx) will make writing runbooks easier.

Our [Surfpool 101 Series](https://www.youtube.com/playlist?list=PL0FMgRjJMRzO1FdunpMS-aUS4GNkgyr3T) is also a great place to start learning about Surfpool and its features:
<a href="https://www.youtube.com/playlist?list=PL0FMgRjJMRzO1FdunpMS-aUS4GNkgyr3T">
  <picture>
    <source srcset="https://raw.githubusercontent.com/txtx/surfpool/main/doc/assets/youtube.png">
    <img alt="Surfpool 101 series" style="max-width: 100%;">
  </picture>
</a>

## Quickstart

### List runbooks available in this repository
```console
$ surfpool ls
Name                                    Description
deployment                              Deploy programs
```

### Start a Surfnet, automatically executing the `deployment` runbook on program recompile:
```console
$ surfpool start --watch
```

### Execute an existing runbook
```console
$ surfpool run deployment
```
