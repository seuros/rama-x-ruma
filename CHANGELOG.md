# Changelog

## 0.2.0 (2026-11-20)

### Features

* **client:** `MatrixClient` sends any ruma `OutgoingRequest` through a rama
  `Service<Request>` and parses the typed `IncomingResponse`, behind the new
  `client` feature. Tokens follow each endpoint's auth scheme (user or
  appservice `Credentials`), paths follow the homeserver's
  `SupportedVersions`, response bodies are read under a size limit, and
  requests take a default or per-call timeout. The `Authorization` header
  is marked sensitive so header-logging layers never print the token.
* **client:** `ClientError<E>` keeps the answer's HTTP status and ruma's
  `FromHttpResponseError`, and reads `status()`, `errcode()`, `error_kind()`,
  `retry_after()` and `is_retryable()` from them, for ruma's `Error` and
  `UiaaResponse` (a UIA challenge still reports its 401).
* **examples:** `whoami_client`.

### ⚠ BREAKING CHANGES

* **rejection:** a request body that does not parse as JSON, or is not
  UTF-8 anywhere (even inside a field ruma ignores), now answers
  `M_NOT_JSON`, as the spec requires; `M_BAD_JSON` stays for JSON of the
  wrong shape. Raw-body endpoints such as media upload still take any bytes. `From<serde_json::Error>` follows the same
  split, and `MatrixRejection` has a `NotJson` variant.
* **router:** `route_paths` returns two paths for a trailing state key, the
  second ending in `/?`, instead of three.

### Bug Fixes

* **router:** `PUT`/`GET .../state/{eventType}/`, the spec's spelling of the
  empty state key, routes instead of answering 404 (rama trims a literal
  trailing slash off route patterns).
* **docs:** the `MatrixRejection` link on `MatrixResult` resolves.

## 0.1.0 (2026-11-05)

### Features

* ruma endpoints on rama HTTP: the `Matrix` extractor, `MatrixResponse`,
  `MatrixRejection`, `MatrixAuth`, `MatrixRouterExt`, `StateLayer`,
  `ExtState` and `RequireUser`.
