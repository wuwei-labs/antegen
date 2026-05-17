# Changelog

## [6.0.0](https://github.com/wuwei-labs/antegen/compare/antegen-thread-program-v5.1.0...antegen-thread-program-v6.0.0) (2026-05-17)


### ⚠ BREAKING CHANGES

* **thread:** `create_thread`, `create_fiber`, `update_fiber` instructions gain required `lookup_tables` parameters. The `fiber`, `target`, and `source` accounts on close / swap / exec are now untyped — consumers building the transaction by hand are unaffected, but anyone using Anchor's typed `Account` wrappers must update.

### Features

* **thread:** forward lookup_tables through fiber CPIs ([19eb27b](https://github.com/wuwei-labs/antegen/commit/19eb27ba1f2b6877104be916a87565cb74dc20b9))

## [5.1.0](https://github.com/wuwei-labs/antegen/compare/antegen-thread-program-v5.0.12...antegen-thread-program-v5.1.0) (2026-05-17)


### Features

* :sparkles: added last_exec_timestamp for threads ([a1640dc](https://github.com/wuwei-labs/antegen/commit/a1640dcd1db2bbe911b4a3733211906e0da72b39))
* :sparkles: added last_exec_timestamp for threads ([20fa4e7](https://github.com/wuwei-labs/antegen/commit/20fa4e74411f2c5a149b52dc27b60363cf01a2d4))
* :sparkles: adds cli thread close_to test ([f231e28](https://github.com/wuwei-labs/antegen/commit/f231e284006fe6214bb0fa49a985484d1a29616a))
* :sparkles: adds new CLI commands ([a1640dc](https://github.com/wuwei-labs/antegen/commit/a1640dcd1db2bbe911b4a3733211906e0da72b39))
* :sparkles: adds new CLI commands ([a619efc](https://github.com/wuwei-labs/antegen/commit/a619efc8ef2f00da47402aa23bc834234919662e))
* :sparkles: adds new CLI commands ([#11](https://github.com/wuwei-labs/antegen/issues/11)) ([dbbf170](https://github.com/wuwei-labs/antegen/commit/dbbf1708586d8907b9c5e9ede1e981f9cadc9232))
* :sparkles: adds new CLI commands ([#12](https://github.com/wuwei-labs/antegen/issues/12)) ([a1640dc](https://github.com/wuwei-labs/antegen/commit/a1640dcd1db2bbe911b4a3733211906e0da72b39))
* :sparkles: re-export fiber state through thread crate and improve executor batching ([7c6dcb0](https://github.com/wuwei-labs/antegen/commit/7c6dcb04e2b321782c26216360926f8a85b9c05c))
* :sparkles: solana-verify and updates to release workflow ([bb69201](https://github.com/wuwei-labs/antegen/commit/bb69201b9c931dc2e2ee24cb4e80f4590ead49b5))
* :sparkles: updated program IDs and README.md ([a77d91a](https://github.com/wuwei-labs/antegen/commit/a77d91a24bab719c2c3a503937261625bc49756d))
* :sparkles: updated program IDs and README.md ([#4](https://github.com/wuwei-labs/antegen/issues/4)) ([2545138](https://github.com/wuwei-labs/antegen/commit/25451383ca83ebad647a387b5d6e835c0a0654ec))
* :sparkles: v4.0.0 release ([ef5f7a9](https://github.com/wuwei-labs/antegen/commit/ef5f7a9877dbbce4b7a000cfd18a12650e7bc963))
* configurable program ID, verify command, and version bumps ([64c2e8d](https://github.com/wuwei-labs/antegen/commit/64c2e8d329bc8bc5d97bf5cdb75abbe6dd21998a))
* **fiber:** add init_if_needed pattern to fiber_create ([de2de0b](https://github.com/wuwei-labs/antegen/commit/de2de0bc4e7f1cdd441108bd5595322ed39a4a1d))
* **thread:** add index field to Signal::Update for cursor control ([1d9ac6c](https://github.com/wuwei-labs/antegen/commit/1d9ac6cf86e164743fb59dbe2da1bc9fc71a0869))
* **thread:** allow creating threads in paused state ([6ced5ac](https://github.com/wuwei-labs/antegen/commit/6ced5ac76984ce6a9aa51be5beb07f3f7db80250))
* **thread:** extract fiber into standalone program and add fiber_swap ([be56a6a](https://github.com/wuwei-labs/antegen/commit/be56a6ac75ab62deee70d52ce9fdb7bce8bffe68))
* **thread:** lazy fiber initialization via fiber_update with init_if_needed ([764c6d3](https://github.com/wuwei-labs/antegen/commit/764c6d39895fd815898800e9edd44c8ba21d3d49))
* **thread:** track fiber payer and return rent on close ([9b86f9c](https://github.com/wuwei-labs/antegen/commit/9b86f9cdea3e1abc861e2f16c481dc8d2f05050e))


### Bug Fixes

* :bug: added more localnet cli messages ([ffee110](https://github.com/wuwei-labs/antegen/commit/ffee110f751e8bffa43ebbace736271f775444cd))
* :bug: allow threadResponse trigger updates ([6b9ed58](https://github.com/wuwei-labs/antegen/commit/6b9ed58fd39f58c5aebef5c710e5a1eba5ce4801))
* :bug: fix thread create permissions ([b334a68](https://github.com/wuwei-labs/antegen/commit/b334a68670164d61b785602e9b5568a18f8fbb7a))
* :bug: mainnet fixes ([d444983](https://github.com/wuwei-labs/antegen/commit/d4449834c77e3f5dd3332fe36a20f72cc1f8bf5d))
* :bug: mainnet fixes ([#21](https://github.com/wuwei-labs/antegen/issues/21)) ([9202e20](https://github.com/wuwei-labs/antegen/commit/9202e20b1fc46a477128d829a791be6b38ed06a0))
* :bug: more mainnet issues ([fcb86c3](https://github.com/wuwei-labs/antegen/commit/fcb86c3190427c09796a08c5745162d3f2376bc8))
* :bug: updated to init_if_needed ([bd69f2a](https://github.com/wuwei-labs/antegen/commit/bd69f2a32bf99c1f997a983c4b18d5c81cf292fd))
* bypass fiber cache and improve executor diagnostics ([1cc950e](https://github.com/wuwei-labs/antegen/commit/1cc950e878ed6044ac8792fc4e7067a3f94ebdec))
* **close:** pass fiber PDAs and fiber program to close_fiber CPI ([31e2526](https://github.com/wuwei-labs/antegen/commit/31e252636c3d8b762c59f18057fd078d8eae386f))
* **thread:** allow cfg(target_os = "solana") for cargo publish ([f5e5477](https://github.com/wuwei-labs/antegen/commit/f5e5477b5a387bf54b2afd5dd870d15a8242609f))
* **thread:** don't auto-pause when signal explicitly sets paused: false ([a07e337](https://github.com/wuwei-labs/antegen/commit/a07e3377a6f4d27f672793cf84d53024c439f02c))
* **thread:** use unix_ts for Timestamp schedule.next instead of i64::MAX ([9b643c2](https://github.com/wuwei-labs/antegen/commit/9b643c2825afa029a0723d33c1bf081c3ed6ea14))

## Changelog
