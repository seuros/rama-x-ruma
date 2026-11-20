# Changelog

## 0.2.0 (2026-11-20)

### Features

* **client:** `MatrixClient` sends any ruma `OutgoingRequest` through a rama
  `Service<Request>` and parses the typed `IncomingResponse`, behind the new
  `client` feature. Tokens follow each endpoint's auth scheme (user or
  appservice `Credentials`), paths follow the homeserver's
  `SupportedVersions`, response bodies are read under a size limit, and
  requests take a default or per-call timeout.
* **client:** `ClientError<E>` keeps ruma's `FromHttpResponseError` and reads
  `errcode()`, `error_kind()`, `status()`, `retry_after()` and
  `is_retryable()` from it, for ruma's `Error` and `UiaaResponse`.
* **examples:** `whoami_client`.

### Bug Fixes

* **docs:** the `MatrixRejection` link on `MatrixResult` resolves.

## 0.1.0 (2026-11-05)

### Features

* ruma endpoints on rama HTTP: the `Matrix` extractor, `MatrixResponse`,
  `MatrixRejection`, `MatrixAuth`, `MatrixRouterExt`, `StateLayer`,
  `ExtState` and `RequireUser`.
