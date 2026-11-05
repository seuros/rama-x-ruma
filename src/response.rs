// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2025, Abdelkader Boudih

//! Matrix response wrapper
//!
//! Bridges Ruma's `OutgoingResponse` trait to the HTTP server's `IntoResponse` trait,
//! enabling Matrix API endpoints to return Ruma response types directly.

use rama::bytes::BytesMut;
use rama::http::convert::hyperium::TryIntoRamaHttp;
use rama::http::service::web::response::IntoResponse;
use rama::http::{Body, Response, StatusCode};
use rama::telemetry::tracing;
use ruma::api::{OutgoingResponse, OutgoingResponseExt};
use std::ops::Deref;

/// Wrapper that converts Ruma Matrix responses to HTTP responses.
///
/// This type implements the `IntoResponse` trait for any type that implements
/// Ruma's `OutgoingResponse` trait, bridging the two frameworks.
///
/// # Example
///
/// ```rust
/// use rama_x_ruma::MatrixResponse;
/// use ruma::api::client::sync::sync_events;
///
/// async fn sync_handler() -> MatrixResponse<sync_events::v3::Response> {
///     let response = sync_events::v3::Response::new("batch_token".to_owned());
///     MatrixResponse(response)
/// }
/// ```
#[derive(Clone, Debug)]
pub struct MatrixResponse<T>(pub T);

impl<T: OutgoingResponse> IntoResponse for MatrixResponse<T> {
    fn into_response(self) -> Response {
        match self.0.try_into_http_response::<BytesMut>() {
            Ok(http_response) => {
                // ruma builds `http` crate types; convert to our native ones
                match http_response
                    .map(|body| Body::from(body.freeze()))
                    .try_into_rama_http()
                {
                    Ok(response) => response,
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to convert Matrix response");
                        StatusCode::INTERNAL_SERVER_ERROR.into_response()
                    }
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to serialize Matrix response");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }
}

impl<T> From<T> for MatrixResponse<T> {
    fn from(response: T) -> Self {
        Self(response)
    }
}

impl<T> Deref for MatrixResponse<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> MatrixResponse<T> {
    /// Create a new MatrixResponse wrapper.
    pub fn new(response: T) -> Self {
        Self(response)
    }

    /// Unwrap and return the inner response.
    pub fn into_inner(self) -> T {
        self.0
    }
}

/// Result type for Matrix API handlers.
///
/// Both arms render as HTTP responses, so handlers can return it directly;
/// [`matrix_result`] converts one explicitly. Server error types convert
/// into [`MatrixRejection`] through `From`, so `?` works on them.
///
/// # Example
///
/// ```rust,ignore
/// use rama_x_ruma::{MatrixResponse, MatrixResult, matrix_result};
///
/// async fn get_event() -> impl IntoResponse {
///     let result: MatrixResult<Response> = async {
///         let response = ...?;  // errors with `From` into MatrixRejection
///         Ok(MatrixResponse::new(response))
///     }.await;
///     matrix_result(result)
/// }
/// ```
pub type MatrixResult<T> = Result<MatrixResponse<T>, crate::rejection::MatrixRejection>;

/// Convert a `MatrixResult<T>` to an HTTP response.
///
/// This helper function converts the result to a proper Matrix API response,
/// handling both success and error cases.
pub fn matrix_result<T: OutgoingResponse>(result: MatrixResult<T>) -> Response {
    match result {
        Ok(response) => response.into_response(),
        Err(rejection) => rejection.into_response(),
    }
}

// Backwards compatibility alias
pub type RumaResponse<T> = MatrixResponse<T>;

#[cfg(test)]
mod tests;
