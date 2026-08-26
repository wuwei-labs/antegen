# Changelog

## [6.2.0](https://github.com/wuwei-labs/antegen/compare/antegen-client-v6.1.1...antegen-client-v6.2.0) (2026-08-26)


### Features

* **cli:** add `thread doctor` and drop the standalone recovery script ([22298a7](https://github.com/wuwei-labs/antegen/commit/22298a700d0fb182a410493a5d47d26a340fdffc))
* **cli:** add `thread doctor`, and the post-mortem that produced it ([a3bcea0](https://github.com/wuwei-labs/antegen/commit/a3bcea0e9718ec7351af4b033d4cd0f8cc8a4da7))

## [6.1.1](https://github.com/wuwei-labs/antegen/compare/antegen-client-v6.1.0...antegen-client-v6.1.1) (2026-08-26)


### Bug Fixes

* **client:** park threads whose fiber account is missing ([2316e25](https://github.com/wuwei-labs/antegen/commit/2316e25e80ac9c6f578d84c687ab385fbd405509))
* **fiber:** close rent-sweep hole in create's update-in-place branch ([1aed242](https://github.com/wuwei-labs/antegen/commit/1aed2427c2a11b032cae082f1b73604da2c01c48))

## [6.1.0](https://github.com/wuwei-labs/antegen/compare/antegen-client-v6.0.3...antegen-client-v6.1.0) (2026-08-25)


### Features

* **cli:** add `thread exec` for manually triggering a thread ([eed2a84](https://github.com/wuwei-labs/antegen/commit/eed2a842e1ee754b8d28b6cd1cb5c352a29d78c5))
* **client:** expose a shared thread_exec instruction builder ([031d408](https://github.com/wuwei-labs/antegen/commit/031d40806271068d2965f53e3c48ff169be77953))

## [6.0.3](https://github.com/wuwei-labs/antegen/compare/antegen-client-v6.0.2...antegen-client-v6.0.3) (2026-08-23)


### Bug Fixes

* **client:** park threads a program has rejected instead of retrying them ([eecfbae](https://github.com/wuwei-labs/antegen/commit/eecfbaefb4ded031864d52c64613101b64b144ba))
* **client:** refetch a thread after a program rejects it ([55823af](https://github.com/wuwei-labs/antegen/commit/55823af33c1168725ac8b55361c8dc0a4db5d622))
* **client:** refetch a thread after a program rejects it ([2b7bf9e](https://github.com/wuwei-labs/antegen/commit/2b7bf9e4c047e405fa3cfbadb1747df34da40131))

## [6.0.2](https://github.com/wuwei-labs/antegen/compare/antegen-client-v6.0.1...antegen-client-v6.0.2) (2026-08-22)


### Bug Fixes

* **client:** exit non-zero when the node stops on a failure ([45be73d](https://github.com/wuwei-labs/antegen/commit/45be73d35ff99a00c0057b322864dc7e15143ce0))
* **client:** redact the endpoint in ingest stats too ([87cd008](https://github.com/wuwei-labs/antegen/commit/87cd008ddf8ec7539713851762a774165f0c8697))
* **client:** stop a worker name collision from killing the node ([d0bafe3](https://github.com/wuwei-labs/antegen/commit/d0bafe3a89057a652fb06cbe67cde7ccbde8ced2))
* live version lookups, and stop the release pin drift for good ([5d0d630](https://github.com/wuwei-labs/antegen/commit/5d0d6308b21acf51db9bd2a5dbd402cde570eae1))

## [6.0.1](https://github.com/wuwei-labs/antegen/compare/antegen-client-v6.0.0...antegen-client-v6.0.1) (2026-08-16)


### Bug Fixes

* **client:** never let the observability agent stop the node ([e84d150](https://github.com/wuwei-labs/antegen/commit/e84d1504b527c279091eaad7a4d73f4916c507d3))
* **client:** redact endpoint credentials, recover the clock subscription, report latency ([77ccacf](https://github.com/wuwei-labs/antegen/commit/77ccacfd8263a56771564df51b3b18a606e142d1))
* keep the node alive, stop leaking credentials, and hold the release gate ([c1e42e3](https://github.com/wuwei-labs/antegen/commit/c1e42e3def473e55f0e77343ad0aef65dda0510a))

## [6.0.0](https://github.com/wuwei-labs/antegen/compare/antegen-client-v5.2.0...antegen-client-v6.0.0) (2026-08-16)


### ⚠ BREAKING CHANGES

* **cli:** `antegen-client` no longer builds the `antegen-node` binary and its `node` feature is removed; the library is unchanged. `antegen run` is now `antegen node run` and no longer takes `--version`, since it runs the calling binary rather than exec'ing another one.
* **client:** update loa-core to 3.0
* **client:** CompletionReason is removed. ExecutionResult exposes an `outcome: sched::Outcome` in place of its `success` and `skipped` flags, and is constructed via success/empty_fiber/superseded/lb_skip/retryable/fatal.
* **client:** ExecutorLogic::build_execute_transaction returns an additional Option<u64> of simulated compute units. RpcConfig and RpcPoolConfig each gain a skip_preflight field, which breaks struct-literal construction outside the crate; existing TOML is unaffected as the field defaults.
* **client:** several public signatures changed.
    - AccountUpdate and SharedResources each gain a public field, which breaks
      struct-literal construction outside the crate. Use AccountUpdate::new.
    - RpcPool::get_program_accounts now returns (slot, accounts).
    - RpcPool::get_signature_status now reports errors as String.
    - AccountCache::get_thread_or_fetch now returns FetchError, which
      distinguishes a genuinely absent account from a transport or decode
      failure.
    - RpcSubscription::new takes an IngestStats handle.
* **client:** swap pws for antegen-ws (rustls)

### Features

* **client:** instrument execution latency and harden datasource ingest ([d3d0a52](https://github.com/wuwei-labs/antegen/commit/d3d0a528c9034433d1eeadac04e5edf7fd788491))
* **client:** reconcile tracked threads and honour skippable ([ed5ae65](https://github.com/wuwei-labs/antegen/commit/ed5ae6566595e428dfb33bf0e2194e830c6aa820))
* **cli:** run the daemon from `antegen node run` ([2b3069c](https://github.com/wuwei-labs/antegen/commit/2b3069c399ddf8b6b1553d7e919d232c15a17616))


### Bug Fixes

* **client:** resolve stalls and silent failures found under localnet load ([09b924f](https://github.com/wuwei-labs/antegen/commit/09b924f3d48515ad5e2279077e9b6835f400e88e))


### Performance Improvements

* **client:** fire threads from a timer instead of the next clock message ([2e6e497](https://github.com/wuwei-labs/antegen/commit/2e6e497d103b23980e71b4fd588e2391bd98fa92))
* **client:** release the execution permit at submit and batch confirmation ([a72fe22](https://github.com/wuwei-labs/antegen/commit/a72fe22ddc28d925c59c45b1cfa30004af4996d9))
* **client:** remove redundant RPC round trips from the execution path ([2088b1b](https://github.com/wuwei-labs/antegen/commit/2088b1b195345ab8a29c7a6ac07a1ce9e8df6f64))


### Code Refactoring

* **client:** replace scheduling heaps with a due-time scheduler ([b4cba5e](https://github.com/wuwei-labs/antegen/commit/b4cba5e4f84df7cb50ab3a6712330ac8456513ac))
* **client:** swap pws for antegen-ws (rustls) ([60e8f42](https://github.com/wuwei-labs/antegen/commit/60e8f42ed6ab02adbd85116a2746e1617192b88e))


### Build System

* **client:** update loa-core to 3.0 ([b110f61](https://github.com/wuwei-labs/antegen/commit/b110f61d8e9595f991ca2ddb3fb1716d5516a609))

## [5.2.0](https://github.com/wuwei-labs/antegen/compare/antegen-client-v5.1.4...antegen-client-v5.2.0) (2026-05-17)


### Features

* :sparkles: re-export fiber state through thread crate and improve executor batching ([7c6dcb0](https://github.com/wuwei-labs/antegen/commit/7c6dcb04e2b321782c26216360926f8a85b9c05c))
* :sparkles: v4.0.0 release ([ef5f7a9](https://github.com/wuwei-labs/antegen/commit/ef5f7a9877dbbce4b7a000cfd18a12650e7bc963))
* **client:** add workspace claim to loa-core agent builder ([bb7ac7d](https://github.com/wuwei-labs/antegen/commit/bb7ac7d68625e93f8d7c44e94ba29e4bd8aa6e03))
* **cli:** v5.0.0 — node release pipeline, CLI version management, install script ([4486ad1](https://github.com/wuwei-labs/antegen/commit/4486ad18347f389a0ecbd06108ba84a766dbc11f))
* configurable program ID, verify command, and version bumps ([64c2e8d](https://github.com/wuwei-labs/antegen/commit/64c2e8d329bc8bc5d97bf5cdb75abbe6dd21998a))
* **thread:** add index field to Signal::Update for cursor control ([1d9ac6c](https://github.com/wuwei-labs/antegen/commit/1d9ac6cf86e164743fb59dbe2da1bc9fc71a0869))
* **thread:** extract fiber into standalone program and add fiber_swap ([be56a6a](https://github.com/wuwei-labs/antegen/commit/be56a6ac75ab62deee70d52ce9fdb7bce8bffe68))
* update dependencies and improve load balancer race handling ([7a5df3a](https://github.com/wuwei-labs/antegen/commit/7a5df3aa9686cfd566cb4f126d0f3b4c78543a46))


### Bug Fixes

* :bug: prepend CU limit to simulation for batched fiber execution ([662af0f](https://github.com/wuwei-labs/antegen/commit/662af0f5b6c61b4cd2884972be06d784e37630e3))
* :bug: prevent staging from cancelling worker during continuation batches ([2e332ab](https://github.com/wuwei-labs/antegen/commit/2e332ab07eba8a6c44352640f59609b62c03b62f))
* bypass fiber cache and improve executor diagnostics ([1cc950e](https://github.com/wuwei-labs/antegen/commit/1cc950e878ed6044ac8792fc4e7067a3f94ebdec))
* **client:** move antegen-node binary out of src/bin/ to avoid .gitignore conflict ([212f86a](https://github.com/wuwei-labs/antegen/commit/212f86aeb1b101f46ffa3a681746bb598ae5458a))
* **close:** pass fiber PDAs and fiber program to close_fiber CPI ([31e2526](https://github.com/wuwei-labs/antegen/commit/31e252636c3d8b762c59f18057fd078d8eae386f))
* **executor:** re-schedule thread after Signal::Update trigger change ([3af3f82](https://github.com/wuwei-labs/antegen/commit/3af3f82437c4515109ca5ef75033a55fcb88bf5a))
* late threads not executing after backfill ([093ce65](https://github.com/wuwei-labs/antegen/commit/093ce65e2989b5b08b52a8aa968c08b0e531867e))
* log fiber_cursor source at INFO level for stale cache diagnosis ([2be5c56](https://github.com/wuwei-labs/antegen/commit/2be5c56707f9a68a28869fc8ca9000f34a772918))
* move load balancer skip logs to debug level ([0ddc51e](https://github.com/wuwei-labs/antegen/commit/0ddc51e2c3bcfbcfe3b1f36ae22b9e874716e79e))
* re-queue threads skipped by load balancer for takeover retry ([42aa778](https://github.com/wuwei-labs/antegen/commit/42aa778a76e642feb063ae81d1ae897e6938611a))
* retry on TriggerConditionFailed (6004) instead of failing immediately ([d85cb22](https://github.com/wuwei-labs/antegen/commit/d85cb223124a9a86666b119d8ad75dac29541465))

## Changelog
