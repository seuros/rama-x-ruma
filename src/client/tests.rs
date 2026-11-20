use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use http::StatusCode;

use ruma::api::EndpointError;
use ruma::api::auth_scheme::{AppserviceTokenOptional, SendAccessToken};
use ruma::api::client::account::whoami::v3 as whoami;
use ruma::api::client::uiaa::{UiaaInfo, UiaaResponse};
use ruma::api::error::{Error as RumaError, ErrorCode, FromHttpResponseError};

use super::*;

fn server_error(status: u16, headers: &[(&str, &str)], body: &str) -> ClientError {
    let mut builder = http::Response::builder().status(status);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let response = builder.body(body.as_bytes()).expect("valid response");
    ClientError::Response(FromHttpResponseError::Server(
        RumaError::from_http_response(response),
    ))
}

/// MOCKED: answers every request with one canned response and records
/// whether its Authorization header was marked sensitive.
#[derive(Clone)]
struct Canned {
    status: StatusCode,
    body: &'static str,
    authorization_sensitive: Arc<Mutex<Option<bool>>>,
}

impl Canned {
    fn new(status: StatusCode, body: &'static str) -> Self {
        Self {
            status,
            body,
            authorization_sensitive: Arc::default(),
        }
    }
}

impl Service<Request<Body>> for Canned {
    type Output = Response<Body>;
    type Error = Infallible;

    async fn serve(&self, request: Request<Body>) -> Result<Self::Output, Self::Error> {
        let sensitive = request
            .headers()
            .get(AUTHORIZATION)
            .map(|value| value.is_sensitive());
        *self.authorization_sensitive.lock().expect("lock") = sensitive;
        let mut response = Response::new(Body::from(self.body));
        *response.status_mut() =
            rama::http::StatusCode::from_u16(self.status.as_u16()).expect("valid status");
        Ok(response)
    }
}

#[test]
fn trailing_slashes_are_trimmed() {
    let client = MatrixClient::new((), "https://matrix.example.org//");
    assert_eq!(client.homeserver(), "https://matrix.example.org");
}

#[test]
fn credentials_debug_redacts_tokens() {
    let client = MatrixClient::new((), "https://hs").with_access_token("syt_secret");
    let debug = format!("{client:?}");
    assert!(!debug.contains("syt_secret"), "{debug}");
    assert!(debug.contains("AccessToken(<redacted>)"), "{debug}");
}

#[test]
fn credentials_map_to_send_access_token() {
    assert!(matches!(
        Credentials::None.send_access_token(),
        SendAccessToken::None
    ));
    assert!(matches!(
        Credentials::AccessToken("t".into()).send_access_token(),
        SendAccessToken::IfRequired("t")
    ));
    assert!(matches!(
        Credentials::Appservice("t".into()).send_access_token(),
        SendAccessToken::Appservice("t")
    ));
}

#[test]
fn user_token_skips_appservice_optional_endpoints() {
    // `/login` is AppserviceTokenOptional: a user token must not leak into it
    let user = Credentials::AccessToken("t".into());
    let input = <AppserviceTokenOptional as OutgoingAuth>::input(&user);
    assert_eq!(input.get_required_for_appservice(), None);
    let appservice = Credentials::Appservice("as".into());
    let input = <AppserviceTokenOptional as OutgoingAuth>::input(&appservice);
    assert_eq!(input.get_required_for_appservice(), Some("as"));
}

#[test]
fn builds_v3_path_with_bearer_token() {
    let client = MatrixClient::new((), "https://hs").with_access_token("tok");
    let request = client
        .build_request(whoami::Request::new())
        .expect("request builds");
    assert_eq!(
        request.uri().to_string(),
        "https://hs/_matrix/client/v3/account/whoami"
    );
    let authorization = &request.headers()[AUTHORIZATION];
    assert_eq!(authorization, "Bearer tok");
    // header-logging layers must not print the token
    assert!(authorization.is_sensitive());
}

#[tokio::test]
async fn the_service_receives_a_sensitive_authorization_header() {
    let service = Canned::new(
        StatusCode::OK,
        r#"{"user_id":"@alice:example.org","is_guest":false}"#,
    );
    let client = MatrixClient::new(service.clone(), "https://hs").with_access_token("tok");
    let me = client.send(whoami::Request::new()).await.expect("whoami");
    assert_eq!(me.user_id, "@alice:example.org");
    assert_eq!(
        *service.authorization_sensitive.lock().expect("lock"),
        Some(true)
    );
}

#[test]
fn retry_after_prefers_header_over_body() {
    let error = server_error(
        429,
        &[("retry-after", "7")],
        r#"{"errcode":"M_LIMIT_EXCEEDED","error":"slow down","retry_after_ms":2000}"#,
    );
    assert_eq!(error.errcode(), Some(ErrorCode::LimitExceeded));
    assert_eq!(error.retry_after(), Some(Duration::from_secs(7)));
    assert_eq!(error.status(), Some(http::StatusCode::TOO_MANY_REQUESTS));
    assert!(error.is_retryable());
}

#[test]
fn retry_after_falls_back_to_body() {
    let error = server_error(
        429,
        &[],
        r#"{"errcode":"M_LIMIT_EXCEEDED","error":"slow down","retry_after_ms":2000}"#,
    );
    assert_eq!(error.retry_after(), Some(Duration::from_millis(2000)));
}

#[test]
fn forbidden_is_not_retryable() {
    let error = server_error(403, &[], r#"{"errcode":"M_FORBIDDEN","error":"no"}"#);
    assert_eq!(error.errcode(), Some(ErrorCode::Forbidden));
    assert_eq!(error.retry_after(), None);
    assert!(!error.is_retryable());
}

#[test]
fn server_errors_and_timeouts_are_retryable() {
    let error = server_error(502, &[], "bad gateway");
    assert!(error.errcode().is_none());
    assert!(error.is_retryable());
    assert!(ClientError::<RumaError>::Timeout(Duration::from_secs(1)).is_retryable());
    assert!(!ClientError::<RumaError>::BodyTooLarge { limit: 1 }.is_retryable());
}

#[test]
fn uiaa_auth_response_has_no_matrix_error() {
    let error: ClientError<UiaaResponse> = ClientError::Response(FromHttpResponseError::Server(
        UiaaResponse::AuthResponse(UiaaInfo::new(Vec::new())),
    ));
    assert!(error.server_error().is_some());
    assert!(error.matrix_error().is_none());
    assert!(!error.is_retryable());
}
