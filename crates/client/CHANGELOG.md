# Changelog

## [6.0.0](https://github.com/wuwei-labs/antegen/compare/antegen-client-v5.2.0...antegen-client-v6.0.0) (2026-05-18)


### ⚠ BREAKING CHANGES

* **client:** swap pws for antegen-ws (rustls)

### Code Refactoring

* **client:** swap pws for antegen-ws (rustls) ([60e8f42](https://github.com/wuwei-labs/antegen/commit/60e8f42ed6ab02adbd85116a2746e1617192b88e))

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
