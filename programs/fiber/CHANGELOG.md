# Changelog

## [6.0.0](https://github.com/wuwei-labs/antegen/compare/antegen-fiber-program-v5.1.0...antegen-fiber-program-v6.0.0) (2026-05-17)


### ⚠ BREAKING CHANGES

* **fiber:** `create` and `update` ix surfaces gain a required parameter; consumers must rebuild against the new IDL.

### Features

* **fiber:** add versioned Fiber state with lookup_tables ([6b1a7b0](https://github.com/wuwei-labs/antegen/commit/6b1a7b0c6e400c6aa790558e2cb780560af0d494))

## [5.1.0](https://github.com/wuwei-labs/antegen/compare/antegen-fiber-program-v5.0.7...antegen-fiber-program-v5.1.0) (2026-05-17)


### Features

* configurable program ID, verify command, and version bumps ([64c2e8d](https://github.com/wuwei-labs/antegen/commit/64c2e8d329bc8bc5d97bf5cdb75abbe6dd21998a))
* **fiber:** add init_if_needed pattern to fiber_create ([de2de0b](https://github.com/wuwei-labs/antegen/commit/de2de0bc4e7f1cdd441108bd5595322ed39a4a1d))
* **thread:** extract fiber into standalone program and add fiber_swap ([be56a6a](https://github.com/wuwei-labs/antegen/commit/be56a6ac75ab62deee70d52ce9fdb7bce8bffe68))


### Bug Fixes

* bypass fiber cache and improve executor diagnostics ([1cc950e](https://github.com/wuwei-labs/antegen/commit/1cc950e878ed6044ac8792fc4e7067a3f94ebdec))

## Changelog
