// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2025, Abdelkader Boudih

//! Matrix rejection types
//!
//! This module provides error types that implement the HTTP server's `IntoResponse` trait
//! to return Matrix-spec-compliant JSON error responses.
//!
//! # Response Format
//!
//! All rejections produce responses matching the Matrix error format:
//! ```json
//! {"errcode": "M_UNKNOWN", "error": "An unknown error occurred"}
//! ```
//!
//! For `UnknownToken`, an additional `soft_logout` field may be included.

use rama::http::header::{CONTENT_TYPE, RETRY_AFTER};
use rama::http::service::web::response::IntoResponse;
use rama::http::{Body, HeaderValue, Response, StatusCode};
use rama::telemetry::tracing;
use ruma::api::client::uiaa::UiaaInfo;
use ruma::api::error::{DeserializationError, FromHttpRequestError};
use ruma::api::error::{
    ErrorKind, IncompatibleRoomVersionErrorData, LimitExceededErrorData, RetryAfter,
    UnknownTokenErrorData, WrongRoomKeysVersionErrorData,
};
use serde::Serialize;

use crate::errcode;

/// Standard Matrix error response body.
#[derive(Debug, Clone, Serialize)]
struct MatrixErrorBody {
    errcode: &'static str,
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    soft_logout: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    room_version: Option<String>,
}

/// Rejection type for Matrix API request failures.
///
/// Each variant maps to a specific Matrix error code and HTTP status.
/// Implements `IntoResponse` to produce spec-compliant error responses.
#[derive(Debug)]
pub enum MatrixRejection {
    // === Auth errors ===
    /// Missing access token in Authorization header (M_MISSING_TOKEN)
    MissingToken,
    /// Token format is invalid (M_UNKNOWN_TOKEN)
    InvalidToken,
    /// Token is unknown or expired (M_UNKNOWN_TOKEN with soft_logout)
    UnknownToken {
        /// If true, client should soft-logout (preserve encrypted data)
        soft_logout: bool,
    },
    /// Access forbidden for this resource/operation (M_FORBIDDEN)
    Forbidden(String),
    /// Request authentication is missing or invalid (M_UNAUTHORIZED, 401),
    /// used for X-Matrix server signatures
    Unauthorized(String),
    /// Key backup write targeted a stale/non-current backup version
    /// (M_WRONG_ROOM_KEYS_VERSION). Carries the server's current version so
    /// the client can re-sync.
    WrongBackupVersion {
        /// The backup version the server currently considers current.
        current_version: String,
    },
    /// Requested room alias is already taken (M_ROOM_IN_USE)
    RoomInUse(String),
    /// Requested username is already registered (M_USER_IN_USE)
    UserInUse,
    /// Requested username is not a valid user ID localpart (M_INVALID_USERNAME)
    InvalidUsername(String),
    /// Requested user ID is in an application service's exclusive
    /// namespace (M_EXCLUSIVE)
    Exclusive(String),
    /// None of a restricted room's allow conditions can be checked here
    /// (M_UNABLE_TO_AUTHORISE_JOIN, 400)
    UnableToAuthoriseJoin(String),
    /// The joiner qualifies but this server cannot authorise the join; try
    /// another (M_UNABLE_TO_GRANT_JOIN, 400)
    UnableToGrantJoin(String),

    // === Request parsing errors ===
    /// Path parameters are missing from the request context
    MissingPathParams,
    /// A required path parameter is missing (M_NOT_FOUND)
    MissingParam(String),
    /// The requested resource (room, event, user, ...) does not exist (M_NOT_FOUND, 404)
    NotFound(String),
    /// No path template was registered for this route
    NoPathTemplate,
    /// A required query or body parameter is absent (M_MISSING_PARAM, 400)
    MissingQueryParam(String),
    /// A parameter has an unacceptable value (M_INVALID_PARAM, 400)
    InvalidParam(String),
    /// A sliding sync `pos` unknown to this server (M_UNKNOWN_POS, 400)
    UnknownPos(String),
    /// A signature on uploaded keys is missing or does not verify
    /// (M_INVALID_SIGNATURE, 400)
    InvalidSignature(String),
    /// The endpoint or request kind is not served here (M_UNRECOGNIZED, 404)
    Unrecognized(String),
    /// The caller does not support the room's version (M_INCOMPATIBLE_ROOM_VERSION, 400)
    IncompatibleRoomVersion {
        /// The version of the room in question.
        room_version: String,
    },
    /// Failed to read request body (M_BAD_JSON)
    BodyReadError(String),
    /// Request body is not valid JSON (M_BAD_JSON)
    InvalidJson(String),
    /// Failed to parse request using Ruma types (M_BAD_JSON)
    RumaParseError(String),

    // === Rate limiting ===
    /// Rate limit exceeded (M_LIMIT_EXCEEDED)
    LimitExceeded {
        /// Suggested retry delay in milliseconds
        retry_after_ms: Option<u64>,
    },

    /// Request body exceeded the configured size limit (M_TOO_LARGE)
    TooLarge(String),

    /// The client tried to write data the server manages itself
    /// (M_BAD_JSON, 405)
    ServerManaged(String),

    /// The requested room version is not supported (M_UNSUPPORTED_ROOM_VERSION)
    UnsupportedRoomVersion(String),

    /// Another server needed for the answer failed us (M_UNKNOWN, 502)
    BadGateway(String),

    /// Remote content is too large to fetch or serve (M_TOO_LARGE, 502)
    RemoteTooLarge(String),

    /// The content is not available yet (M_NOT_YET_UPLOADED, 504)
    NotYetUploaded(String),

    /// The media cannot be thumbnailed, as the spec words it: the server
    /// "cannot thumbnail the content" (M_UNKNOWN, 400)
    CannotThumbnail(String),

    // === Internal errors ===
    /// Internal server error (M_UNKNOWN)
    Internal(String),

    // === User-Interactive Authentication ===
    /// User-Interactive Authentication is required to complete this request.
    ///
    /// Serializes the wrapped `UiaaInfo` directly (401), which already
    /// produces the spec-required `{flows, completed, session, errcode?,
    /// error?}` shape via its own `Serialize` implementation.
    Uiaa(Box<UiaaInfo>),
}

impl MatrixRejection {
    // === Convenience constructors (for compatibility with existing code) ===

    /// Create a MissingToken rejection
    pub fn missing_token() -> Self {
        Self::MissingToken
    }

    /// Create an InvalidToken rejection
    pub fn invalid_token() -> Self {
        Self::InvalidToken
    }

    /// Create a rejection for a missing path parameter
    pub fn missing_path_param(param: impl Into<String>) -> Self {
        Self::MissingParam(param.into())
    }

    /// Create a rejection for body read errors
    pub fn body_read_error(error: impl Into<String>) -> Self {
        Self::BodyReadError(error.into())
    }

    /// Create a rejection for request build errors
    pub fn request_build_error(error: impl Into<String>) -> Self {
        Self::Internal(format!("Failed to build request: {}", error.into()))
    }

    /// Create an internal error rejection
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    /// Create a forbidden rejection
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::Forbidden(message.into())
    }

    /// Create an unauthorized rejection (M_UNAUTHORIZED, HTTP 401)
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::Unauthorized(message.into())
    }

    /// Create a wrong-backup-version rejection (M_WRONG_ROOM_KEYS_VERSION)
    pub fn wrong_backup_version(current_version: impl Into<String>) -> Self {
        Self::WrongBackupVersion {
            current_version: current_version.into(),
        }
    }

    /// Create a not found rejection (M_NOT_FOUND, HTTP 404)
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    /// Create a missing parameter rejection (M_MISSING_PARAM, HTTP 400)
    pub fn missing_param(name: impl Into<String>) -> Self {
        Self::MissingQueryParam(name.into())
    }

    /// Create an invalid parameter rejection (M_INVALID_PARAM, HTTP 400)
    pub fn invalid_param(message: impl Into<String>) -> Self {
        Self::InvalidParam(message.into())
    }

    /// Create an unrecognized request rejection (M_UNRECOGNIZED, HTTP 404)
    pub fn unrecognized(message: impl Into<String>) -> Self {
        Self::Unrecognized(message.into())
    }

    /// Create a bad JSON rejection
    pub fn bad_json(message: impl Into<String>) -> Self {
        Self::InvalidJson(message.into())
    }

    /// Create a rate limited rejection
    pub fn rate_limited(retry_after_ms: Option<u64>) -> Self {
        Self::LimitExceeded { retry_after_ms }
    }

    /// Create a User-Interactive Authentication rejection (401 with flows).
    pub fn uiaa(info: UiaaInfo) -> Self {
        Self::Uiaa(Box::new(info))
    }

    /// Convert from Ruma's FromHttpRequestError
    pub fn from_ruma_error(error: FromHttpRequestError) -> Self {
        match error {
            FromHttpRequestError::Deserialization(DeserializationError::Query(err)) => {
                Self::from_query_error(&err.to_string())
            }
            FromHttpRequestError::Deserialization(DeserializationError::Ident(err)) => {
                Self::InvalidParam(format!("Invalid identifier: {err}"))
            }
            FromHttpRequestError::Deserialization(de_err) => {
                let err_string = de_err.to_string();
                if err_string.contains("header") {
                    Self::InvalidJson(format!("Invalid header: {}", err_string))
                } else {
                    Self::RumaParseError(err_string)
                }
            }
            // FromHttpRequestError is non-exhaustive, handle future variants
            _ => Self::Internal(format!("Unknown request parsing error: {}", error)),
        }
    }

    /// A query string that did not decode: a required parameter that is
    /// absent is `M_MISSING_PARAM`, anything else `M_INVALID_PARAM`.
    fn from_query_error(message: &str) -> Self {
        let missing = message
            .split_once("missing field `")
            .and_then(|(_, rest)| rest.split_once('`'))
            .map(|(field, _)| field);
        match missing {
            Some(field) => Self::MissingQueryParam(field.to_owned()),
            None => Self::InvalidParam(format!("Invalid query parameter: {message}")),
        }
    }

    /// Get the Matrix error code for this rejection.
    pub fn error_kind(&self) -> ErrorKind {
        match self {
            Self::MissingToken => ErrorKind::MissingToken,
            Self::InvalidToken => ErrorKind::UnknownToken(UnknownTokenErrorData::new()),
            Self::UnknownToken { soft_logout } => {
                let mut data = UnknownTokenErrorData::new();
                data.soft_logout = *soft_logout;
                ErrorKind::UnknownToken(data)
            }
            Self::Forbidden(_) => ErrorKind::Forbidden,
            Self::Unauthorized(_) => ErrorKind::Unauthorized,
            Self::WrongBackupVersion { current_version } => ErrorKind::WrongRoomKeysVersion(
                WrongRoomKeysVersionErrorData::new(current_version.clone()),
            ),
            Self::RoomInUse(_) => ErrorKind::RoomInUse,
            Self::UserInUse => ErrorKind::UserInUse,
            Self::InvalidUsername(_) => ErrorKind::InvalidUsername,
            Self::Exclusive(_) => ErrorKind::Exclusive,
            Self::UnableToAuthoriseJoin(_) => ErrorKind::UnableToAuthorizeJoin,
            Self::UnableToGrantJoin(_) => ErrorKind::UnableToGrantJoin,
            Self::MissingPathParams => ErrorKind::Unknown,
            Self::MissingParam(_) => ErrorKind::MissingParam,
            Self::NotFound(_) => ErrorKind::NotFound,
            Self::NoPathTemplate => ErrorKind::Unknown,
            Self::MissingQueryParam(_) => ErrorKind::MissingParam,
            Self::InvalidParam(_) => ErrorKind::InvalidParam,
            #[cfg(feature = "unstable-msc4186")]
            Self::UnknownPos(_) => ErrorKind::UnknownPos,
            #[cfg(not(feature = "unstable-msc4186"))]
            Self::UnknownPos(_) => ErrorKind::Unknown,
            Self::InvalidSignature(_) => ErrorKind::Unknown,
            Self::Unrecognized(_) => ErrorKind::Unrecognized,
            Self::IncompatibleRoomVersion { room_version } => {
                match ruma::RoomVersionId::try_from(room_version.as_str()) {
                    Ok(room_version) => ErrorKind::IncompatibleRoomVersion(
                        IncompatibleRoomVersionErrorData::new(room_version),
                    ),
                    Err(_) => ErrorKind::Unknown,
                }
            }
            Self::BodyReadError(_) => ErrorKind::BadJson,
            Self::InvalidJson(_) => ErrorKind::BadJson,
            Self::RumaParseError(_) => ErrorKind::BadJson,
            Self::LimitExceeded { retry_after_ms } => {
                let mut data = LimitExceededErrorData::new();
                data.retry_after = retry_after_ms
                    .map(|ms| RetryAfter::Delay(std::time::Duration::from_millis(ms)));
                ErrorKind::LimitExceeded(data)
            }
            Self::TooLarge(_) | Self::RemoteTooLarge(_) => ErrorKind::TooLarge,
            Self::NotYetUploaded(_) => ErrorKind::NotYetUploaded,
            Self::CannotThumbnail(_) => ErrorKind::Unknown,
            Self::ServerManaged(_) => ErrorKind::BadJson,
            Self::UnsupportedRoomVersion(_) => ErrorKind::UnsupportedRoomVersion,
            Self::Internal(_) | Self::BadGateway(_) => ErrorKind::Unknown,
            Self::Uiaa(_) => ErrorKind::Unknown,
        }
    }

    /// Get the error code string (e.g., "M_MISSING_TOKEN").
    pub fn errcode(&self) -> &'static str {
        match self {
            Self::MissingToken => errcode::MISSING_TOKEN,
            Self::InvalidToken | Self::UnknownToken { .. } => errcode::UNKNOWN_TOKEN,
            Self::Forbidden(_) => errcode::FORBIDDEN,
            Self::Unauthorized(_) => errcode::UNAUTHORIZED,
            Self::WrongBackupVersion { .. } => errcode::WRONG_ROOM_KEYS_VERSION,
            Self::RoomInUse(_) => errcode::ROOM_IN_USE,
            Self::UserInUse => errcode::USER_IN_USE,
            Self::InvalidUsername(_) => errcode::INVALID_USERNAME,
            Self::Exclusive(_) => errcode::EXCLUSIVE,
            Self::UnableToAuthoriseJoin(_) => errcode::UNABLE_TO_AUTHORISE_JOIN,
            Self::UnableToGrantJoin(_) => errcode::UNABLE_TO_GRANT_JOIN,
            Self::MissingPathParams
            | Self::NoPathTemplate
            | Self::Internal(_)
            | Self::BadGateway(_) => errcode::UNKNOWN,
            Self::MissingParam(_) | Self::MissingQueryParam(_) => errcode::MISSING_PARAM,
            Self::NotFound(_) => errcode::NOT_FOUND,
            Self::InvalidParam(_) => errcode::INVALID_PARAM,
            Self::UnknownPos(_) => errcode::UNKNOWN_POS,
            Self::InvalidSignature(_) => errcode::INVALID_SIGNATURE,
            Self::Unrecognized(_) => errcode::UNRECOGNIZED,
            Self::IncompatibleRoomVersion { .. } => errcode::INCOMPATIBLE_ROOM_VERSION,
            Self::BodyReadError(_)
            | Self::InvalidJson(_)
            | Self::RumaParseError(_)
            | Self::ServerManaged(_) => errcode::BAD_JSON,
            Self::LimitExceeded { .. } => errcode::LIMIT_EXCEEDED,
            Self::TooLarge(_) | Self::RemoteTooLarge(_) => errcode::TOO_LARGE,
            Self::NotYetUploaded(_) => errcode::NOT_YET_UPLOADED,
            Self::CannotThumbnail(_) => errcode::UNKNOWN,
            Self::UnsupportedRoomVersion(_) => errcode::UNSUPPORTED_ROOM_VERSION,
            // The real errcode (when present) is serialized straight from the
            // wrapped `UiaaInfo` by `into_response`; this is only a fallback
            // for callers inspecting `errcode()` directly.
            Self::Uiaa(_) => errcode::FORBIDDEN,
        }
    }

    /// Get the HTTP status code for this rejection.
    pub fn status(&self) -> StatusCode {
        match self {
            Self::MissingToken
            | Self::InvalidToken
            | Self::UnknownToken { .. }
            | Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) | Self::WrongBackupVersion { .. } => StatusCode::FORBIDDEN,
            Self::RoomInUse(_)
            | Self::UserInUse
            | Self::InvalidUsername(_)
            | Self::Exclusive(_)
            | Self::UnableToAuthoriseJoin(_)
            | Self::UnableToGrantJoin(_) => StatusCode::BAD_REQUEST,
            Self::MissingPathParams | Self::NoPathTemplate | Self::Internal(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            Self::MissingParam(_)
            | Self::MissingQueryParam(_)
            | Self::InvalidParam(_)
            | Self::UnknownPos(_)
            | Self::InvalidSignature(_)
            | Self::IncompatibleRoomVersion { .. }
            | Self::UnsupportedRoomVersion(_)
            | Self::BodyReadError(_)
            | Self::InvalidJson(_)
            | Self::RumaParseError(_) => StatusCode::BAD_REQUEST,
            Self::NotFound(_) | Self::Unrecognized(_) => StatusCode::NOT_FOUND,
            Self::LimitExceeded { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::TooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Self::ServerManaged(_) => StatusCode::METHOD_NOT_ALLOWED,
            Self::BadGateway(_) | Self::RemoteTooLarge(_) => StatusCode::BAD_GATEWAY,
            Self::NotYetUploaded(_) => StatusCode::GATEWAY_TIMEOUT,
            Self::CannotThumbnail(_) => StatusCode::BAD_REQUEST,
            Self::Uiaa(_) => StatusCode::UNAUTHORIZED,
        }
    }

    /// Get the human-readable error message.
    pub fn message(&self) -> String {
        match self {
            Self::MissingToken => "Missing access token.".to_owned(),
            Self::InvalidToken => "Invalid access token.".to_owned(),
            Self::UnknownToken { .. } => "Unknown or expired access token.".to_owned(),
            Self::Forbidden(msg) | Self::Unauthorized(msg) => msg.clone(),
            Self::WrongBackupVersion { .. } => "Wrong backup version.".to_owned(),
            Self::UserInUse => "Username is already taken.".to_owned(),
            Self::RoomInUse(msg)
            | Self::InvalidUsername(msg)
            | Self::Exclusive(msg)
            | Self::UnableToAuthoriseJoin(msg)
            | Self::UnableToGrantJoin(msg) => msg.clone(),
            Self::MissingPathParams => "Missing path parameters.".to_owned(),
            Self::MissingParam(param) => format!("Missing path parameter: {}", param),
            Self::NotFound(msg)
            | Self::InvalidParam(msg)
            | Self::UnknownPos(msg)
            | Self::InvalidSignature(msg)
            | Self::Unrecognized(msg) => msg.clone(),
            Self::MissingQueryParam(name) => format!("Missing parameter: {name}"),
            Self::IncompatibleRoomVersion { room_version } => {
                format!("Your homeserver does not support room version {room_version}.")
            }
            Self::NoPathTemplate => "No path template registered.".to_owned(),
            Self::BodyReadError(msg) => format!("Failed to read request body: {}", msg),
            Self::InvalidJson(msg) => format!("Invalid JSON: {}", msg),
            Self::RumaParseError(msg) => format!("Failed to deserialize request: {}", msg),
            Self::LimitExceeded { retry_after_ms } => {
                if let Some(ms) = retry_after_ms {
                    format!("Too many requests. Retry after {} ms.", ms)
                } else {
                    "Too many requests.".to_owned()
                }
            }
            Self::Internal(msg) => {
                // Log internal errors but don't expose details to clients
                tracing::error!("Internal error: {}", msg);
                "Internal server error.".to_owned()
            }
            Self::TooLarge(msg)
            | Self::ServerManaged(msg)
            | Self::BadGateway(msg)
            | Self::RemoteTooLarge(msg)
            | Self::NotYetUploaded(msg)
            | Self::CannotThumbnail(msg) => msg.clone(),
            Self::UnsupportedRoomVersion(version) => {
                format!("This homeserver does not support room version {version}.")
            }
            Self::Uiaa(info) => info
                .auth_error
                .as_ref()
                .map(|e| e.message.clone())
                .unwrap_or_else(|| "User-interactive authentication required.".to_owned()),
        }
    }

    /// Build the response body as JSON bytes.
    fn to_json_bytes(&self) -> Vec<u8> {
        let soft_logout = match self {
            Self::UnknownToken { soft_logout } => Some(*soft_logout),
            _ => None,
        };

        let retry_after_ms = match self {
            Self::LimitExceeded { retry_after_ms } => *retry_after_ms,
            _ => None,
        };

        let current_version = match self {
            Self::WrongBackupVersion { current_version } => Some(current_version.clone()),
            _ => None,
        };

        let room_version = match self {
            Self::IncompatibleRoomVersion { room_version } => Some(room_version.clone()),
            _ => None,
        };

        let body = MatrixErrorBody {
            errcode: self.errcode(),
            error: self.message(),
            soft_logout,
            retry_after_ms,
            current_version,
            room_version,
        };

        serde_json::to_vec(&body).unwrap_or_else(|_| {
            br#"{"errcode":"M_UNKNOWN","error":"Failed to serialize error"}"#.to_vec()
        })
    }
}

impl std::fmt::Display for MatrixRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: {} - {}",
            self.status(),
            self.errcode(),
            self.message()
        )
    }
}

impl std::error::Error for MatrixRejection {}

impl IntoResponse for MatrixRejection {
    fn into_response(self) -> Response {
        if let Self::Uiaa(info) = &self {
            let body_bytes = serde_json::to_vec(info).unwrap_or_else(|_| {
                br#"{"errcode":"M_FORBIDDEN","error":"User-interactive authentication required.","flows":[]}"#
                    .to_vec()
            });
            return json_response(StatusCode::UNAUTHORIZED, body_bytes);
        }

        let status = self.status();
        let body_bytes = self.to_json_bytes();
        let mut response = json_response(status, body_bytes);
        // The body's `retry_after_ms`, for clients and proxies that only
        // read the header (whole seconds, rounded up so nobody retries
        // early).
        if let Self::LimitExceeded {
            retry_after_ms: Some(ms),
        } = self
        {
            response
                .headers_mut()
                .insert(RETRY_AFTER, HeaderValue::from(ms.div_ceil(1000)));
        }
        response
    }
}

/// Builds an `application/json` response without a fallible builder.
fn json_response(status: StatusCode, body_bytes: Vec<u8>) -> Response {
    let mut response = Response::new(Body::from(body_bytes));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

// === From implementations for common error types ===

/// Lets handlers take extractors that cannot fail, such as the raw request.
impl From<std::convert::Infallible> for MatrixRejection {
    fn from(never: std::convert::Infallible) -> Self {
        match never {}
    }
}

impl From<serde_json::Error> for MatrixRejection {
    fn from(err: serde_json::Error) -> Self {
        Self::InvalidJson(err.to_string())
    }
}

impl From<FromHttpRequestError> for MatrixRejection {
    fn from(err: FromHttpRequestError) -> Self {
        Self::from_ruma_error(err)
    }
}

impl From<crate::ext_state::ExtStateMissing> for MatrixRejection {
    fn from(err: crate::ext_state::ExtStateMissing) -> Self {
        Self::internal_error(err.to_string())
    }
}

impl From<rama::http::service::web::extract::path::PathRejection> for MatrixRejection {
    fn from(err: rama::http::service::web::extract::path::PathRejection) -> Self {
        Self::MissingParam(err.to_string())
    }
}

impl From<rama::http::service::web::extract::query::FailedToDeserializeQueryString>
    for MatrixRejection
{
    fn from(err: rama::http::service::web::extract::query::FailedToDeserializeQueryString) -> Self {
        use std::error::Error as _;
        // the wrapped decoder error names the offending field
        let detail = err
            .source()
            .map_or_else(|| err.body_text(), ToString::to_string);
        Self::from_query_error(&detail)
    }
}

impl From<rama::http::service::web::extract::body::JsonRejection> for MatrixRejection {
    fn from(err: rama::http::service::web::extract::body::JsonRejection) -> Self {
        Self::bad_json(err.to_string())
    }
}

#[cfg(test)]
mod tests;
