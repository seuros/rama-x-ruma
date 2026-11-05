use super::*;
use std::assert_matches;

#[test]
fn test_missing_token() {
    let rejection = MatrixRejection::missing_token();
    assert_eq!(rejection.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(rejection.errcode(), "M_MISSING_TOKEN");
}

#[test]
fn test_invalid_token() {
    let rejection = MatrixRejection::invalid_token();
    assert_eq!(rejection.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(rejection.errcode(), "M_UNKNOWN_TOKEN");
}

#[test]
fn test_unknown_token_soft_logout() {
    let rejection = MatrixRejection::UnknownToken { soft_logout: true };
    let body_json: serde_json::Value = serde_json::from_slice(&rejection.to_json_bytes()).unwrap();

    assert_eq!(body_json["errcode"], "M_UNKNOWN_TOKEN");
    assert_eq!(body_json["soft_logout"], true);
}

#[test]
fn test_missing_path_param() {
    let rejection = MatrixRejection::missing_path_param("roomId");
    assert_eq!(rejection.status(), StatusCode::BAD_REQUEST);
    assert_eq!(rejection.errcode(), "M_MISSING_PARAM");
    assert!(rejection.message().contains("roomId"));
}

#[test]
fn test_rate_limited_with_retry() {
    let rejection = MatrixRejection::rate_limited(Some(5000));
    let body_json: serde_json::Value = serde_json::from_slice(&rejection.to_json_bytes()).unwrap();

    assert_eq!(body_json["errcode"], "M_LIMIT_EXCEEDED");
    assert_eq!(body_json["retry_after_ms"], 5000);
}

/// Every 429 telling the client when to retry says so in the body, the
/// `Retry-After` header (whole seconds, rounded up) and the ruma kind.
#[test]
fn limit_exceeded_sets_retry_after_everywhere() {
    use rama::http::header::RETRY_AFTER;

    let cases: [(MatrixRejection, Option<(u64, &str)>); 4] = [
        (MatrixRejection::rate_limited(Some(5000)), Some((5000, "5"))),
        (MatrixRejection::rate_limited(Some(1)), Some((1, "1"))),
        (MatrixRejection::rate_limited(Some(2001)), Some((2001, "3"))),
        (MatrixRejection::rate_limited(None), None),
    ];
    for (rejection, expected) in cases {
        let label = format!("{rejection:?}");
        let expected_kind =
            expected.map(|(ms, _)| RetryAfter::Delay(std::time::Duration::from_millis(ms)));
        assert_matches!(
            rejection.error_kind(),
            ErrorKind::LimitExceeded(data) if data.retry_after == expected_kind,
            "{label}"
        );
        let body: serde_json::Value =
            serde_json::from_slice(&rejection.to_json_bytes()).expect("json");
        assert_eq!(
            body.get("retry_after_ms")
                .and_then(serde_json::Value::as_u64),
            expected.map(|(ms, _)| ms),
            "{label}"
        );
        let response = rejection.into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS, "{label}");
        assert_eq!(
            response
                .headers()
                .get(RETRY_AFTER)
                .map(|value| value.to_str().expect("ascii")),
            expected.map(|(_, secs)| secs),
            "{label}"
        );
    }
}

#[test]
fn test_forbidden() {
    let rejection = MatrixRejection::forbidden("Access denied");
    assert_eq!(rejection.status(), StatusCode::FORBIDDEN);
    assert_eq!(rejection.errcode(), "M_FORBIDDEN");
    assert_eq!(rejection.message(), "Access denied");
}

#[test]
fn test_display() {
    let rejection = MatrixRejection::forbidden("Access denied");
    let display = format!("{}", rejection);
    assert!(display.contains("403"));
    assert!(display.contains("M_FORBIDDEN"));
    assert!(display.contains("Access denied"));
}

#[test]
fn test_internal_error_hides_details() {
    let rejection = MatrixRejection::Internal("secret database password".to_owned());
    // The message() method should NOT expose internal details
    assert!(!rejection.message().contains("secret"));
    assert_eq!(rejection.message(), "Internal server error.");
}

#[test]
fn test_json_serialization() {
    let rejection = MatrixRejection::InvalidJson("expected '}' at line 1".to_owned());
    let body_json: serde_json::Value = serde_json::from_slice(&rejection.to_json_bytes()).unwrap();

    assert_eq!(body_json["errcode"], "M_BAD_JSON");
    assert!(
        body_json["error"]
            .as_str()
            .unwrap()
            .contains("Invalid JSON")
    );
    // soft_logout should not be present
    assert!(body_json.get("soft_logout").is_none());
}

#[test]
fn parameter_and_routing_rejections() {
    let cases = [
        (
            MatrixRejection::missing_param("limit"),
            StatusCode::BAD_REQUEST,
            "M_MISSING_PARAM",
        ),
        (
            MatrixRejection::invalid_param("bad width"),
            StatusCode::BAD_REQUEST,
            "M_INVALID_PARAM",
        ),
        (
            MatrixRejection::unrecognized("unknown query type"),
            StatusCode::NOT_FOUND,
            "M_UNRECOGNIZED",
        ),
        (
            MatrixRejection::IncompatibleRoomVersion {
                room_version: "10".to_owned(),
            },
            StatusCode::BAD_REQUEST,
            "M_INCOMPATIBLE_ROOM_VERSION",
        ),
    ];
    for (rejection, status, errcode) in cases {
        assert_eq!(rejection.status(), status, "{rejection}");
        assert_eq!(rejection.errcode(), errcode, "{rejection}");
        let body: serde_json::Value =
            serde_json::from_slice(&rejection.to_json_bytes()).expect("json");
        assert_eq!(body["errcode"], errcode);
    }
}

#[test]
fn incompatible_room_version_names_the_version() {
    let rejection = MatrixRejection::IncompatibleRoomVersion {
        room_version: "11".to_owned(),
    };
    let body: serde_json::Value = serde_json::from_slice(&rejection.to_json_bytes()).expect("json");
    assert_eq!(body["room_version"], "11");
    assert!(rejection.message().contains("11"));
}

#[test]
fn query_errors_name_the_parameter() {
    let cases = [
        ("missing field `dir`", "M_MISSING_PARAM", "dir"),
        ("missing field `ts`", "M_MISSING_PARAM", "ts"),
        (
            "invalid digit found in string",
            "M_INVALID_PARAM",
            "invalid digit",
        ),
        (
            "unknown variant `x`, expected `f` or `b`",
            "M_INVALID_PARAM",
            "unknown variant",
        ),
    ];
    for (message, errcode, mentions) in cases {
        let rejection = MatrixRejection::from_query_error(message);
        assert_eq!(rejection.errcode(), errcode, "{message}");
        assert_eq!(rejection.status(), StatusCode::BAD_REQUEST, "{message}");
        assert!(
            rejection.message().contains(mentions),
            "{message}: {rejection}"
        );
    }
}

/// Query strings rejected by the generic query extractor are bad
/// parameters (`M_INVALID_PARAM` / `M_MISSING_PARAM`), never `M_BAD_JSON`.
#[test]
fn query_extractor_rejections_are_param_errors() {
    use rama::http::service::web::extract::Query;

    #[derive(Debug, serde::Deserialize)]
    struct Params {
        #[allow(dead_code, reason = "only decoded to provoke a rejection")]
        limit: u32,
    }

    let cases = [
        ("limit=abc", "M_INVALID_PARAM"),
        ("limit=-1", "M_INVALID_PARAM"),
        ("", "M_MISSING_PARAM"),
    ];
    for (query, errcode) in cases {
        let err = Query::<Params>::parse_query_str(query).expect_err(query);
        let rejection = MatrixRejection::from(err);
        assert_eq!(
            (rejection.status(), rejection.errcode()),
            (StatusCode::BAD_REQUEST, errcode),
            "query: {query:?}, rejection: {rejection}"
        );
    }
}

/// Every variant: HTTP status, errcode, and the body `into_response`
/// actually sends (`error` plus any variant-specific field).
#[tokio::test]
async fn rejection_mapping_table() {
    use rama::http::body::util::BodyExt;
    use serde_json::{Value, json};

    let uiaa = || {
        let mut info = UiaaInfo::new(vec![]);
        info.session = Some("s1".to_owned());
        info
    };
    let cases: Vec<(MatrixRejection, StatusCode, &str, Value)> = vec![
        (
            MatrixRejection::MissingToken,
            StatusCode::UNAUTHORIZED,
            "M_MISSING_TOKEN",
            json!({"error": "Missing access token."}),
        ),
        (
            MatrixRejection::InvalidToken,
            StatusCode::UNAUTHORIZED,
            "M_UNKNOWN_TOKEN",
            json!({"error": "Invalid access token."}),
        ),
        (
            MatrixRejection::UnknownToken { soft_logout: false },
            StatusCode::UNAUTHORIZED,
            "M_UNKNOWN_TOKEN",
            json!({"soft_logout": false}),
        ),
        (
            MatrixRejection::Forbidden("no".into()),
            StatusCode::FORBIDDEN,
            "M_FORBIDDEN",
            json!({"error": "no"}),
        ),
        (
            MatrixRejection::Unauthorized("sig".into()),
            StatusCode::UNAUTHORIZED,
            "M_UNAUTHORIZED",
            json!({"error": "sig"}),
        ),
        (
            MatrixRejection::wrong_backup_version("7"),
            StatusCode::FORBIDDEN,
            "M_WRONG_ROOM_KEYS_VERSION",
            json!({"current_version": "7"}),
        ),
        (
            MatrixRejection::RoomInUse("taken".into()),
            StatusCode::BAD_REQUEST,
            "M_ROOM_IN_USE",
            json!({"error": "taken"}),
        ),
        (
            MatrixRejection::UserInUse,
            StatusCode::BAD_REQUEST,
            "M_USER_IN_USE",
            json!({"error": "Username is already taken."}),
        ),
        (
            MatrixRejection::InvalidUsername("bad".into()),
            StatusCode::BAD_REQUEST,
            "M_INVALID_USERNAME",
            json!({}),
        ),
        (
            MatrixRejection::Exclusive("ns".into()),
            StatusCode::BAD_REQUEST,
            "M_EXCLUSIVE",
            json!({}),
        ),
        (
            MatrixRejection::UnableToAuthoriseJoin("x".into()),
            StatusCode::BAD_REQUEST,
            "M_UNABLE_TO_AUTHORISE_JOIN",
            json!({}),
        ),
        (
            MatrixRejection::UnableToGrantJoin("x".into()),
            StatusCode::BAD_REQUEST,
            "M_UNABLE_TO_GRANT_JOIN",
            json!({}),
        ),
        (
            MatrixRejection::MissingPathParams,
            StatusCode::INTERNAL_SERVER_ERROR,
            "M_UNKNOWN",
            json!({}),
        ),
        (
            MatrixRejection::missing_path_param("roomId"),
            StatusCode::BAD_REQUEST,
            "M_MISSING_PARAM",
            json!({"error": "Missing path parameter: roomId"}),
        ),
        (
            MatrixRejection::not_found("gone"),
            StatusCode::NOT_FOUND,
            "M_NOT_FOUND",
            json!({"error": "gone"}),
        ),
        (
            MatrixRejection::NoPathTemplate,
            StatusCode::INTERNAL_SERVER_ERROR,
            "M_UNKNOWN",
            json!({}),
        ),
        (
            MatrixRejection::missing_param("dir"),
            StatusCode::BAD_REQUEST,
            "M_MISSING_PARAM",
            json!({"error": "Missing parameter: dir"}),
        ),
        (
            MatrixRejection::invalid_param("w"),
            StatusCode::BAD_REQUEST,
            "M_INVALID_PARAM",
            json!({}),
        ),
        (
            MatrixRejection::UnknownPos("p".into()),
            StatusCode::BAD_REQUEST,
            "M_UNKNOWN_POS",
            json!({}),
        ),
        (
            MatrixRejection::InvalidSignature("s".into()),
            StatusCode::BAD_REQUEST,
            "M_INVALID_SIGNATURE",
            json!({}),
        ),
        (
            MatrixRejection::unrecognized("u"),
            StatusCode::NOT_FOUND,
            "M_UNRECOGNIZED",
            json!({}),
        ),
        (
            MatrixRejection::IncompatibleRoomVersion {
                room_version: "10".into(),
            },
            StatusCode::BAD_REQUEST,
            "M_INCOMPATIBLE_ROOM_VERSION",
            json!({"room_version": "10"}),
        ),
        (
            MatrixRejection::body_read_error("eof"),
            StatusCode::BAD_REQUEST,
            "M_BAD_JSON",
            json!({"error": "Failed to read request body: eof"}),
        ),
        (
            MatrixRejection::bad_json("x"),
            StatusCode::BAD_REQUEST,
            "M_BAD_JSON",
            json!({"error": "Invalid JSON: x"}),
        ),
        (
            MatrixRejection::RumaParseError("y".into()),
            StatusCode::BAD_REQUEST,
            "M_BAD_JSON",
            json!({"error": "Failed to deserialize request: y"}),
        ),
        (
            MatrixRejection::rate_limited(Some(1500)),
            StatusCode::TOO_MANY_REQUESTS,
            "M_LIMIT_EXCEEDED",
            json!({"retry_after_ms": 1500}),
        ),
        (
            MatrixRejection::TooLarge("big".into()),
            StatusCode::PAYLOAD_TOO_LARGE,
            "M_TOO_LARGE",
            json!({}),
        ),
        (
            MatrixRejection::ServerManaged("m".into()),
            StatusCode::METHOD_NOT_ALLOWED,
            "M_BAD_JSON",
            json!({}),
        ),
        (
            MatrixRejection::UnsupportedRoomVersion("99".into()),
            StatusCode::BAD_REQUEST,
            "M_UNSUPPORTED_ROOM_VERSION",
            json!({"error": "This homeserver does not support room version 99."}),
        ),
        (
            MatrixRejection::BadGateway("down".into()),
            StatusCode::BAD_GATEWAY,
            "M_UNKNOWN",
            json!({"error": "down"}),
        ),
        (
            MatrixRejection::RemoteTooLarge("r".into()),
            StatusCode::BAD_GATEWAY,
            "M_TOO_LARGE",
            json!({}),
        ),
        (
            MatrixRejection::NotYetUploaded("later".into()),
            StatusCode::GATEWAY_TIMEOUT,
            "M_NOT_YET_UPLOADED",
            json!({}),
        ),
        (
            MatrixRejection::CannotThumbnail("svg".into()),
            StatusCode::BAD_REQUEST,
            "M_UNKNOWN",
            json!({}),
        ),
        (
            MatrixRejection::internal_error("db password"),
            StatusCode::INTERNAL_SERVER_ERROR,
            "M_UNKNOWN",
            json!({"error": "Internal server error."}),
        ),
    ];

    for (rejection, status, errcode, fields) in cases {
        let label = format!("{rejection:?}");
        assert_eq!(rejection.status(), status, "{label}");
        assert_eq!(rejection.errcode(), errcode, "{label}");

        let response = rejection.into_response();
        assert_eq!(response.status(), status, "{label}");
        assert_eq!(
            response
                .headers()
                .get(CONTENT_TYPE)
                .map(HeaderValue::as_bytes),
            Some(&b"application/json"[..]),
            "{label}"
        );
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let body: Value = serde_json::from_slice(&bytes).expect("json body");
        assert_eq!(body["errcode"], errcode, "{label}");
        assert!(body["error"].is_string(), "{label}");
        for (key, value) in fields.as_object().expect("object") {
            assert_eq!(&body[key], value, "{label}: field {key}");
        }
    }

    // UIA answers with the UiaaInfo itself, not the error envelope
    let response = MatrixRejection::uiaa(uiaa()).into_response();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let body: Value = serde_json::from_slice(&bytes).expect("json body");
    assert_eq!(body["session"], "s1");
    assert!(body["flows"].is_array());
    assert!(body.get("errcode").is_none());
}

/// The ruma parse errors a handler sees most map onto param and JSON
/// errcodes, never M_UNKNOWN.
#[test]
fn ruma_parse_errors_map_to_client_errcodes() {
    use ruma::api::IncomingRequestExt;
    use ruma::api::client::message::get_message_events::v3 as messages;

    // `dir` is required
    let request = http::Request::builder()
        .method("GET")
        .uri("/_matrix/client/v3/rooms/!r:x/messages")
        .body(&b""[..])
        .expect("request");
    let err = messages::Request::try_from_http_request(request, &["!r:x"]).expect_err("no dir");
    assert_eq!(MatrixRejection::from(err).errcode(), "M_MISSING_PARAM");

    let request = http::Request::builder()
        .method("GET")
        .uri("/_matrix/client/v3/rooms/!r:x/messages?dir=b")
        .body(&b""[..])
        .expect("request");
    let err = messages::Request::try_from_http_request(request, &["not a room id"])
        .expect_err("bad room id");
    assert_eq!(MatrixRejection::from(err).errcode(), "M_INVALID_PARAM");
}
