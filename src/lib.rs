// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2025, Abdelkader Boudih
// See the LICENSE file at the root of this crate for the full license text.

//! Serve [ruma]'s typed Matrix API endpoints on [rama] HTTP.
//!
//! ruma describes every Matrix endpoint as a request type (path templates,
//! method, authentication scheme, how to parse it from HTTP) and a response
//! type (how to render it). This crate plugs those into rama's web router.
//!
//! ```text
//! HTTP request
//!   -> StateLayer<S>          puts Arc<S> into the request extensions
//!   -> Router                 routes registered from ruma metadata
//!                             (MatrixRouterExt::with_matrix_endpoint)
//!   -> Matrix<T, S>           path params in template order, auth scheme
//!                             checked through S: MatrixAuth, body bytes,
//!                             T::try_from_http_request
//!   -> handler                returns MatrixResult<T::OutgoingResponse>
//!   -> MatrixResponse / MatrixRejection   rendered as Matrix JSON
//! ```
//!
//! # Components
//!
//! - [`Matrix`]: extractor for ruma `IncomingRequest` types, carrying the
//!   authenticated identity.
//! - [`MatrixAuth`]: what the server provides: resolving access tokens and
//!   verifying X-Matrix signatures.
//! - [`MatrixResponse`] / [`MatrixResult`]: render ruma `OutgoingResponse`s.
//! - [`MatrixRejection`]: Matrix errcodes, HTTP statuses and JSON bodies.
//! - [`MatrixRouterExt`]: register handlers from ruma endpoint metadata.
//! - [`StateLayer`], [`ExtState`], [`RequireUser`]: state and identity for
//!   routes without a ruma request type.
//!
//! # Example
//!
//! ```rust
//! use rama::http::service::web::Router;
//! use rama_x_ruma::{Matrix, MatrixAuth, MatrixResponse, MatrixResult, MatrixRouterExt, TokenIdentity};
//! use ruma::api::client::session::get_login_types::v3 as login_types;
//!
//! #[derive(Debug)]
//! struct Server;
//!
//! // request extensions carry the state
//! impl rama::extensions::Extension for Server {}
//!
//! impl MatrixAuth for Server {
//!     async fn resolve_token(&self, _token: &str) -> TokenIdentity {
//!         TokenIdentity::Invalid { soft_logout: false }
//!     }
//! }
//!
//! async fn login_types(
//!     _req: Matrix<login_types::Request, Server>,
//! ) -> MatrixResult<login_types::Response> {
//!     Ok(MatrixResponse::new(login_types::Response::new(vec![])))
//! }
//!
//! let router = Router::new()
//!     .with_matrix_endpoint::<login_types::Request, _, _>(login_types);
//! # let _ = router;
//! ```

pub mod auth;
pub mod errcode;
pub mod ext_state;
pub mod extractor;
mod percent;
pub mod rejection;
pub mod require_user;
pub mod response;
pub mod router_ext;
pub mod state_layer;

pub use auth::{
    AuthContext, AuthScheme, MatrixAuth, TokenIdentity, extract_access_token, handle_auth, identify,
};
pub use ext_state::{ExtState, ExtStateMissing};
pub use extractor::{Matrix, body_rejection};
pub use rejection::MatrixRejection;
pub use require_user::RequireUser;
pub use response::{MatrixResponse, MatrixResult, RumaResponse, matrix_result};
pub use router_ext::{
    MatrixEndpoint, MatrixEndpointInfo, MatrixRouterExt, convert_ruma_path_to_rama,
    endpoint_method, register_route_with_method, route_paths,
};
pub use state_layer::{StateLayer, StateService};

/// Compiles the README's example.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
