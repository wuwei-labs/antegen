# Changelog

## [9.0.0](https://github.com/wuwei-labs/antegen/compare/antegen-cli-v8.0.0...antegen-cli-v9.0.0) (2026-08-22)


### ⚠ BREAKING CHANGES

* **cli:** the `antegen-cli-core` crate is removed; its contents are modules of `antegen-cli`. Neither crate was published, so this affects only in-repo paths.

### Features

* :sparkles: v4.0.0 release ([ef5f7a9](https://github.com/wuwei-labs/antegen/commit/ef5f7a9877dbbce4b7a000cfd18a12650e7bc963))
* add install command for self-installing binary ([3d65da6](https://github.com/wuwei-labs/antegen/commit/3d65da6540afb7729906471d8eafee4429eea8d8))
* add service management and self-update commands ([17e96d8](https://github.com/wuwei-labs/antegen/commit/17e96d85c8d61d33340349fdefcd12b0def93774))
* **cli:** add admin thread delete command (dev feature only) ([479109c](https://github.com/wuwei-labs/antegen/commit/479109cae4e3c0e903c4e01e8a2f5fa8fbda148b))
* **cli:** add config get/set commands, decouple crate versions ([bcca911](https://github.com/wuwei-labs/antegen/commit/bcca9114c456b5c952dba6c776c06a8deb0fe155))
* **cli:** add info command, version flags, and improved update flow ([230ae10](https://github.com/wuwei-labs/antegen/commit/230ae102a6b3ed29e84effa1007160385a1ac567))
* **cli:** add logs command, symlink updates, and RPC override ([d0340ff](https://github.com/wuwei-labs/antegen/commit/d0340ff1ba9a7c86355d5bd739fc49635a3c58c9))
* **cli:** integrate self_update, fix Linux issues, add PATH setup ([e1867ab](https://github.com/wuwei-labs/antegen/commit/e1867aba99927453152410081e552ae17f0b48ff))
* **cli:** v5.0.0 — node release pipeline, CLI version management, install script ([4486ad1](https://github.com/wuwei-labs/antegen/commit/4486ad18347f389a0ecbd06108ba84a766dbc11f))
* **cli:** v5.0.0 — reorder help commands, update references to anm ([508e893](https://github.com/wuwei-labs/antegen/commit/508e8931863662a24330573fd738a10837e2ceb4))
* configurable program ID, verify command, and version bumps ([64c2e8d](https://github.com/wuwei-labs/antegen/commit/64c2e8d329bc8bc5d97bf5cdb75abbe6dd21998a))
* enhance config init with CLI flags and auto-permissions ([01f8cef](https://github.com/wuwei-labs/antegen/commit/01f8cefdade3537fe060bdec776279e4e41bc536))
* **fiber:** add init_if_needed pattern to fiber_create ([de2de0b](https://github.com/wuwei-labs/antegen/commit/de2de0bc4e7f1cdd441108bd5595322ed39a4a1d))
* **thread:** extract fiber into standalone program and add fiber_swap ([be56a6a](https://github.com/wuwei-labs/antegen/commit/be56a6ac75ab62deee70d52ce9fdb7bce8bffe68))
* update dependencies and improve load balancer race handling ([7a5df3a](https://github.com/wuwei-labs/antegen/commit/7a5df3aa9686cfd566cb4f126d0f3b4c78543a46))


### Bug Fixes

* CLI strips quotes from string inputs for user-friendliness ([5bfdf4d](https://github.com/wuwei-labs/antegen/commit/5bfdf4db4f11599c3c963489c9e1aeea968ad24d))
* **cli:** add cfg attribute to macOS-only get_log_path function ([0adc760](https://github.com/wuwei-labs/antegen/commit/0adc760a2abc43000f9da36de8b285b43936ff62))
* **cli:** fix install script and config init default path ([566f925](https://github.com/wuwei-labs/antegen/commit/566f925d0f388cffb77e565e343e7f7b338989d5))
* **cli:** graceful handling when no node release exists yet ([2987f87](https://github.com/wuwei-labs/antegen/commit/2987f8782dd06882d9c7761068231df622ca553e))
* **cli:** remove invalid .context() calls on SystemdServiceManager ([9a51d93](https://github.com/wuwei-labs/antegen/commit/9a51d93aeb309b9e4b1c6df863898d0225ddb945))
* **cli:** support cargo-installed binary in version switching and list ([c849123](https://github.com/wuwei-labs/antegen/commit/c849123f1d3af2270727c8e8bf46f03e69ddf204))
* **cli:** update AgentInfo::read call for loa-core 2.0.0 API ([9b231b6](https://github.com/wuwei-labs/antegen/commit/9b231b6cd9e246859d7aec753bc93cf12e1f51a7))
* **cli:** wire global --rpc flag through to `config set` command ([7d8233b](https://github.com/wuwei-labs/antegen/commit/7d8233bdb9cb8732a8f2b4fb012f573457a07f6b))
* generate keypair during config init and add polling fallback ([4f62fc6](https://github.com/wuwei-labs/antegen/commit/4f62fc604543547cf08b7fcc1c4bdb2049def710))
* late threads not executing after backfill ([093ce65](https://github.com/wuwei-labs/antegen/commit/093ce65e2989b5b08b52a8aa968c08b0e531867e))
* require --rpc flag in non-interactive mode ([e83f367](https://github.com/wuwei-labs/antegen/commit/e83f367b7895cf3350551ecdf1065f09cfaddebb))
* use bash instead of sh in install script references ([30aad94](https://github.com/wuwei-labs/antegen/commit/30aad941b1027afa936374b0302bfdbe696905f1))
* use symlink-based updates with automatic rollback ([80b2a9a](https://github.com/wuwei-labs/antegen/commit/80b2a9acde5c6c89c8ebd21d0e0564c3a0c02391))


### Code Refactoring

* **cli:** merge cli-core into the CLI and move it to crates/cli ([c3b8db2](https://github.com/wuwei-labs/antegen/commit/c3b8db2af98a84077fcb17953e1f439634ce1219))

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
