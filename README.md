# rama-x-ruma

Serve [ruma]'s typed Matrix API endpoints on [rama] HTTP, and send them
through a rama HTTP client with the `client` feature.

ruma describes every Matrix endpoint as types: the request (method, path
templates, authentication scheme, how to parse it from HTTP) and the
response (how to render it). `rama-x-ruma` plugs those types into rama's
web router, so a handler takes a parsed ruma request and returns a ruma
response, and every failure answers with a spec-shaped Matrix error.

- `Matrix<T, S>`: extractor for any ruma `IncomingRequest`. It feeds path
  parameters to ruma in template order, enforces the endpoint's
  authentication scheme, reads the body and calls `try_from_http_request`.
  An over-limit body is `M_TOO_LARGE`, bytes that are not JSON
  `M_NOT_JSON`, and JSON of the wrong shape `M_BAD_JSON`.
- `MatrixAuth`: the trait your state implements. It resolves access tokens
  to a user/device or an appservice, and optionally verifies X-Matrix
  signatures for federation endpoints.
- `MatrixResponse` / `MatrixResult`: render any ruma `OutgoingResponse`.
- `MatrixRejection`: Matrix errcodes with their HTTP status and JSON body
  (`soft_logout`, `retry_after_ms` plus `Retry-After`, UIA flows, ...).
  Implement `From<YourError> for MatrixRejection` and `?` just works.
- `MatrixRouterExt`: register a handler on every path ruma lists for an
  endpoint, with the method from ruma's metadata.
- `StateLayer`, `ExtState`, `RequireUser`: state and identity for routes
  without a ruma request type.
- `MatrixClient` (feature `client`): send any ruma `OutgoingRequest`
  through a rama HTTP client and get the typed response back.

## Example

A tiny server with one authenticated endpoint
(`examples/whoami.rs`, `cargo run -p rama-x-ruma --example whoami`):

```rust,no_run
use std::sync::Arc;

use rama::Layer;
use rama::extensions::Extension;
use rama::http::layer::error_handling::ErrorHandlerLayer;
use rama::http::server::HttpServer;
use rama::http::service::web::Router;
use rama::rt::Executor;
use rama_x_ruma::{
    Matrix, MatrixAuth, MatrixRejection, MatrixResponse, MatrixResult, MatrixRouterExt,
    StateLayer, TokenIdentity,
};
use ruma::api::client::account::whoami::v3 as whoami;
use ruma::{owned_device_id, owned_user_id};

#[derive(Debug)]
struct Server;

impl Extension for Server {}

impl MatrixAuth for Server {
    async fn resolve_token(&self, token: &str) -> TokenIdentity {
        // a real server looks the token up in its session store
        if token == "secret" {
            TokenIdentity::User {
                user_id: owned_user_id!("@alice:example.org"),
                device_id: owned_device_id!("ALICEDEV"),
            }
        } else {
            TokenIdentity::Invalid { soft_logout: false }
        }
    }
}

async fn whoami(req: Matrix<whoami::Request, Server>) -> MatrixResult<whoami::Response> {
    let user_id = req
        .sender_user
        .ok_or_else(|| MatrixRejection::internal_error("authenticated without a user"))?;
    let mut response = whoami::Response::new(user_id, false);
    response.device_id = req.sender_device;
    Ok(MatrixResponse::new(response))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let router = Router::new().with_matrix_endpoint::<whoami::Request, _, _>(whoami);
    let service = Arc::new(
        StateLayer::new(Arc::new(Server)).into_layer(ErrorHandlerLayer::new().into_layer(router)),
    );

    HttpServer::auto(Executor::default())
        .listen("127.0.0.1:8008", service)
        .await
}
```

```sh
$ curl -H 'Authorization: Bearer secret' http://127.0.0.1:8008/_matrix/client/v3/account/whoami
{"user_id":"@alice:example.org","device_id":"ALICEDEV"}
$ curl http://127.0.0.1:8008/_matrix/client/v3/account/whoami
{"errcode":"M_MISSING_TOKEN","error":"Missing access token."}
```

The example needs rama's `http-full` feature for `HttpServer`; the crate
itself only enables `http`, `http-hyperium` and `net`.

## Client

With the `client` feature, `MatrixClient` sends any ruma `OutgoingRequest`
through a rama `Service<Request>` and parses the endpoint's typed
`IncomingResponse`
(`examples/whoami_client.rs`, `cargo run -p rama-x-ruma --features client --example whoami_client`):

```rust,ignore
use std::time::Duration;

use rama::http::client::DefaultHttpWebClient;
use rama_x_ruma::MatrixClient;
use ruma::api::client::account::whoami::v3 as whoami;
use ruma::api::client::sync::sync_events::v3 as sync;

let client = MatrixClient::new(DefaultHttpWebClient::default(), "https://matrix.example.org")
    .with_access_token(token)
    .with_timeout(Duration::from_secs(30));

// picks /v3/ or /r0/ from the homeserver's /versions
let client = client.clone().with_supported_versions(client.fetch_supported_versions().await?);

let me = client.send(whoami::Request::new()).await?;

// a long poll needs more than the default timeout
let mut request = sync::Request::new();
request.timeout = Some(Duration::from_secs(30));
let batch = client.send_with_timeout(request, Duration::from_secs(60)).await?;
```

- The service is generic: wrap `DefaultHttpWebClient` (or your own
  connector stack) in rama layers for TLS, proxies, retries or tracing.
  HTTPS needs rama's `rustls` or `boring` feature in your build.
- The token is sent the way each endpoint's ruma auth scheme asks:
  `with_access_token` for users, `with_appservice_token` for appservices
  (also sent on appservice-optional endpoints such as `/login`). An
  endpoint that needs a token the client lacks fails before sending.
- Path versions come from `with_supported_versions`; the default is
  Matrix 1.1 (`/v3/` paths).
- Raw-body endpoints work as-is: `media::create_content::v3::Request`
  uploads its bytes with the `Content-Type` you set.
- Response bodies are read up to `with_max_response_body` (64 MiB by
  default); `with_timeout` bounds each request, `send_with_timeout` one call.
- Errors are `ClientError<E>`, where `E` is the endpoint's ruma error type
  (`ruma::api::error::Error`, or `UiaaResponse` behind user-interactive
  auth). `ClientError::Response { status, error }` keeps the answer's HTTP
  status (UIA challenges included) and ruma's `FromHttpResponseError`;
  `status()`, `errcode()`, `error_kind()` and `retry_after()` (from
  `Retry-After` or `retry_after_ms` on `M_LIMIT_EXCEEDED`) read it, and
  `is_retryable()` says whether trying again can help (transport errors,
  timeouts, rate limits, 5xx).

## ruma version

Built against ruma 0.17. `MatrixRejection::error_kind` returns ruma's
`ruma::api::error::ErrorKind` (ruma 0.15 merged the client error module
into `api::error`). Endpoints declared with ruma's `NoAccessToken` scheme
(login types, profiles, aliases, media downloads...) are served without
authentication, like
`NoAuthentication` ones.

## Features

- `client`: `MatrixClient` and `ClientError`. Enables ruma's
  `client-api-c` and tokio's timer; server-only builds compile neither.
- `unstable-msc4186`: `MatrixRejection::UnknownPos` reports ruma's
  `ErrorKind::UnknownPos` (simplified sliding sync); without it the kind is
  `ErrorKind::Unknown`. The errcode is `M_UNKNOWN_POS` either way.

## Origin

Extracted from the [dagma] Matrix homeserver, where it carries every
client, federation and admin endpoint.

## License

BSD-3-Clause, see [LICENSE](LICENSE).

[ruma]: https://crates.io/crates/ruma
[rama]: https://crates.io/crates/rama
[dagma]: https://dag.ma
