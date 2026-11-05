// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2025, Abdelkader Boudih

//! Matrix request extractor
//!
//! Bridges rama's `FromRequest` trait to ruma's `IncomingRequest` trait,
//! enabling type-safe Matrix API endpoint handlers.

use std::ops::Deref;
use std::sync::Arc;

use rama::extensions::Extensions;
use rama::http::body::util::{BodyExt, LengthLimitError};
use rama::http::{Request, matcher::UriParams, service::web::extract::FromRequest};
use rama::telemetry::tracing::{debug, warn};
use ruma::{
    OwnedDeviceId, OwnedServerName, OwnedUserId,
    api::path_builder::PathBuilder,
    api::{IncomingRequest, IncomingRequestExt},
};

use crate::auth::{AuthScheme, MatrixAuth, handle_auth};
use crate::rejection::MatrixRejection;

/// A parsed ruma request plus the identity that sent it.
///
/// `S` is the state [`StateLayer`](crate::StateLayer) injects; it
/// authenticates the request through [`MatrixAuth`].
///
/// # Example
///
/// ```rust,ignore
/// use rama_x_ruma::Matrix;
/// use ruma::api::client::sync::sync_events;
///
/// async fn sync_handler(req: Matrix<sync_events::v3::Request, MyState>) -> impl IntoResponse {
///     // req.body is the parsed request; sender_user, sender_device beside it
/// }
/// ```
pub struct Matrix<T: IncomingRequest, S> {
    /// The parsed Ruma request body
    pub body: T,
    /// The authenticated user ID (if any)
    pub sender_user: Option<OwnedUserId>,
    /// The authenticated device ID (if any)
    pub sender_device: Option<OwnedDeviceId>,
    /// The sender server name (for federation requests)
    pub sender_servername: Option<OwnedServerName>,
    /// The appservice ID the request authenticated as, for `AppserviceToken`
    /// and `AppserviceTokenOptional` endpoints.
    pub sender_appservice: Option<String>,
    /// The application state (injected by StateLayer)
    pub state: Option<Arc<S>>,
    /// The request's query string, for parameters Ruma does not parse
    /// (unstable names such as `org.matrix.msc4222.use_state_after`).
    pub raw_query: Option<String>,
}

impl<T: IncomingRequest, S> Deref for Matrix<T, S> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.body
    }
}

impl<T, S> std::fmt::Debug for Matrix<T, S>
where
    T: IncomingRequest + std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Matrix")
            .field("body", &self.body)
            .field("sender_user", &self.sender_user)
            .field("sender_device", &self.sender_device)
            .field("sender_servername", &self.sender_servername)
            .field("sender_appservice", &self.sender_appservice)
            .field("state", &self.state.as_ref().map(|_| "<state>"))
            .field("raw_query", &self.raw_query)
            .finish()
    }
}

impl<T: IncomingRequest, S> Matrix<T, S> {
    /// Get the application state, panicking if not available.
    ///
    /// This is safe to call for any endpoint that requires authentication,
    /// as the state is always injected by StateLayer.
    ///
    /// # Panics
    ///
    /// Panics if state was not injected (missing StateLayer).
    pub fn app_state(&self) -> &Arc<S> {
        self.state
            .as_ref()
            .expect("StateLayer not applied - state missing")
    }

    /// Get the application state, returning None if not available.
    pub fn try_app_state(&self) -> Option<&Arc<S>> {
        self.state.as_ref()
    }
}

impl<T, S> FromRequest for Matrix<T, S>
where
    T: IncomingRequest + Send + 'static,
    S: MatrixAuth,
{
    type Rejection = MatrixRejection;

    async fn from_request(req: Request) -> Result<Self, Self::Rejection> {
        let (parts, body) = req.into_parts();

        let body_bytes = body
            .collect()
            .await
            .map_err(|e| body_rejection(&e))?
            .to_bytes();

        // Only required for authenticated endpoints
        let state = parts.extensions.get_arc::<S>();

        let Some(declared) = AuthScheme::of::<T>() else {
            // an unknown scheme must never fall through to a weaker check
            return Err(MatrixRejection::internal_error(format!(
                "unsupported authentication scheme {}",
                std::any::type_name::<T::Authentication>()
            )));
        };
        let auth_scheme = S::auth_scheme_for::<T>(declared);

        let (sender_user, sender_device, sender_servername, sender_appservice) = match auth_scheme {
            AuthScheme::None => (None, None, None, None),
            scheme => {
                let auth_state = state.clone().ok_or_else(|| {
                    MatrixRejection::internal_error("state not found in request extensions")
                })?;

                let auth_context = handle_auth(&parts, scheme, &*auth_state, &body_bytes).await?;

                (
                    auth_context.sender_user,
                    auth_context.sender_device,
                    auth_context.sender_servername,
                    auth_context.appservice_id,
                )
            }
        };

        let path_args = extract_path_args_in_order::<T>(&parts.extensions)?;

        // ruma parses `http` crate requests
        let mut http_request = ::http::Request::builder()
            .method(parts.method.as_str())
            .uri(parts.uri.request_target().as_ref());

        if let Some(headers) = http_request.headers_mut() {
            for (name, value) in parts.headers.iter() {
                if let Ok(http_value) = ::http::HeaderValue::from_bytes(value.as_bytes()) {
                    headers.append(
                        ::http::HeaderName::from_bytes(name.as_str().as_bytes())
                            .unwrap_or_else(|_| ::http::HeaderName::from_static("x-unknown")),
                        http_value,
                    );
                }
            }
        }

        let http_request = http_request
            .body(body_bytes.as_ref())
            .map_err(|e| MatrixRejection::request_build_error(e.to_string()))?;

        debug!(
            "Parsing {} request with {} path args",
            std::any::type_name::<T>(),
            path_args.len()
        );

        let path_args: Vec<&str> = path_args.iter().map(String::as_str).collect();
        let body = T::try_from_http_request(http_request, &path_args).map_err(|e| {
            warn!("Failed to parse Matrix request: {:?}", e);
            MatrixRejection::from_ruma_error(e)
        })?;

        Ok(Matrix {
            body,
            sender_user,
            sender_device,
            sender_servername,
            sender_appservice,
            state,
            raw_query: parts.uri.query().map(|query| query.to_string()),
        })
    }
}

/// Path arguments in the order ruma's `try_from_http_request` expects:
/// the order of the placeholders in the endpoint's first path template
/// (e.g. `/_matrix/client/v3/rooms/{roomId}/state/{eventType}/{stateKey}`),
/// each looked up by lowercase name in the router's [`UriParams`].
fn extract_path_args_in_order<T: IncomingRequest>(
    extensions: &Extensions,
) -> Result<Vec<String>, MatrixRejection> {
    let Some(path_template) = T::PATH_BUILDER.all_paths().next() else {
        return Ok(Vec::new());
    };

    // set by the router's path matcher
    let uri_params = extensions.get_ref::<UriParams>();

    let mut path_args = Vec::new();

    for segment in path_template.split('/') {
        if let Some(var_name) = segment
            .strip_prefix('{')
            .and_then(|s: &str| s.strip_suffix('}'))
        {
            // routes register their captures lowercase (convert_ruma_path_to_rama)
            let var_name_lower = var_name.to_lowercase();

            let value = match uri_params.and_then(|params| params.get(&var_name_lower)) {
                Some(value) => value,
                // routed through the empty-state-key alias of the path
                None if var_name_lower == "state_key" => "",
                None => return Err(MatrixRejection::missing_path_param(var_name.to_string())),
            };

            path_args.push(value.to_string());
        }
    }

    Ok(path_args)
}

/// The rejection for a request body that failed to read.
///
/// A body limit layer fails the read once the cap is crossed; that is a
/// size problem (`M_TOO_LARGE`), not malformed JSON.
pub fn body_rejection(err: &(dyn std::error::Error + 'static)) -> MatrixRejection {
    let too_large =
        std::iter::successors(Some(err), |e| e.source()).any(|e| e.is::<LengthLimitError>());
    if too_large {
        MatrixRejection::TooLarge(err.to_string())
    } else {
        MatrixRejection::body_read_error(err.to_string())
    }
}

#[cfg(test)]
mod tests;
