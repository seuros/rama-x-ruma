use super::*;

use rama::extensions::Extension;
use rama::http::service::web::Router;
use rama::http::service::web::extract::Path;
use rama::http::service::web::response::IntoResponse;
use rama::http::{Body, BodyExtractExt, Response, StatusCode};
use rama::{Layer, Service};
use ruma::api::client::session::get_login_types::v3 as login_types;
use ruma::api::client::state::get_state_event_for_key::v3 as state_event;
use ruma::{owned_device_id, owned_user_id};

use crate::{
    AuthScheme, MatrixAuth, MatrixResponse, MatrixResult, MatrixRouterExt, StateLayer,
    TokenIdentity,
};

// MOCKED: every bearer token is alice's session.
#[derive(Debug)]
struct Alice;

impl Extension for Alice {}

impl MatrixAuth for Alice {
    async fn resolve_token(&self, _token: &str) -> TokenIdentity {
        // MOCKED: no session store, one fixed user
        TokenIdentity::User {
            user_id: owned_user_id!("@alice:example.org"),
            device_id: owned_device_id!("ALICEDEV"),
        }
    }

    // a server may ask for more than ruma declares
    fn auth_scheme_for<T: 'static>(declared: AuthScheme) -> AuthScheme {
        if std::any::TypeId::of::<T>() == std::any::TypeId::of::<login_types::Request>() {
            AuthScheme::AccessTokenOptional
        } else {
            declared
        }
    }
}

fn login_types_matrix() -> Matrix<login_types::Request, Alice> {
    Matrix {
        body: login_types::Request::new(),
        sender_user: None,
        sender_device: None,
        sender_servername: None,
        sender_appservice: None,
        state: None,
        raw_query: None,
    }
}

async fn serve(
    svc: &impl Service<Request, Output = Response, Error: std::fmt::Debug>,
    method: &str,
    target: &str,
    token: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(target);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let request = builder.body(Body::empty()).expect("request");
    let response = svc
        .serve(request)
        .await
        .unwrap_or_else(|e| panic!("{method} {target}: {e:?}"));
    let status = response.status();
    let body = response
        .try_into_json()
        .await
        .unwrap_or(serde_json::Value::Null);
    (status, body)
}

#[test]
fn test_matrix_struct_deref() {
    let matrix = login_types_matrix();
    let _: &login_types::Request = &matrix;
}

#[test]
fn test_matrix_extractor_debug() {
    let debug_str = format!("{:?}", login_types_matrix());
    assert!(debug_str.contains("Matrix"));
}

#[tokio::test]
async fn oversized_body_is_too_large_not_bad_json() {
    use rama::http::body::util::Limited;
    use ruma::api::client::session::login::v3 as login;

    let cases = [
        (vec![b'x'; 64], 16, true),
        (b"not json".to_vec(), 1024, false),
    ];
    for (bytes, limit, too_large) in cases {
        let req = Request::builder()
            .method("POST")
            .uri("/_matrix/client/v3/login")
            .body(Body::new(Limited::new(Body::from(bytes), limit)))
            .expect("request");
        let Err(rejection) = Matrix::<login::Request, Alice>::from_request(req).await else {
            panic!("limit {limit}: request should be rejected");
        };
        assert_eq!(
            rejection.errcode() == "M_TOO_LARGE",
            too_large,
            "limit {limit}"
        );
    }
}

/// The three placeholders of a ruma path reach ruma in template order,
/// whatever their names, including the empty state key alias.
#[tokio::test]
async fn multi_segment_path_params_reach_ruma_in_template_order() {
    async fn handler(req: Matrix<state_event::Request, Alice>) -> impl IntoResponse {
        let body = serde_json::json!({
            "room_id": req.room_id,
            "event_type": req.event_type.to_string(),
            "state_key": req.state_key,
            "sender": req.sender_user,
        });
        rama::http::service::web::response::Json(body)
    }

    let router = Router::new().with_matrix_endpoint::<state_event::Request, _, _>(handler);
    let svc = StateLayer::new(std::sync::Arc::new(Alice)).into_layer(router);

    let cases = [
        (
            "/_matrix/client/v3/rooms/!room:example.org/state/m.room.member/@bob:example.org",
            "@bob:example.org",
        ),
        (
            "/_matrix/client/v3/rooms/!room:example.org/state/m.room.member",
            "",
        ),
    ];
    for (target, state_key) in cases {
        let (status, body) = serve(&svc, "GET", target, Some("t")).await;
        assert_eq!(status, StatusCode::OK, "{target}: {body}");
        assert_eq!(body["room_id"], "!room:example.org", "{target}");
        assert_eq!(body["event_type"], "m.room.member", "{target}");
        assert_eq!(body["state_key"], state_key, "{target}");
        assert_eq!(body["sender"], "@alice:example.org", "{target}");
    }

    // authentication runs before the handler
    let (status, body) = serve(&svc, "GET", cases[0].0, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["errcode"], "M_MISSING_TOKEN");
}

/// Routes without a ruma type take multi-segment paths as a named struct
/// (never a tuple, whose elements can bind out of order); a failed path
/// extraction is a Matrix error.
#[tokio::test]
async fn named_struct_path_params_bind_by_name() {
    #[derive(Debug, serde::Deserialize)]
    struct RoomStatePath {
        event_type: String,
        room_id: ruma::OwnedRoomId,
        state_key: String,
    }

    async fn handler(req: Request) -> Result<Response, MatrixRejection> {
        use rama::http::service::web::extract::FromPartsStateRefPair;
        let (parts, _) = req.into_parts();
        let Path(path) = Path::<RoomStatePath>::from_parts_state_ref_pair(&parts, &()).await?;
        let body = serde_json::json!({
            "room_id": path.room_id,
            "event_type": path.event_type,
            "state_key": path.state_key,
        });
        Ok(rama::http::service::web::response::Json(body).into_response())
    }

    let router = Router::new().with_get(
        "/_dagma/rooms/{room_id}/state/{event_type}/{state_key}",
        handler,
    );

    let (status, body) = serve(
        &router,
        "GET",
        "/_dagma/rooms/!r:example.org/state/m.room.topic/k",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["room_id"], "!r:example.org");
    assert_eq!(body["event_type"], "m.room.topic");
    assert_eq!(body["state_key"], "k");

    let (status, body) = serve(
        &router,
        "GET",
        "/_dagma/rooms/not-a-room/state/m.room.topic/k",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["errcode"], "M_MISSING_PARAM");
}

/// `auth_scheme_for` upgrades an unauthenticated endpoint to an optional
/// identity: anonymous still works, a token identifies the caller.
#[tokio::test]
async fn auth_scheme_override_identifies_optional_requesters() {
    async fn handler(
        req: Matrix<login_types::Request, Alice>,
    ) -> MatrixResult<login_types::Response> {
        let flows = if req.sender_user.is_some() { 1 } else { 0 };
        let flow = login_types::LoginType::Password(Default::default());
        Ok(MatrixResponse::new(login_types::Response::new(vec![
            flow;
            flows
        ])))
    }

    let router = Router::new().with_matrix_endpoint::<login_types::Request, _, _>(handler);
    let svc = StateLayer::new(std::sync::Arc::new(Alice)).into_layer(router);

    let (status, body) = serve(&svc, "GET", "/_matrix/client/v3/login", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["flows"].as_array().map(Vec::len), Some(0));

    let (status, body) = serve(&svc, "GET", "/_matrix/client/v3/login", Some("t")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["flows"].as_array().map(Vec::len), Some(1));
}

/// Authenticated endpoints need the state; without it they fail closed.
#[tokio::test]
async fn authenticated_endpoint_without_state_is_internal_error() {
    let req = Request::builder()
        .uri("/_matrix/client/v3/rooms/!r:x/state/m.room.name/")
        .header("authorization", "Bearer t")
        .body(Body::empty())
        .expect("request");
    let Err(rejection) = Matrix::<state_event::Request, Alice>::from_request(req).await else {
        panic!("no state, no auth");
    };
    assert_eq!(rejection.status(), StatusCode::INTERNAL_SERVER_ERROR);
}
