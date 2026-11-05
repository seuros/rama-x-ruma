// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2025, Abdelkader Boudih

//! Matrix Router Extension
//!
//! Provides a trait extension for the HTTP `Router` that auto-registers Matrix API endpoints
//! from Ruma's METADATA, handling path variable conversion and HTTP method mapping.
//!
//! # Example
//!
//! ```rust,ignore
//! use rama::http::service::web::Router;
//! use rama_x_ruma::{Matrix, MatrixResponse, MatrixRouterExt};
//! use ruma::api::client::session::get_login_types;
//!
//! async fn login_types_handler(
//!     _req: Matrix<get_login_types::v3::Request, MyState>,
//! ) -> MatrixResponse<get_login_types::v3::Response> {
//!     // ...
//! }
//!
//! let router = Router::new()
//!     .with_matrix_endpoint::<get_login_types::v3::Request, _, _>(login_types_handler);
//! ```

use rama::http::Method;
use rama::http::Request;
use rama::http::convert::hyperium::TryIntoRamaHttp;
use rama::http::service::web::response::{ErrorResponse, IntoResponse};
use rama::http::service::web::{IntoEndpointServiceWithState, Router};
use rama::telemetry::tracing;
use ruma::api::IncomingRequest;
use ruma::api::path_builder::PathBuilder;

use crate::auth::AuthScheme;

/// The method of Ruma endpoint `T` as a rama [`Method`].
///
/// Ruma's metadata is typed with the `http` crate; routing is rama's.
pub fn endpoint_method<T: IncomingRequest>() -> Method {
    T::METHOD
        .try_into_rama_http()
        .expect("ruma endpoint metadata holds a valid HTTP method")
}

/// A handler the default router endpoint layer accepts: it must render its
/// output and error into HTTP responses.
pub trait MatrixEndpoint<M, State>:
    IntoEndpointServiceWithState<
        M,
        State,
        Service: rama::Service<Request, Output: IntoResponse, Error: Into<ErrorResponse>>,
    >
{
}

impl<I, M, State> MatrixEndpoint<M, State> for I where
    I: IntoEndpointServiceWithState<
            M,
            State,
            Service: rama::Service<Request, Output: IntoResponse, Error: Into<ErrorResponse>>,
        >
{
}

/// Extension trait for the HTTP `Router` to register Matrix API endpoints.
///
/// This trait provides methods to automatically register routes using Ruma's
/// endpoint metadata, lowercasing path placeholders for the extractor.
pub trait MatrixRouterExt<State> {
    /// Register a Matrix API endpoint handler using Ruma's METADATA.
    ///
    /// This method extracts the HTTP method and all paths from the endpoint's
    /// METADATA and registers them with the provided handler.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The Ruma request type that implements `IncomingRequest`
    /// * `I` - The handler/service type
    /// * `M` - Marker type for endpoint service conversion
    ///
    /// # Path Conversion
    ///
    /// Ruma's `{variableName}` placeholders are rewritten to `{variablename}`,
    /// since the extractor looks captures up by lowercase name.
    fn with_matrix_endpoint<T, I, M>(self, handler: I) -> Self
    where
        T: IncomingRequest + Send + 'static,
        I: MatrixEndpoint<M, State> + Clone;

    /// Register a Matrix API endpoint for the primary (first) path only.
    ///
    /// Unlike `with_matrix_endpoint`, this only registers the first path
    /// from the endpoint's history. Useful when you don't need backward-compatible
    /// paths.
    fn with_matrix_endpoint_primary<T, I, M>(self, handler: I) -> Self
    where
        T: IncomingRequest + Send + 'static,
        I: MatrixEndpoint<M, State> + Clone;
}

impl<State> MatrixRouterExt<State> for Router<State>
where
    State: Clone + Send + Sync + 'static,
{
    fn with_matrix_endpoint<T, I, M>(mut self, handler: I) -> Self
    where
        T: IncomingRequest + Send + 'static,
        I: MatrixEndpoint<M, State> + Clone,
    {
        let method = &endpoint_method::<T>();

        // Register all paths from the endpoint's PathBuilder
        for ruma_path in T::PATH_BUILDER.all_paths() {
            for rama_path in route_paths(ruma_path) {
                self = register_route_with_method(self, method, &rama_path, handler.clone());
            }
        }

        self
    }

    fn with_matrix_endpoint_primary<T, I, M>(self, handler: I) -> Self
    where
        T: IncomingRequest + Send + 'static,
        I: MatrixEndpoint<M, State> + Clone,
    {
        let method = &endpoint_method::<T>();

        // Only register the first (primary) path
        if let Some(ruma_path) = T::PATH_BUILDER.all_paths().next() {
            let rama_path = convert_ruma_path_to_rama(ruma_path);
            return register_route_with_method(self, method, &rama_path, handler);
        }

        self
    }
}

/// Convert a Ruma path template to the router's path format.
///
/// Ruma uses `{variableName}` for path parameters; the router uses the same
/// brace syntax but captures are looked up lowercase.
///
/// # Examples
///
/// ```rust
/// # use rama_x_ruma::convert_ruma_path_to_rama;
/// assert_eq!(
///     convert_ruma_path_to_rama("/_matrix/client/v3/rooms/{roomId}/state/{eventType}"),
///     "/_matrix/client/v3/rooms/{roomid}/state/{eventtype}"
/// );
/// ```
pub fn convert_ruma_path_to_rama(ruma_path: &str) -> String {
    let mut result = String::with_capacity(ruma_path.len());
    let mut chars = ruma_path.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '{' {
            // Start of a variable - collect until '}'
            result.push('{');
            let mut var_name = String::new();
            while let Some(next_c) = chars.next_if(|&next_c| next_c != '}') {
                var_name.push(next_c);
            }
            chars.next_if_eq(&'}'); // consume the closing '}'
            // The extractor looks captures up by lowercase name
            result.push_str(&var_name.to_lowercase());
            result.push('}');
        } else {
            result.push(c);
        }
    }

    result
}

/// Trailing path placeholder that may legitimately be empty.
const STATE_KEY_SEGMENT: &str = "/{state_key}";

/// Every router path serving a Ruma path template.
///
/// A trailing `{stateKey}` may be the empty string, which clients send as
/// `.../state/{eventType}` or `.../state/{eventType}/`; an empty segment
/// never matches a placeholder, so both spellings get their own route and
/// the extractor fills in the empty key.
pub fn route_paths(ruma_path: &str) -> Vec<String> {
    let path = convert_ruma_path_to_rama(ruma_path);
    match path.strip_suffix(STATE_KEY_SEGMENT) {
        Some(prefix) => {
            let (bare, slashed) = (prefix.to_owned(), format!("{prefix}/"));
            vec![path, bare, slashed]
        }
        None => vec![path],
    }
}

/// Register a route with the given HTTP method and path.
///
/// This function dispatches to the appropriate Router method based on the HTTP method.
pub fn register_route_with_method<State, I, M>(
    router: Router<State>,
    method: &Method,
    path: &str,
    handler: I,
) -> Router<State>
where
    State: Clone + Send + Sync + 'static,
    I: MatrixEndpoint<M, State>,
{
    match method.as_str() {
        "GET" => router.with_get(path, handler),
        "POST" => router.with_post(path, handler),
        "PUT" => router.with_put(path, handler),
        "DELETE" => router.with_delete(path, handler),
        "PATCH" => router.with_patch(path, handler),
        "HEAD" => router.with_head(path, handler),
        "OPTIONS" => router.with_options(path, handler),
        "TRACE" => router.with_trace(path, handler),
        _ => {
            tracing::warn!(method = %method, path = %path, "Unsupported HTTP method for Matrix endpoint");
            router
        }
    }
}

/// Information about a registered Matrix endpoint.
///
/// Useful for debugging, introspection, and generating API documentation.
#[derive(Debug, Clone)]
pub struct MatrixEndpointInfo {
    /// The HTTP method for this endpoint.
    pub method: Method,
    /// The path template with `{lowercase}` params.
    pub path: String,
    /// The original Ruma path template (with {camelCase} params).
    pub ruma_path: String,
    /// Whether this endpoint requires authentication.
    pub requires_auth: bool,
    /// Whether this endpoint is rate-limited.
    pub rate_limited: bool,
}

impl MatrixEndpointInfo {
    /// Create endpoint info from a Ruma IncomingRequest type.
    ///
    /// Returns a Vec because endpoints can have multiple historical paths.
    pub fn from_request<T: IncomingRequest>() -> Vec<Self> {
        // An access token (even an optional one) or X-Matrix signatures.
        let requires_auth = matches!(
            AuthScheme::of::<T>(),
            Some(AuthScheme::AccessToken | AuthScheme::AccessTokenOptional)
                | Some(AuthScheme::ServerSignatures)
        );

        T::PATH_BUILDER
            .all_paths()
            .map(|ruma_path| MatrixEndpointInfo {
                method: endpoint_method::<T>(),
                path: convert_ruma_path_to_rama(ruma_path),
                ruma_path: ruma_path.to_string(),
                requires_auth,
                rate_limited: T::RATE_LIMITED,
            })
            .collect()
    }

    /// Get the primary (first) path info for an endpoint.
    pub fn primary<T: IncomingRequest>() -> Option<Self> {
        Self::from_request::<T>().into_iter().next()
    }
}

#[cfg(test)]
mod tests;
