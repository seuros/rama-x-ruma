use std::sync::Mutex;

use super::*;
use rama::http::{Request, StatusCode};
use ruma::{owned_device_id, owned_server_name, owned_user_id};

// MOCKED: a fixed token table and a header check standing in for a real
// session store and X-Matrix signature verification.
#[derive(Debug, Default)]
struct MockAuth {
    authenticated: Mutex<Vec<OwnedUserId>>,
}

impl Extension for MockAuth {}

impl MatrixAuth for MockAuth {
    async fn resolve_token(&self, token: &str) -> TokenIdentity {
        // MOCKED: tokens are looked up in a literal table
        match token {
            "alice_token" => TokenIdentity::User {
                user_id: owned_user_id!("@alice:example.org"),
                device_id: owned_device_id!("ALICEDEV"),
            },
            "bridge_token" => TokenIdentity::Appservice {
                appservice_id: "bridge".to_owned(),
            },
            "stale_token" => TokenIdentity::Invalid { soft_logout: true },
            _ => TokenIdentity::Invalid { soft_logout: false },
        }
    }

    async fn verify_server_signatures(
        &self,
        parts: &RequestParts,
        body: &[u8],
    ) -> Result<OwnedServerName, MatrixRejection> {
        // MOCKED: any X-Matrix header from remote.test whose body is not
        // "tampered" verifies
        let signed = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("X-Matrix origin=\"remote.test\""));
        if signed && body != b"tampered" {
            Ok(owned_server_name!("remote.test"))
        } else {
            Err(MatrixRejection::unauthorized("bad signature"))
        }
    }

    fn user_authenticated(&self, user_id: &UserId) {
        if let Ok(mut seen) = self.authenticated.lock() {
            seen.push(user_id.to_owned());
        }
    }
}

/// Implements only what is required: no server signatures accepted.
#[derive(Debug)]
struct ClientOnly;

impl Extension for ClientOnly {}

impl MatrixAuth for ClientOnly {
    async fn resolve_token(&self, _token: &str) -> TokenIdentity {
        TokenIdentity::Invalid { soft_logout: false }
    }
}

fn parts(target: &str, authorization: Option<&str>) -> RequestParts {
    let mut builder = Request::builder().uri(target);
    if let Some(value) = authorization {
        builder = builder.header(AUTHORIZATION, value);
    }
    builder.body(()).expect("valid request").into_parts().0
}

#[test]
fn test_auth_context_default() {
    let ctx = AuthContext::default();
    assert!(ctx.sender_user.is_none());
    assert!(ctx.sender_device.is_none());
    assert!(ctx.sender_servername.is_none());
    assert!(!ctx.is_appservice);
    assert!(ctx.appservice_id.is_none());
}

#[test]
fn extract_access_token_prefers_header_then_query() {
    // (request target, Authorization header, expected token)
    const CASES: &[(&str, Option<&str>, Option<&str>)] = &[
        (
            "/_matrix/client/v3/sync",
            Some("Bearer test_token_123"),
            Some("test_token_123"),
        ),
        (
            "/_matrix/client/v3/sync?access_token=query_token_456",
            None,
            Some("query_token_456"),
        ),
        // the header wins over the query parameter
        (
            "/_matrix/client/v3/sync?access_token=query_token",
            Some("Bearer header_token"),
            Some("header_token"),
        ),
        ("/_matrix/client/v3/sync", None, None),
        // the query value is percent-decoded
        (
            "/_matrix/client/v3/sync?access_token=token%20with%20spaces",
            None,
            Some("token with spaces"),
        ),
        (
            "/_matrix/client/v3/sync?since=s1&access_token=second",
            None,
            Some("second"),
        ),
    ];

    for &(target, header, expected) in CASES {
        assert_eq!(
            extract_access_token(&parts(target, header)).as_deref(),
            expected,
            "target: {target:?}, header: {header:?}"
        );
    }
}

/// `(scheme, request target, Authorization header, body, expected)`.
type Case = (
    AuthScheme,
    &'static str,
    Option<&'static str>,
    &'static [u8],
    Outcome,
);

/// What `handle_auth` should produce.
#[derive(Debug)]
enum Outcome {
    Anonymous,
    User(&'static str),
    Appservice(&'static str, Option<&'static str>),
    Server(&'static str),
    Rejected(StatusCode, &'static str),
}

#[tokio::test]
async fn handle_auth_enforces_each_scheme() {
    use AuthScheme as S;
    use Outcome::*;

    const SYNC: &str = "/_matrix/client/v3/sync";
    const ALICE: Option<&str> = Some("Bearer alice_token");
    const BRIDGE: Option<&str> = Some("Bearer bridge_token");
    const SIGNED: Option<&str> = Some("X-Matrix origin=\"remote.test\",key=\"ed25519:1\"");

    let cases: &[Case] = &[
        // no token
        (S::None, SYNC, None, b"", Anonymous),
        (S::AccessTokenOptional, SYNC, None, b"", Anonymous),
        (S::AppserviceTokenOptional, SYNC, None, b"", Anonymous),
        (
            S::AccessToken,
            SYNC,
            None,
            b"",
            Rejected(StatusCode::UNAUTHORIZED, "M_MISSING_TOKEN"),
        ),
        (
            S::AppserviceToken,
            SYNC,
            None,
            b"",
            Rejected(StatusCode::UNAUTHORIZED, "M_MISSING_TOKEN"),
        ),
        // user tokens
        (S::AccessToken, SYNC, ALICE, b"", User("@alice:example.org")),
        (
            S::AccessTokenOptional,
            SYNC,
            ALICE,
            b"",
            User("@alice:example.org"),
        ),
        (S::None, SYNC, ALICE, b"", User("@alice:example.org")),
        (
            S::AppserviceToken,
            SYNC,
            ALICE,
            b"",
            Rejected(StatusCode::FORBIDDEN, "M_FORBIDDEN"),
        ),
        // server-signed endpoints never read tokens: a bearer is no signature
        (
            S::ServerSignatures,
            SYNC,
            ALICE,
            b"",
            Rejected(StatusCode::UNAUTHORIZED, "M_UNAUTHORIZED"),
        ),
        // invalid tokens, soft logout carried through
        (
            S::AccessToken,
            SYNC,
            Some("Bearer nope"),
            b"",
            Rejected(StatusCode::UNAUTHORIZED, "M_UNKNOWN_TOKEN"),
        ),
        (
            S::None,
            SYNC,
            Some("Bearer stale_token"),
            b"",
            Rejected(StatusCode::UNAUTHORIZED, "M_UNKNOWN_TOKEN"),
        ),
        // the OpenID userinfo endpoint's access_token is not an access token
        (
            S::None,
            "/_matrix/federation/v1/openid/userinfo?access_token=openid",
            None,
            b"",
            Anonymous,
        ),
        // appservices, optionally masquerading
        (
            S::AppserviceToken,
            SYNC,
            BRIDGE,
            b"",
            Appservice("bridge", None),
        ),
        (
            S::AccessToken,
            "/_matrix/client/v3/sync?user_id=%40bot%3Aexample.org",
            BRIDGE,
            b"",
            Appservice("bridge", Some("@bot:example.org")),
        ),
        (
            S::AccessToken,
            "/_matrix/client/v3/sync?user_id=not-a-user",
            BRIDGE,
            b"",
            Rejected(StatusCode::FORBIDDEN, "M_FORBIDDEN"),
        ),
        (S::None, SYNC, BRIDGE, b"", Appservice("bridge", None)),
        // server signatures
        (
            S::ServerSignatures,
            "/_matrix/federation/v1/version",
            SIGNED,
            b"{}",
            Server("remote.test"),
        ),
        (
            S::ServerSignatures,
            "/_matrix/federation/v1/version",
            SIGNED,
            b"tampered",
            Rejected(StatusCode::UNAUTHORIZED, "M_UNAUTHORIZED"),
        ),
        (
            S::ServerSignatures,
            "/_matrix/federation/v1/version",
            None,
            b"",
            Rejected(StatusCode::UNAUTHORIZED, "M_UNAUTHORIZED"),
        ),
    ];

    for (scheme, target, header, body, expected) in cases {
        let state = MockAuth::default();
        let label = format!("{scheme:?} {target} {header:?}");
        let result = handle_auth(&parts(target, *header), *scheme, &state, body).await;
        match (expected, result) {
            (Anonymous, Ok(ctx)) => {
                assert!(
                    ctx.sender_user.is_none() && ctx.sender_servername.is_none(),
                    "{label}"
                );
                assert!(!ctx.is_appservice, "{label}");
            }
            (User(user), Ok(ctx)) => {
                assert_eq!(
                    ctx.sender_user.as_deref().map(UserId::as_str),
                    Some(*user),
                    "{label}"
                );
                assert_eq!(
                    ctx.sender_device.as_deref().map(|d| d.as_str()),
                    Some("ALICEDEV")
                );
                let seen = state.authenticated.lock().expect("lock").clone();
                assert_eq!(seen.len(), 1, "{label}: user_authenticated runs once");
            }
            (Appservice(id, user), Ok(ctx)) => {
                assert!(ctx.is_appservice, "{label}");
                assert_eq!(ctx.appservice_id.as_deref(), Some(*id), "{label}");
                assert_eq!(
                    ctx.sender_user.as_deref().map(UserId::as_str),
                    *user,
                    "{label}"
                );
                assert!(ctx.sender_device.is_none(), "{label}");
            }
            (Server(origin), Ok(ctx)) => {
                assert_eq!(
                    ctx.sender_servername.as_deref().map(|s| s.as_str()),
                    Some(*origin)
                );
                assert!(ctx.sender_user.is_none(), "{label}");
            }
            (Rejected(status, errcode), Err(rejection)) => {
                assert_eq!(
                    (rejection.status(), rejection.errcode()),
                    (*status, *errcode),
                    "{label}"
                );
            }
            (expected, got) => panic!("{label}: expected {expected:?}, got {got:?}"),
        }
    }
}

#[tokio::test]
async fn soft_logout_reaches_the_rejection() {
    let state = MockAuth::default();
    let rejection = handle_auth(
        &parts("/_matrix/client/v3/sync", Some("Bearer stale_token")),
        AuthScheme::AccessToken,
        &state,
        b"",
    )
    .await
    .expect_err("stale token");
    assert!(matches!(
        rejection,
        MatrixRejection::UnknownToken { soft_logout: true }
    ));
}

#[tokio::test]
async fn server_signatures_are_refused_by_default() {
    let rejection = handle_auth(
        &parts(
            "/_matrix/federation/v1/version",
            Some("X-Matrix origin=\"remote.test\""),
        ),
        AuthScheme::ServerSignatures,
        &ClientOnly,
        b"",
    )
    .await
    .expect_err("default refuses");
    assert_eq!(rejection.errcode(), "M_UNAUTHORIZED");
}

#[tokio::test]
async fn identify_reports_absent_tokens_as_none() {
    let state = MockAuth::default();
    assert_eq!(identify(&parts("/x", None), &state).await, None);
    assert_eq!(
        identify(&parts("/x?access_token=bridge_token", None), &state).await,
        Some(TokenIdentity::Appservice {
            appservice_id: "bridge".to_owned()
        })
    );
}

#[test]
fn auth_scheme_is_read_from_ruma_metadata() {
    use ruma::api::client::{session::get_login_types, sync::sync_events};
    use ruma::api::federation::discovery::get_server_keys;

    assert_eq!(
        AuthScheme::of::<get_login_types::v3::Request>(),
        Some(AuthScheme::None)
    );
    assert_eq!(
        AuthScheme::of::<sync_events::v3::Request>(),
        Some(AuthScheme::AccessToken)
    );
    assert_eq!(
        AuthScheme::of::<get_server_keys::v2::Request>(),
        Some(AuthScheme::None)
    );
    assert_eq!(
        AuthScheme::of::<ruma::api::federation::transactions::send_transaction_message::v1::Request>(
        ),
        Some(AuthScheme::ServerSignatures)
    );
}

#[test]
fn auth_scheme_names_match_whole_never_as_substring() {
    use ruma::api::client::account::get_username_availability;

    // ruma's `NoAccessToken` (profile, aliases, media...) takes no token:
    // reading it as `AccessToken` would demand one on public endpoints.
    assert_eq!(
        AuthScheme::of::<get_username_availability::v3::Request>(),
        Some(AuthScheme::None)
    );
    assert_eq!(
        AuthScheme::from_type_name("ruma_common::api::auth_scheme::NoAccessToken"),
        Some(AuthScheme::None)
    );
    assert_eq!(
        AuthScheme::from_type_name("ruma_common::api::auth_scheme::AccessTokenOptional"),
        Some(AuthScheme::AccessTokenOptional)
    );
    assert_eq!(
        AuthScheme::from_type_name("ruma_common::api::auth_scheme::AppserviceTokenOptional"),
        Some(AuthScheme::AppserviceTokenOptional)
    );
    assert_eq!(
        AuthScheme::from_type_name("my_crate::SomeAccessTokenScheme"),
        None
    );
}
