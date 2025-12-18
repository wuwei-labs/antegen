# antegen-cli

Command-line interface for [Antegen](https://antegen.xyz) - Solana automation and scheduling.

## Installation

### From crates.io

```bash
cargo install antegen-cli
```

### From script

```bash
curl -sSfL https://antegen.xyz/install.sh | sh
```

### From source

```bash
git clone https://github.com/wuwei-labs/antegen
cd antegen
cargo install --path crates/cli
```

## Usage

```bash
# Show help
antegen --help

# Get thread info
antegen thread get <THREAD_PUBKEY>

# Run executor
antegen run --config antegen.toml
```

## Configuration

The CLI reads Solana configuration from `~/.config/solana/cli/config.yml` by default.

## Documentation

- [Documentation](https://docs.antegen.xyz)
- [GitHub](https://github.com/wuwei-labs/antegen)

## License

MIT
