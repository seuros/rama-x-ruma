// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2025, Abdelkader Boudih

//! Extractor requiring a valid user access token for non-ruma endpoints.
//!
//! [`Matrix<T, S>`](crate::Matrix) drives authentication from a ruma
//! endpoint's declared `AuthScheme`, so routes without a ruma type have
//! nothing to read that scheme from. This extractor runs the same
//! `AccessToken` check directly.

use std::sync::Arc;

use rama::http::request::Parts;
use rama::http::service::web::extract::FromPartsStateRefPair;

use crate::auth::{AuthContext, AuthScheme, MatrixAuth, handle_auth};
use crate::rejection::MatrixRejection;

/// The authenticated requester of an endpoint without a ruma request type.
///
/// Rejects the request the same way `Matrix<T, S>` would for an
/// `AccessToken`-scheme endpoint: missing, invalid and revoked tokens are
/// all rejected before the handler runs.
///
/// ```rust,ignore
/// async fn handler(RequireUser { auth, .. }: RequireUser<MyState>) { /* ... */ }
/// ```
pub struct RequireUser<S> {
    /// Who the token belongs to.
    pub auth: AuthContext,
    /// The state that authenticated the request.
    pub state: Arc<S>,
}

impl<S> std::fmt::Debug for RequireUser<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequireUser")
            .field("auth", &self.auth)
            .field("state", &"<state>")
            .finish()
    }
}

impl<RouterState, S> FromPartsStateRefPair<RouterState> for RequireUser<S>
where
    RouterState: Send + Sync,
    S: MatrixAuth,
{
    type Rejection = MatrixRejection;

    async fn from_parts_state_ref_pair(
        parts: &Parts,
        _state: &RouterState,
    ) -> Result<Self, Self::Rejection> {
        let state = parts.extensions.get_arc::<S>().ok_or_else(|| {
            MatrixRejection::internal_error("state not found in request extensions")
        })?;

        let auth = handle_auth(parts, AuthScheme::AccessToken, &*state, &[]).await?;

        Ok(RequireUser { auth, state })
    }
}
