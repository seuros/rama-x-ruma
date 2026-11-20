// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2025, Abdelkader Boudih

//! Why a client request failed.

use std::error::Error as StdError;
use std::fmt;
use std::time::{Duration, SystemTime};

use rama::error::BoxError;
use ruma::api::client::uiaa::UiaaResponse;
use ruma::api::error::{
    Error as MatrixError, ErrorCode, ErrorKind, FromHttpResponseError, IntoHttpError,
    LimitExceededErrorData, RetryAfter,
};

/// A failed [`MatrixClient::send`](super::MatrixClient::send).
///
/// `E` is the endpoint's error type: ruma's [`MatrixError`] for most
/// endpoints, [`UiaaResponse`] for those behind user-interactive auth.
#[derive(Debug)]
#[non_exhaustive]
pub enum ClientError<E = MatrixError> {
    /// ruma could not build the request: a required token is missing, no
    /// supported version has a path for the endpoint, or serialization failed.
    IntoHttp(IntoHttpError),
    /// Translating between rama and `http` crate types failed.
    Convert(BoxError),
    /// Connecting, sending or reading the response failed.
    Transport(BoxError),
    /// No full response within the timeout.
    Timeout(Duration),
    /// The response body exceeded the client's limit (bytes).
    BodyTooLarge {
        /// The limit in bytes.
        limit: usize,
    },
    /// The homeserver answered with an error, or a body ruma could not parse.
    Response {
        /// The answer's HTTP status.
        status: http::StatusCode,
        /// The answer's `Retry-After` header (seconds or HTTP date), whatever
        /// its body; a proxy's 429 or 503 carries no Matrix error.
        retry_after_header: Option<RetryAfter>,
        /// ruma's reading of the answer.
        error: FromHttpResponseError<E>,
    },
}

impl<E> ClientError<E> {
    /// The homeserver's error, if it answered with one.
    pub fn server_error(&self) -> Option<&E> {
        match self {
            Self::Response {
                error: FromHttpResponseError::Server(error),
                ..
            } => Some(error),
            _ => None,
        }
    }

    /// The HTTP status of the homeserver's answer, UIA challenges and
    /// unparseable bodies included.
    pub fn status(&self) -> Option<http::StatusCode> {
        match self {
            Self::Response { status, .. } => Some(*status),
            _ => None,
        }
    }
}

impl<E: MatrixErrorBody> ClientError<E> {
    /// The standard Matrix error the homeserver answered with.
    pub fn matrix_error(&self) -> Option<&MatrixError> {
        self.server_error().and_then(MatrixErrorBody::matrix_error)
    }

    /// The error's kind, parsed from `errcode` and its extra fields.
    pub fn error_kind(&self) -> Option<&ErrorKind> {
        self.matrix_error().and_then(MatrixError::error_kind)
    }

    /// The Matrix `errcode` (`M_FORBIDDEN`, `M_LIMIT_EXCEEDED`, ...).
    pub fn errcode(&self) -> Option<ErrorCode> {
        self.error_kind().map(ErrorKind::errcode)
    }

    /// How long to wait before retrying; a past date is zero.
    ///
    /// An `M_LIMIT_EXCEEDED` body's value comes first (ruma prefers its
    /// `Retry-After` header over `retry_after_ms`); otherwise any answer's
    /// `Retry-After` header, such as a proxy's 429 or 503 with an HTML body.
    pub fn retry_after(&self) -> Option<Duration> {
        let from_body = match self.error_kind() {
            Some(ErrorKind::LimitExceeded(LimitExceededErrorData {
                retry_after: Some(retry_after),
                ..
            })) => Some(retry_after),
            _ => None,
        };
        let from_header = match self {
            Self::Response {
                retry_after_header, ..
            } => retry_after_header.as_ref(),
            _ => None,
        };
        from_body.or(from_header).map(delay)
    }

    /// Transport trouble, timeouts, rate limits and 5xx may pass; anything
    /// else fails the same way again.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Transport(_) | Self::Timeout(_) => true,
            Self::Response {
                status,
                error: FromHttpResponseError::Server(_),
                ..
            } => {
                status.is_server_error()
                    || *status == http::StatusCode::TOO_MANY_REQUESTS
                    || matches!(self.error_kind(), Some(ErrorKind::LimitExceeded(_)))
            }
            _ => false,
        }
    }
}

/// A `Retry-After` header: delay-seconds or an HTTP date. Seconds beyond
/// `u64` cap at the maximum; anything else malformed is no hint at all.
pub(super) fn parse_retry_after(value: &http::HeaderValue) -> Option<RetryAfter> {
    match RetryAfter::try_from(value) {
        Ok(retry_after) => Some(retry_after),
        Err(_) => {
            let digits = value.as_bytes();
            (!digits.is_empty() && digits.iter().all(u8::is_ascii_digit))
                .then(|| RetryAfter::Delay(Duration::from_secs(u64::MAX)))
        }
    }
}

/// The wait `retry_after` asks for, from now.
fn delay(retry_after: &RetryAfter) -> Duration {
    match retry_after {
        RetryAfter::Delay(delay) => *delay,
        RetryAfter::DateTime(at) => at
            .duration_since(SystemTime::now())
            .unwrap_or(Duration::ZERO),
    }
}

impl<E: fmt::Display> fmt::Display for ClientError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IntoHttp(error) => write!(f, "cannot build the request: {error}"),
            Self::Convert(error) => write!(f, "cannot convert the HTTP message: {error}"),
            Self::Transport(error) => write!(f, "homeserver unreachable: {error}"),
            Self::Timeout(limit) => write!(f, "no response within {limit:?}"),
            Self::BodyTooLarge { limit } => write!(f, "response body over {limit} bytes"),
            Self::Response { error, .. } => error.fmt(f),
        }
    }
}

impl<E: StdError + 'static> StdError for ClientError<E> {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::IntoHttp(error) => Some(error),
            Self::Convert(error) | Self::Transport(error) => Some(&**error),
            Self::Response { error, .. } => Some(error),
            Self::Timeout(_) | Self::BodyTooLarge { .. } => None,
        }
    }
}

/// An endpoint error that may carry a standard Matrix error.
pub trait MatrixErrorBody {
    /// The standard Matrix error, if this is one.
    fn matrix_error(&self) -> Option<&MatrixError>;
}

impl MatrixErrorBody for MatrixError {
    fn matrix_error(&self) -> Option<&MatrixError> {
        Some(self)
    }
}

impl MatrixErrorBody for UiaaResponse {
    fn matrix_error(&self) -> Option<&MatrixError> {
        match self {
            Self::MatrixError(error) => Some(error),
            Self::AuthResponse(_) => None,
        }
    }
}
