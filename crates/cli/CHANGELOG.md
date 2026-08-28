# Changelog

## [8.3.0](https://github.com/wuwei-labs/antegen/compare/antegen-cli-v8.2.0...antegen-cli-v8.3.0) (2026-08-26)


### Features

* **cli:** add `thread doctor` and drop the standalone recovery script ([22298a7](https://github.com/wuwei-labs/antegen/commit/22298a700d0fb182a410493a5d47d26a340fdffc))
* **cli:** add `thread doctor`, and the post-mortem that produced it ([a3bcea0](https://github.com/wuwei-labs/antegen/commit/a3bcea0e9718ec7351af4b033d4cd0f8cc8a4da7))
* **cli:** report writes excluded as not made by the thread's authority ([5b09fda](https://github.com/wuwei-labs/antegen/commit/5b09fdad3e002d943f0ff6fe5a5a0e03cc1bf2c9))


### Bug Fixes

* **cli:** exclude forged writes when rebuilding a fiber ([3a33327](https://github.com/wuwei-labs/antegen/commit/3a33327f54540a5ec7f8af879d32c26189436449))

## [8.2.0](https://github.com/wuwei-labs/antegen/compare/antegen-cli-v8.1.0...antegen-cli-v8.2.0) (2026-08-26)


### Features

* **cli:** add `thread doctor` and drop the standalone recovery script ([22298a7](https://github.com/wuwei-labs/antegen/commit/22298a700d0fb182a410493a5d47d26a340fdffc))
* **cli:** add `thread doctor`, and the post-mortem that produced it ([a3bcea0](https://github.com/wuwei-labs/antegen/commit/a3bcea0e9718ec7351af4b033d4cd0f8cc8a4da7))
* **cli:** report writes excluded as not made by the thread's authority ([5b09fda](https://github.com/wuwei-labs/antegen/commit/5b09fdad3e002d943f0ff6fe5a5a0e03cc1bf2c9))


### Bug Fixes

* **cli:** exclude forged writes when rebuilding a fiber ([3a33327](https://github.com/wuwei-labs/antegen/commit/3a33327f54540a5ec7f8af879d32c26189436449))

## [8.1.0](https://github.com/wuwei-labs/antegen/compare/antegen-cli-v8.0.0...antegen-cli-v8.1.0) (2026-08-25)


### Features

* **cli:** add `thread exec` for manually triggering a thread ([eed2a84](https://github.com/wuwei-labs/antegen/commit/eed2a842e1ee754b8d28b6cd1cb5c352a29d78c5))
* **cli:** add `thread exec` for manually triggering a thread ([bd16bea](https://github.com/wuwei-labs/antegen/commit/bd16beaa69947171a7ac1d547f47bd888b1bad7c))

## [8.0.0](https://github.com/wuwei-labs/antegen/compare/antegen-cli-v7.0.1...antegen-cli-v8.0.0) (2026-08-22)


### ⚠ BREAKING CHANGES

* **cli:** the `antegen-cli-core` crate is removed; its contents are modules of `antegen-cli`. Neither crate was published, so this affects only in-repo paths.

### Code Refactoring

* **cli:** merge cli-core into the CLI and move it to crates/cli ([c3b8db2](https://github.com/wuwei-labs/antegen/commit/c3b8db2af98a84077fcb17953e1f439634ce1219))

## [7.0.1](https://github.com/wuwei-labs/antegen/compare/antegen-cli-v7.0.0...antegen-cli-v7.0.1) (2026-08-16)


### Bug Fixes

* **cli:** stop publishing antegen-cli and antegen-cli-core ([52663b5](https://github.com/wuwei-labs/antegen/commit/52663b575b157cbe0af66b6fc7d4a47cb8e47d6b))
* keep the node alive, stop leaking credentials, and hold the release gate ([c1e42e3](https://github.com/wuwei-labs/antegen/commit/c1e42e3def473e55f0e77343ad0aef65dda0510a))

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
