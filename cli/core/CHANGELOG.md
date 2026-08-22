# Changelog

## [9.0.0](https://github.com/wuwei-labs/antegen/compare/antegen-cli-core-v8.0.2...antegen-cli-core-v9.0.0) (2026-08-22)


### ⚠ BREAKING CHANGES

* **cli-core:** `antegen update`, `antegen list`, `antegen use`, `antegen install` and `antegen verify` are removed; re-run the install script to update the CLI, and use `antegen node <cmd>` for the daemon. The corresponding functions are removed from `antegen_cli_core::commands::update`.
* **cli:** `antegen-client` no longer builds the `antegen-node` binary and its `node` feature is removed; the library is unchanged. `antegen run` is now `antegen node run` and no longer takes `--version`, since it runs the calling binary rather than exec'ing another one.
* **client:** update loa-core to 3.0

### Features

* :sparkles: re-export fiber state through thread crate and improve executor batching ([7c6dcb0](https://github.com/wuwei-labs/antegen/commit/7c6dcb04e2b321782c26216360926f8a85b9c05c))
* **cli:** run the daemon from `antegen node run` ([2b3069c](https://github.com/wuwei-labs/antegen/commit/2b3069c399ddf8b6b1553d7e919d232c15a17616))


### Bug Fixes

* **cli-core:** clean the legacy layout on `antegen node start` too ([9bfccf1](https://github.com/wuwei-labs/antegen/commit/9bfccf16ef733b99dc8a9213c66e5c1982747adf))
* **cli-core:** do not answer version queries from the cache ([ccb7105](https://github.com/wuwei-labs/antegen/commit/ccb710569e973e5afe38e8d7ec25caedb06d26a6))
* **cli-core:** migrate installs off the antegen-node layout ([abd9f70](https://github.com/wuwei-labs/antegen/commit/abd9f70eab4bcfa7388fc5b863ca5099904dc83d))
* **cli-core:** resolve daemon releases from `antegen-cli-v` tags ([3b6ae3d](https://github.com/wuwei-labs/antegen/commit/3b6ae3df34be47d40e1d9a5f7c69c3f4126c8410))
* **client:** exit non-zero when the node stops on a failure ([45be73d](https://github.com/wuwei-labs/antegen/commit/45be73d35ff99a00c0057b322864dc7e15143ce0))
* **cli:** stop publishing antegen-cli and antegen-cli-core ([52663b5](https://github.com/wuwei-labs/antegen/commit/52663b575b157cbe0af66b6fc7d4a47cb8e47d6b))
* keep the node alive, stop leaking credentials, and hold the release gate ([c1e42e3](https://github.com/wuwei-labs/antegen/commit/c1e42e3def473e55f0e77343ad0aef65dda0510a))
* live version lookups, and stop the release pin drift for good ([5d0d630](https://github.com/wuwei-labs/antegen/commit/5d0d6308b21acf51db9bd2a5dbd402cde570eae1))


### Code Refactoring

* **cli-core:** delete CLI self-management ([2851ae7](https://github.com/wuwei-labs/antegen/commit/2851ae79ff3aaa019862bffbffed409a7e0f92f3))


### Build System

* **client:** update loa-core to 3.0 ([b110f61](https://github.com/wuwei-labs/antegen/commit/b110f61d8e9595f991ca2ddb3fb1716d5516a609))

## [8.0.2](https://github.com/wuwei-labs/antegen/compare/antegen-cli-core-v8.0.1...antegen-cli-core-v8.0.2) (2026-08-22)


### Bug Fixes

* **cli-core:** do not answer version queries from the cache ([ccb7105](https://github.com/wuwei-labs/antegen/commit/ccb710569e973e5afe38e8d7ec25caedb06d26a6))
* **client:** exit non-zero when the node stops on a failure ([45be73d](https://github.com/wuwei-labs/antegen/commit/45be73d35ff99a00c0057b322864dc7e15143ce0))
* live version lookups, and stop the release pin drift for good ([5d0d630](https://github.com/wuwei-labs/antegen/commit/5d0d6308b21acf51db9bd2a5dbd402cde570eae1))

## [8.0.1](https://github.com/wuwei-labs/antegen/compare/antegen-cli-core-v8.0.0...antegen-cli-core-v8.0.1) (2026-08-16)


### Bug Fixes

* **cli:** stop publishing antegen-cli and antegen-cli-core ([52663b5](https://github.com/wuwei-labs/antegen/commit/52663b575b157cbe0af66b6fc7d4a47cb8e47d6b))
* keep the node alive, stop leaking credentials, and hold the release gate ([c1e42e3](https://github.com/wuwei-labs/antegen/commit/c1e42e3def473e55f0e77343ad0aef65dda0510a))

## [8.0.0](https://github.com/wuwei-labs/antegen/compare/antegen-cli-core-v7.0.0...antegen-cli-core-v8.0.0) (2026-08-16)


### ⚠ BREAKING CHANGES

* **cli-core:** `antegen update`, `antegen list`, `antegen use`, `antegen install` and `antegen verify` are removed; re-run the install script to update the CLI, and use `antegen node <cmd>` for the daemon. The corresponding functions are removed from `antegen_cli_core::commands::update`.
* **cli:** `antegen-client` no longer builds the `antegen-node` binary and its `node` feature is removed; the library is unchanged. `antegen run` is now `antegen node run` and no longer takes `--version`, since it runs the calling binary rather than exec'ing another one.
* **client:** update loa-core to 3.0

### Features

* :sparkles: re-export fiber state through thread crate and improve executor batching ([7c6dcb0](https://github.com/wuwei-labs/antegen/commit/7c6dcb04e2b321782c26216360926f8a85b9c05c))
* **cli:** run the daemon from `antegen node run` ([2b3069c](https://github.com/wuwei-labs/antegen/commit/2b3069c399ddf8b6b1553d7e919d232c15a17616))


### Bug Fixes

* **cli-core:** clean the legacy layout on `antegen node start` too ([9bfccf1](https://github.com/wuwei-labs/antegen/commit/9bfccf16ef733b99dc8a9213c66e5c1982747adf))
* **cli-core:** migrate installs off the antegen-node layout ([abd9f70](https://github.com/wuwei-labs/antegen/commit/abd9f70eab4bcfa7388fc5b863ca5099904dc83d))
* **cli-core:** resolve daemon releases from `antegen-cli-v` tags ([3b6ae3d](https://github.com/wuwei-labs/antegen/commit/3b6ae3df34be47d40e1d9a5f7c69c3f4126c8410))


### Code Refactoring

* **cli-core:** delete CLI self-management ([2851ae7](https://github.com/wuwei-labs/antegen/commit/2851ae79ff3aaa019862bffbffed409a7e0f92f3))


### Build System

* **client:** update loa-core to 3.0 ([b110f61](https://github.com/wuwei-labs/antegen/commit/b110f61d8e9595f991ca2ddb3fb1716d5516a609))

## [7.0.0](https://github.com/wuwei-labs/antegen/compare/antegen-cli-core-v6.1.0...antegen-cli-core-v7.0.0) (2026-08-16)


### ⚠ BREAKING CHANGES

* **cli-core:** `antegen update`, `antegen list`, `antegen use`, `antegen install` and `antegen verify` are removed; re-run the install script to update the CLI, and use `antegen node <cmd>` for the daemon. The corresponding functions are removed from `antegen_cli_core::commands::update`.
* **cli:** `antegen-client` no longer builds the `antegen-node` binary and its `node` feature is removed; the library is unchanged. `antegen run` is now `antegen node run` and no longer takes `--version`, since it runs the calling binary rather than exec'ing another one.
* **client:** update loa-core to 3.0

### Features

* **cli:** run the daemon from `antegen node run` ([2b3069c](https://github.com/wuwei-labs/antegen/commit/2b3069c399ddf8b6b1553d7e919d232c15a17616))


### Bug Fixes

* **cli-core:** clean the legacy layout on `antegen node start` too ([9bfccf1](https://github.com/wuwei-labs/antegen/commit/9bfccf16ef733b99dc8a9213c66e5c1982747adf))
* **cli-core:** migrate installs off the antegen-node layout ([abd9f70](https://github.com/wuwei-labs/antegen/commit/abd9f70eab4bcfa7388fc5b863ca5099904dc83d))
* **cli-core:** resolve daemon releases from `antegen-cli-v` tags ([3b6ae3d](https://github.com/wuwei-labs/antegen/commit/3b6ae3df34be47d40e1d9a5f7c69c3f4126c8410))


### Code Refactoring

* **cli-core:** delete CLI self-management ([2851ae7](https://github.com/wuwei-labs/antegen/commit/2851ae79ff3aaa019862bffbffed409a7e0f92f3))


### Build System

* **client:** update loa-core to 3.0 ([b110f61](https://github.com/wuwei-labs/antegen/commit/b110f61d8e9595f991ca2ddb3fb1716d5516a609))

## [6.1.0](https://github.com/wuwei-labs/antegen/compare/antegen-cli-core-v6.0.0...antegen-cli-core-v6.1.0) (2026-05-17)


### Features

* :sparkles: re-export fiber state through thread crate and improve executor batching ([7c6dcb0](https://github.com/wuwei-labs/antegen/commit/7c6dcb04e2b321782c26216360926f8a85b9c05c))

## Changelog
