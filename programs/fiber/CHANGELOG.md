# Changelog

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
