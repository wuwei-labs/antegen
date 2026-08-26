# Changelog

## [6.0.0](https://github.com/wuwei-labs/antegen/compare/antegen-fiber-program-v5.2.2...antegen-fiber-program-v6.0.0) (2026-08-26)


### ⚠ BREAKING CHANGES

* **fiber:** programs that CPI directly into `fiber::close` must be rebuilt against the new IDL; instruction data without a trailing index no longer deserializes. The thread program's `close_fiber` and `thread_close` pass the index they already had, so deploying the thread program first is safe — Anchor ignores the trailing byte on the old fiber program.

### Features

* **fiber:** require fiber_index when closing a fiber ([92c8a87](https://github.com/wuwei-labs/antegen/commit/92c8a87a299f71bf061c326a513808a349c8cc7d))


### Bug Fixes

* **fiber:** close rent-sweep hole in create's update-in-place branch ([1aed242](https://github.com/wuwei-labs/antegen/commit/1aed2427c2a11b032cae082f1b73604da2c01c48))
* **fiber:** validate fiber PDA before update-in-place ([163d906](https://github.com/wuwei-labs/antegen/commit/163d9063a90a1b481f0eec2029fdbfc6f78c597a))

## [5.2.2](https://github.com/wuwei-labs/antegen/compare/antegen-fiber-program-v5.2.1...antegen-fiber-program-v5.2.2) (2026-08-25)


### Bug Fixes

* **fiber:** accept instruction data from callers built before lookup_tables ([891003d](https://github.com/wuwei-labs/antegen/commit/891003dc8ce79c46e4777143205ccc45666f12a2))
* **fiber:** accept instruction data from callers built before lookup_tables ([edb25ec](https://github.com/wuwei-labs/antegen/commit/edb25ecd006984cf903d555c60cde539681585af))

## [5.2.1](https://github.com/wuwei-labs/antegen/compare/antegen-fiber-program-v5.2.0...antegen-fiber-program-v5.2.1) (2026-08-23)


### Bug Fixes

* **thread:** accept instruction data from callers built before lookup_tables ([e39fec2](https://github.com/wuwei-labs/antegen/commit/e39fec21cef0114405297cc21625a6397ffc8b1a))
* **thread:** accept instruction data from callers built before lookup_tables ([3f1d252](https://github.com/wuwei-labs/antegen/commit/3f1d2528ad0489ba7d24aa984807df2bcf2cc42d))

## [5.2.0](https://github.com/wuwei-labs/antegen/compare/antegen-fiber-program-v5.1.0...antegen-fiber-program-v5.2.0) (2026-05-18)


### ⚠ BREAKING CHANGES

* **fiber:** `create` and `update` ix surfaces gain a required parameter; consumers must rebuild against the new IDL.

### Features

* **fiber:** add versioned Fiber state with lookup_tables ([6b1a7b0](https://github.com/wuwei-labs/antegen/commit/6b1a7b0c6e400c6aa790558e2cb780560af0d494))


### Bug Fixes

* **release:** revert phantom v6.0.0 release and force 5.2.0 minor ([07652fa](https://github.com/wuwei-labs/antegen/commit/07652faf34dfabba02d18e290042530fafc97366))
* **release:** roll thread/fiber manifest back to 5.1.0 to re-cut v6.0.0 ([9d5ee18](https://github.com/wuwei-labs/antegen/commit/9d5ee187d20c654a65d61fd4d7bd2783dcfb3b38))

## [5.1.0](https://github.com/wuwei-labs/antegen/compare/antegen-fiber-program-v5.0.7...antegen-fiber-program-v5.1.0) (2026-05-17)


### Features

* configurable program ID, verify command, and version bumps ([64c2e8d](https://github.com/wuwei-labs/antegen/commit/64c2e8d329bc8bc5d97bf5cdb75abbe6dd21998a))
* **fiber:** add init_if_needed pattern to fiber_create ([de2de0b](https://github.com/wuwei-labs/antegen/commit/de2de0bc4e7f1cdd441108bd5595322ed39a4a1d))
* **thread:** extract fiber into standalone program and add fiber_swap ([be56a6a](https://github.com/wuwei-labs/antegen/commit/be56a6ac75ab62deee70d52ce9fdb7bce8bffe68))


### Bug Fixes

* bypass fiber cache and improve executor diagnostics ([1cc950e](https://github.com/wuwei-labs/antegen/commit/1cc950e878ed6044ac8792fc4e7067a3f94ebdec))

## Changelog
