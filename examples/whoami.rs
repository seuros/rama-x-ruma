//! A tiny Matrix server: one authenticated ruma endpoint on rama.
//!
//! ```sh
//! cargo run -p rama-x-ruma --example whoami
//! curl -H 'Authorization: Bearer secret' http://127.0.0.1:8008/_matrix/client/v3/account/whoami
//! curl http://127.0.0.1:8008/_matrix/client/v3/account/whoami   # 401 M_MISSING_TOKEN
//! ```

use std::sync::Arc;

use rama::Layer;
use rama::extensions::Extension;
use rama::http::layer::error_handling::ErrorHandlerLayer;
use rama::http::server::HttpServer;
use rama::http::service::web::Router;
use rama::rt::Executor;
use rama_x_ruma::{
    Matrix, MatrixAuth, MatrixRejection, MatrixResponse, MatrixResult, MatrixRouterExt, StateLayer,
    TokenIdentity,
};
use ruma::api::client::account::whoami::v3 as whoami;
use ruma::{owned_device_id, owned_user_id};

/// The server state every request can reach.
#[derive(Debug)]
struct Server;

impl Extension for Server {}

impl MatrixAuth for Server {
    async fn resolve_token(&self, token: &str) -> TokenIdentity {
        // MOCKED: one hard-coded session instead of a session store
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
    // the server clones its service per connection; share the router instead
    let service = Arc::new(
        StateLayer::new(Arc::new(Server)).into_layer(ErrorHandlerLayer::new().into_layer(router)),
    );

    HttpServer::auto(Executor::default())
        .listen("127.0.0.1:8008", service)
        .await
}
