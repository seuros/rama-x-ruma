use super::*;

#[test]
fn test_convert_ruma_path_to_rama() {
    const CASES: &[(&str, &str)] = &[
        ("/_matrix/client/v3/login", "/_matrix/client/v3/login"),
        (
            "/_matrix/client/v3/rooms/{roomId}",
            "/_matrix/client/v3/rooms/{roomid}",
        ),
        (
            "/_matrix/client/v3/rooms/{roomId}/state/{eventType}/{stateKey}",
            "/_matrix/client/v3/rooms/{roomid}/state/{eventtype}/{statekey}",
        ),
        (
            "/_matrix/media/v3/download/{serverName}/{mediaId}",
            "/_matrix/media/v3/download/{servername}/{mediaid}",
        ),
        ("/path/{camelCaseVar}/end", "/path/{camelcasevar}/end"),
    ];
    for (input, expected) in CASES {
        assert_eq!(
            convert_ruma_path_to_rama(input),
            *expected,
            "input: {input:?}"
        );
    }
}

#[test]
fn test_endpoint_info_from_login_types() {
    use ruma::api::client::session::get_login_types;

    let infos = MatrixEndpointInfo::from_request::<get_login_types::v3::Request>();

    // Should have at least one path
    assert!(!infos.is_empty());

    // Check the primary path
    let primary = &infos[0];
    assert_eq!(primary.method, Method::GET);
    assert!(!primary.requires_auth); // Login types is a public endpoint
}

#[test]
fn test_endpoint_info_from_sync() {
    use ruma::api::client::sync::sync_events;

    let infos = MatrixEndpointInfo::from_request::<sync_events::v3::Request>();

    assert!(!infos.is_empty());

    let primary = &infos[0];
    assert_eq!(primary.method, Method::GET);
    assert!(primary.requires_auth); // Sync requires authentication
}

#[test]
fn test_endpoint_info_primary() {
    use ruma::api::client::session::get_login_types;

    let primary = MatrixEndpointInfo::primary::<get_login_types::v3::Request>();
    assert!(primary.is_some());

    let info = primary.unwrap();
    assert_eq!(info.method, Method::GET);
    assert!(info.path.contains("login"));
}

#[test]
fn trailing_state_key_also_routes_empty_key_spellings() {
    let cases: &[(&str, &[&str])] = &[
        (
            "/_matrix/client/v3/rooms/{room_id}/state/{event_type}/{state_key}",
            &[
                "/_matrix/client/v3/rooms/{room_id}/state/{event_type}/{state_key}",
                "/_matrix/client/v3/rooms/{room_id}/state/{event_type}",
                "/_matrix/client/v3/rooms/{room_id}/state/{event_type}/",
            ],
        ),
        (
            "/_matrix/client/v3/rooms/{room_id}/state",
            &["/_matrix/client/v3/rooms/{room_id}/state"],
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(route_paths(input), *expected, "input: {input:?}");
    }
}

/// A ruma endpoint registered on a rama router answers on every historical
/// path (or only the primary one), with the parsed response rendered.
#[tokio::test]
async fn ruma_endpoint_registers_on_a_rama_router() {
    use crate::{Matrix, MatrixAuth, MatrixResponse, MatrixResult, TokenIdentity};
    use rama::Service;
    use rama::http::{Body, BodyExtractExt, Request, StatusCode};
    use ruma::api::client::session::get_login_types::v3 as login_types;

    // MOCKED: no tokens resolve; the endpoint is unauthenticated anyway
    #[derive(Debug)]
    struct NoSessions;
    impl rama::extensions::Extension for NoSessions {}
    impl MatrixAuth for NoSessions {
        async fn resolve_token(&self, _token: &str) -> TokenIdentity {
            TokenIdentity::Invalid { soft_logout: false }
        }
    }

    async fn handler(
        _req: Matrix<login_types::Request, NoSessions>,
    ) -> MatrixResult<login_types::Response> {
        let flow = login_types::LoginType::Password(Default::default());
        Ok(MatrixResponse::new(login_types::Response::new(vec![flow])))
    }

    let all = Router::new().with_matrix_endpoint::<login_types::Request, _, _>(handler);
    let primary = Router::new().with_matrix_endpoint_primary::<login_types::Request, _, _>(handler);

    let request = |method: &str, path: &str| {
        Request::builder()
            .method(method)
            .uri(path)
            .body(Body::empty())
            .expect("request")
    };

    for path in ["/_matrix/client/v3/login", "/_matrix/client/r0/login"] {
        let response = all.serve(request("GET", path)).await.expect("routed");
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let body: serde_json::Value = response.try_into_json().await.expect("json");
        assert_eq!(body["flows"][0]["type"], "m.login.password", "{path}");
    }

    // the primary path is the oldest one in ruma's history
    let response = primary
        .serve(request("GET", "/_matrix/client/r0/login"))
        .await
        .expect("routed");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        primary
            .serve(request("GET", "/_matrix/client/v3/login"))
            .await
            .is_err(),
        "only the primary path is registered"
    );
    // the method comes from ruma's metadata
    assert!(
        all.serve(request("POST", "/_matrix/client/v3/login"))
            .await
            .is_err()
    );
}
