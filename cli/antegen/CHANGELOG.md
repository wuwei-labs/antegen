# Changelog

## [8.0.0](https://github.com/wuwei-labs/antegen/compare/antegen-cli-v7.0.0...antegen-cli-v8.0.0) (2026-08-16)


### ⚠ BREAKING CHANGES

* **cli-core:** `antegen update`, `antegen list`, `antegen use`, `antegen install` and `antegen verify` are removed; re-run the install script to update the CLI, and use `antegen node <cmd>` for the daemon. The corresponding functions are removed from `antegen_cli_core::commands::update`.
* **cli:** the `antegenctl` binary and crate are removed. Its commands are `antegen node <cmd>`. Service control and node version management moved off the top level of `antegen`; the old spellings warn for one release, except `antegen verify`, which is unchanged.
* **cli:** `antegen-client` no longer builds the `antegen-node` binary and its `node` feature is removed; the library is unchanged. `antegen run` is now `antegen node run` and no longer takes `--version`, since it runs the calling binary rather than exec'ing another one.

### Features

* :sparkles: add full deploy mode for fiber + thread programs ([2111b84](https://github.com/wuwei-labs/antegen/commit/2111b8408ad7f60eca3480f74144ccafd4da23ee))
* **cli:** add a load generator for benchmarking, and fix the deploy runbook ([211bb4a](https://github.com/wuwei-labs/antegen/commit/211bb4aa1507bf15be2b06c4e31890a934ca0db3))
* **cli:** run the daemon from `antegen node run` ([2b3069c](https://github.com/wuwei-labs/antegen/commit/2b3069c399ddf8b6b1553d7e919d232c15a17616))


### Code Refactoring

* **cli-core:** delete CLI self-management ([2851ae7](https://github.com/wuwei-labs/antegen/commit/2851ae79ff3aaa019862bffbffed409a7e0f92f3))
* **cli:** group operator commands under `antegen node` ([22f465b](https://github.com/wuwei-labs/antegen/commit/22f465bc4c50164195add3580d0d9f5152d776b9))

## [7.0.0](https://github.com/wuwei-labs/antegen/compare/antegen-cli-v6.1.0...antegen-cli-v7.0.0) (2026-08-16)


### ⚠ BREAKING CHANGES

* **cli-core:** `antegen update`, `antegen list`, `antegen use`, `antegen install` and `antegen verify` are removed; re-run the install script to update the CLI, and use `antegen node <cmd>` for the daemon. The corresponding functions are removed from `antegen_cli_core::commands::update`.
* **cli:** the `antegenctl` binary and crate are removed. Its commands are `antegen node <cmd>`. Service control and node version management moved off the top level of `antegen`; the old spellings warn for one release, except `antegen verify`, which is unchanged.
* **cli:** `antegen-client` no longer builds the `antegen-node` binary and its `node` feature is removed; the library is unchanged. `antegen run` is now `antegen node run` and no longer takes `--version`, since it runs the calling binary rather than exec'ing another one.

### Features

* **cli:** add a load generator for benchmarking, and fix the deploy runbook ([211bb4a](https://github.com/wuwei-labs/antegen/commit/211bb4aa1507bf15be2b06c4e31890a934ca0db3))
* **cli:** run the daemon from `antegen node run` ([2b3069c](https://github.com/wuwei-labs/antegen/commit/2b3069c399ddf8b6b1553d7e919d232c15a17616))


### Code Refactoring

* **cli-core:** delete CLI self-management ([2851ae7](https://github.com/wuwei-labs/antegen/commit/2851ae79ff3aaa019862bffbffed409a7e0f92f3))
* **cli:** group operator commands under `antegen node` ([22f465b](https://github.com/wuwei-labs/antegen/commit/22f465bc4c50164195add3580d0d9f5152d776b9))

## [6.1.0](https://github.com/wuwei-labs/antegen/compare/antegen-cli-v6.0.0...antegen-cli-v6.1.0) (2026-05-17)


### Features

* :sparkles: add full deploy mode for fiber + thread programs ([2111b84](https://github.com/wuwei-labs/antegen/commit/2111b8408ad7f60eca3480f74144ccafd4da23ee))

## Changelog
