# rama-x-ruma

Serve [ruma]'s typed Matrix API endpoints on [rama] HTTP.

ruma describes every Matrix endpoint as types: the request (method, path
templates, authentication scheme, how to parse it from HTTP) and the
response (how to render it). `rama-x-ruma` plugs those types into rama's
web router, so a handler takes a parsed ruma request and returns a ruma
response, and every failure answers with a spec-shaped Matrix error.

- `Matrix<T, S>`: extractor for any ruma `IncomingRequest`. It feeds path
  parameters to ruma in template order, enforces the endpoint's
  authentication scheme, reads the body (an over-limit body is
  `M_TOO_LARGE`, not `M_BAD_JSON`) and calls `try_from_http_request`.
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

## ruma version

Built against ruma 0.17. `MatrixRejection::error_kind` returns ruma's
`ruma::api::error::ErrorKind` (ruma 0.15 merged the client error module
into `api::error`). Endpoints declared with ruma's `NoAccessToken` scheme
(login types, profiles, aliases, media downloads...) are served without
authentication, like
`NoAuthentication` ones.

## Features

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
