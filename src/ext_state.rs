// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2025, Abdelkader Boudih

//! Extension-based state extractor
//!
//! Pulls the state from request extensions rather than from the router's
//! own state, so handlers work on a `Router<()>` while still reaching the
//! application state. [`StateLayer`](crate::StateLayer) injects it.
//!
//! # Example
//!
//! ```rust,ignore
//! use rama_x_ruma::ExtState;
//!
//! async fn handler(ExtState(state): ExtState<MyState>) -> impl IntoResponse {
//!     // state: Arc<MyState>
//! }
//! ```

use std::sync::Arc;

use rama::extensions::Extension;
use rama::http::request::Parts;
use rama::http::service::web::extract::FromPartsStateRefPair;

/// Extractor that retrieves `Arc<S>` from request extensions.
///
/// Used instead of `rama::http::service::web::extract::State` because the
/// state is injected into extensions by `StateLayer`, not held by the router.
pub struct ExtState<S>(pub Arc<S>);

impl<S> Clone for ExtState<S> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<S> std::fmt::Debug for ExtState<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtState").field("0", &"<state>").finish()
    }
}

impl<S> std::ops::Deref for ExtState<S> {
    type Target = Arc<S>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Error returned when state is not found in extensions
#[derive(Debug)]
pub struct ExtStateMissing;

impl std::fmt::Display for ExtStateMissing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "State not found in request extensions. Ensure StateLayer is applied."
        )
    }
}

impl std::error::Error for ExtStateMissing {}

impl rama::http::service::web::response::IntoResponse for ExtStateMissing {
    fn into_response(self) -> rama::http::Response {
        use rama::http::StatusCode;
        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()).into_response()
    }
}

// a parts extractor works in any argument position, including the last
impl<RouterState, S> FromPartsStateRefPair<RouterState> for ExtState<S>
where
    RouterState: Send + Sync,
    S: Extension,
{
    type Rejection = ExtStateMissing;

    async fn from_parts_state_ref_pair(
        parts: &Parts,
        _state: &RouterState,
    ) -> Result<Self, Self::Rejection> {
        let state = parts.extensions.get_arc::<S>().ok_or(ExtStateMissing)?;
        Ok(ExtState(state))
    }
}

#[cfg(test)]
mod tests;
