//! `MatrixClient` against a homeserver built from this crate's server side,
//! over real TCP through rama's `DefaultHttpWebClient`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rama::Layer;
use rama::extensions::Extension;
use rama::http::client::DefaultHttpWebClient;
use rama::http::layer::error_handling::ErrorHandlerLayer;
use rama::http::server::HttpServer;
use rama::http::service::web::Router;
use rama::rt::Executor;
use rama::tcp::server::TcpListener;
use rama_x_ruma::client::ClientError;
use rama_x_ruma::{
    Matrix, MatrixAuth, MatrixClient, MatrixRejection, MatrixResponse, MatrixResult,
    MatrixRouterExt, StateLayer, TokenIdentity,
};
use ruma::api::client::account::whoami::v3 as whoami;
use ruma::api::client::media::create_content::v3 as upload;
use ruma::api::client::message::send_message_event::v3 as send_message;
use ruma::api::client::sync::sync_events::v3 as sync;
use ruma::api::client::typing::create_typing_event::v3 as typing;
use ruma::api::error::ErrorCode;
use ruma::events::room::message::RoomMessageEventContent;
use ruma::{OwnedEventId, owned_device_id, owned_room_id, owned_user_id};

const TOKEN: &str = "secret";

#[derive(Debug, Default)]
struct Homeserver {
    // (room, txn id) -> event id: repeated txn ids answer the first event
    sent: Mutex<HashMap<(String, String), OwnedEventId>>,
}

impl Extension for Homeserver {}

impl MatrixAuth for Homeserver {
    async fn resolve_token(&self, token: &str) -> TokenIdentity {
        if token == TOKEN {
            TokenIdentity::User {
                user_id: owned_user_id!("@alice:example.org"),
                device_id: owned_device_id!("ALICEDEV"),
            }
        } else {
            TokenIdentity::Invalid { soft_logout: false }
        }
    }
}

async fn whoami(req: Matrix<whoami::Request, Homeserver>) -> MatrixResult<whoami::Response> {
    let user_id = req
        .sender_user
        .ok_or_else(|| MatrixRejection::internal_error("no user"))?;
    let mut response = whoami::Response::new(user_id, false);
    response.device_id = req.sender_device;
    Ok(MatrixResponse::new(response))
}

async fn send_message(
    req: Matrix<send_message::Request, Homeserver>,
) -> MatrixResult<send_message::Response> {
    let state = req.app_state().clone();
    let mut sent = state.sent.lock().expect("lock");
    let next = sent.len();
    let key = (req.body.room_id.to_string(), req.body.txn_id.to_string());
    let event_id = sent
        .entry(key)
        .or_insert_with(|| OwnedEventId::try_from(format!("$event{next}")).expect("valid event id"))
        .clone();
    Ok(MatrixResponse::new(send_message::Response::new(event_id)))
}

async fn upload(req: Matrix<upload::Request, Homeserver>) -> MatrixResult<upload::Response> {
    // the mxc id echoes what arrived, so the test sees the raw body survived
    let content_type = req.body.content_type.clone().unwrap_or_default();
    let name = req.body.filename.clone().unwrap_or_default();
    let uri = format!(
        "mxc://example.org/{}-{}-{}",
        req.body.file.len(),
        content_type.replace('/', "_"),
        name
    );
    Ok(MatrixResponse::new(upload::Response::new(uri.into())))
}

async fn typing(_req: Matrix<typing::Request, Homeserver>) -> MatrixResult<typing::Response> {
    Err(MatrixRejection::rate_limited(Some(1500)))
}

async fn sync(_req: Matrix<sync::Request, Homeserver>) -> MatrixResult<sync::Response> {
    // longer than any client timeout in these tests
    tokio::time::sleep(Duration::from_secs(30)).await;
    Ok(MatrixResponse::new(sync::Response::new("never".to_owned())))
}

/// Serves the test homeserver on an ephemeral port; returns its base URL.
async fn spawn_homeserver() -> String {
    let router = Router::new()
        .with_matrix_endpoint::<whoami::Request, _, _>(whoami)
        .with_matrix_endpoint::<send_message::Request, _, _>(send_message)
        .with_matrix_endpoint::<upload::Request, _, _>(upload)
        .with_matrix_endpoint::<typing::Request, _, _>(typing)
        .with_matrix_endpoint::<sync::Request, _, _>(sync);
    let service = Arc::new(
        StateLayer::new(Arc::new(Homeserver::default()))
            .into_layer(ErrorHandlerLayer::new().into_layer(router)),
    );
    let exec = Executor::default();
    let listener = TcpListener::bind_address("127.0.0.1:0", exec.clone())
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(listener.serve(HttpServer::auto(exec).service(service)));
    format!("http://{addr}/")
}

async fn client() -> MatrixClient<DefaultHttpWebClient> {
    MatrixClient::new(DefaultHttpWebClient::default(), spawn_homeserver().await)
        .with_access_token(TOKEN)
        .with_timeout(Duration::from_secs(10))
}

#[tokio::test]
async fn whoami_round_trips() {
    let response = client()
        .await
        .send(whoami::Request::new())
        .await
        .expect("whoami");
    assert_eq!(response.user_id, "@alice:example.org");
    assert_eq!(
        response.device_id.as_deref().map(|d| d.as_str()),
        Some("ALICEDEV")
    );
}

#[tokio::test]
async fn unknown_token_is_a_typed_matrix_error() {
    let client = client().await.with_access_token("wrong");
    let error = client
        .send(whoami::Request::new())
        .await
        .expect_err("rejected");
    assert_eq!(error.errcode(), Some(ErrorCode::UnknownToken));
    assert_eq!(error.status(), Some(http::StatusCode::UNAUTHORIZED));
    assert!(!error.is_retryable());
}

#[tokio::test]
async fn missing_token_fails_before_sending() {
    let client = client()
        .await
        .with_credentials(rama_x_ruma::Credentials::None);
    let error = client
        .send(whoami::Request::new())
        .await
        .expect_err("needs a token");
    assert!(matches!(error, ClientError::IntoHttp(_)), "{error:?}");
}

#[tokio::test]
async fn send_with_txn_id_is_idempotent() {
    let client = client().await;
    let content = RoomMessageEventContent::text_plain("hello");
    let request =
        send_message::Request::new(owned_room_id!("!room:example.org"), "txn1".into(), &content)
            .expect("serializes");

    let first = client.send(request.clone()).await.expect("first send");
    let again = client.send(request).await.expect("repeat send");
    assert_eq!(first.event_id, again.event_id);

    let other =
        send_message::Request::new(owned_room_id!("!room:example.org"), "txn2".into(), &content)
            .expect("serializes");
    let second = client.send(other).await.expect("second send");
    assert_ne!(first.event_id, second.event_id);
}

#[tokio::test]
async fn media_upload_sends_the_raw_body() {
    let mut request = upload::Request::new(vec![0xAB; 1024]);
    request.content_type = Some("image/png".to_owned());
    request.filename = Some("cat.png".to_owned());
    let response = client().await.send(request).await.expect("upload");
    assert_eq!(
        response.content_uri.as_str(),
        "mxc://example.org/1024-image_png-cat.png"
    );
}

#[tokio::test]
async fn rate_limit_exposes_retry_after() {
    let request = typing::Request::new(
        owned_user_id!("@alice:example.org"),
        owned_room_id!("!room:example.org"),
        typing::Typing::No,
    );
    let error = client().await.send(request).await.expect_err("limited");
    assert_eq!(error.errcode(), Some(ErrorCode::LimitExceeded));
    assert_eq!(error.status(), Some(http::StatusCode::TOO_MANY_REQUESTS));
    // the server rounds the Retry-After header up to whole seconds
    let retry_after = error.retry_after().expect("retry_after");
    assert!(
        retry_after >= Duration::from_millis(1500) && retry_after <= Duration::from_secs(2),
        "{retry_after:?}"
    );
    assert!(error.is_retryable());
}

#[tokio::test]
async fn per_request_timeout_bounds_long_polls() {
    let limit = Duration::from_millis(200);
    let error = client()
        .await
        .send_with_timeout(sync::Request::new(), limit)
        .await
        .expect_err("times out");
    assert!(
        matches!(error, ClientError::Timeout(l) if l == limit),
        "{error:?}"
    );
    assert!(error.is_retryable());
}

#[tokio::test]
async fn oversized_response_bodies_are_refused() {
    let client = client().await.with_max_response_body(8);
    let error = client
        .send(whoami::Request::new())
        .await
        .expect_err("too large");
    assert!(
        matches!(error, ClientError::BodyTooLarge { limit: 8 }),
        "{error:?}"
    );
}
