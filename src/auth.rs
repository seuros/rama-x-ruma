// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2025, Abdelkader Boudih

//! Matrix request authentication.
//!
//! [`handle_auth`] enforces every ruma authentication scheme:
//! - `None`: no authentication required (a supplied identity still passes)
//! - `AccessToken`: a user or appservice token is required
//! - `AccessTokenOptional`: a token is optional
//! - `AppserviceToken` / `AppserviceTokenOptional`: appservice tokens only
//! - `ServerSignatures`: X-Matrix signatures for federation
//!
//! The scheme logic is generic; the server supplies what only it can know
//! through [`MatrixAuth`]: what a token belongs to, and whether a federation
//! request's signatures verify.

use std::future::Future;

use rama::extensions::Extension;
use rama::http::header::AUTHORIZATION;
use rama::http::request::Parts as RequestParts;
use rama::telemetry::tracing::debug;
use ruma::{OwnedDeviceId, OwnedServerName, OwnedUserId, UserId};

use crate::percent;
use crate::rejection::MatrixRejection;

/// The one endpoint whose `access_token` query parameter is not an access
/// token: a remote server presents a user's OpenID token there.
const FEDERATION_OPENID_USERINFO_PATH: &str = "/_matrix/federation/v1/openid/userinfo";

/// Authentication scheme an endpoint requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthScheme {
    None,
    AccessToken,
    AccessTokenOptional,
    AppserviceToken,
    AppserviceTokenOptional,
    ServerSignatures,
}

impl AuthScheme {
    /// The scheme ruma declares for endpoint `T`, read from the name of its
    /// `Authentication` type; `None` for a scheme this crate does not know.
    ///
    /// The name is matched whole (its last path segment), never as a
    /// substring: `NoAccessToken` must not read as `AccessToken`.
    pub fn of<T: ruma::api::IncomingRequest>() -> Option<Self> {
        Self::from_type_name(std::any::type_name::<T::Authentication>())
    }

    /// The scheme named by `type_name`, the full path of a ruma
    /// authentication scheme type.
    fn from_type_name(type_name: &str) -> Option<Self> {
        let name = type_name.rsplit("::").next().unwrap_or(type_name);
        match name {
            // `NoAccessToken`: the endpoint takes no access token at all
            // (login, registration, ...), the scheme those endpoints had
            // as `NoAuthentication` before ruma 0.15.
            "NoAuthentication" | "NoAccessToken" => Some(Self::None),
            "AccessToken" => Some(Self::AccessToken),
            "AccessTokenOptional" => Some(Self::AccessTokenOptional),
            "AppserviceToken" => Some(Self::AppserviceToken),
            "AppserviceTokenOptional" => Some(Self::AppserviceTokenOptional),
            "ServerSignatures" => Some(Self::ServerSignatures),
            _ => None,
        }
    }
}

/// Result of authentication processing
#[derive(Debug, Clone, Default)]
pub struct AuthContext {
    /// Authenticated user ID (for client requests)
    pub sender_user: Option<OwnedUserId>,
    /// Device ID associated with the token
    pub sender_device: Option<OwnedDeviceId>,
    /// Originating server name (for federation requests)
    pub sender_servername: Option<OwnedServerName>,
    /// Whether this is an appservice request
    pub is_appservice: bool,
    /// Appservice ID if applicable
    pub appservice_id: Option<String>,
}

/// What a presented token belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenIdentity {
    /// A live user session.
    User {
        user_id: OwnedUserId,
        device_id: OwnedDeviceId,
    },
    /// An application service's `as_token`.
    Appservice { appservice_id: String },
    /// Nobody: the token is unknown, expired or revoked.
    Invalid {
        /// The session lives on; only this token expired or was superseded.
        soft_logout: bool,
    },
}

/// What the server provides to authenticate Matrix requests.
///
/// Implemented by the state type `S` that [`StateLayer`](crate::StateLayer)
/// injects; [`Matrix<T, S>`](crate::Matrix) and
/// [`RequireUser<S>`](crate::RequireUser) call it.
pub trait MatrixAuth: Extension {
    /// Resolve a token taken from the `Authorization: Bearer` header or the
    /// `access_token` query parameter.
    fn resolve_token(&self, token: &str) -> impl Future<Output = TokenIdentity> + Send;

    /// Verify the X-Matrix signatures of a federation request and return its
    /// origin. `body` is the raw request body the signatures cover.
    ///
    /// The default refuses every server-signed request.
    fn verify_server_signatures(
        &self,
        _parts: &RequestParts,
        _body: &[u8],
    ) -> impl Future<Output = Result<OwnedServerName, MatrixRejection>> + Send {
        async {
            Err(MatrixRejection::unauthorized(
                "Server signatures are not accepted here.",
            ))
        }
    }

    /// Called once per request authenticated by a user access token.
    fn user_authenticated(&self, _user_id: &UserId) {}

    /// The scheme to enforce for endpoint `T`, given the one ruma declares.
    ///
    /// Lets a server ask for more than ruma declares, such as identifying
    /// optional requesters on endpoints ruma marks unauthenticated.
    fn auth_scheme_for<T: 'static>(declared: AuthScheme) -> AuthScheme {
        declared
    }
}

/// The token a request presents: the `Authorization: Bearer` header,
/// otherwise the percent-decoded `access_token` query parameter.
pub fn extract_access_token(parts: &RequestParts) -> Option<String> {
    // Try Authorization header first (preferred)
    if let Some(auth_header) = parts.headers.get(AUTHORIZATION)
        && let Ok(auth_str) = auth_header.to_str()
        && let Some(token) = auth_str.strip_prefix("Bearer ")
    {
        return Some(token.to_string());
    }

    // Fallback to access_token query parameter
    if let Some(query) = parts.uri.query() {
        for pair in query.as_encoded_str().split('&') {
            if let Some((key, value)) = pair.split_once('=')
                && key == "access_token"
            {
                return percent::decode(value)
                    .map(|d| d.into_owned())
                    .ok()
                    .or_else(|| Some(value.to_string()));
            }
        }
    }

    None
}

/// Resolve the token a request presents, if any.
pub async fn identify<S: MatrixAuth>(parts: &RequestParts, state: &S) -> Option<TokenIdentity> {
    let token = extract_access_token(parts)?;
    Some(state.resolve_token(&token).await)
}

/// Authenticate a request under the scheme its endpoint requires.
///
/// `body` is the raw request body, covered by federation request
/// signatures.
pub async fn handle_auth<S: MatrixAuth>(
    parts: &RequestParts,
    scheme: AuthScheme,
    state: &S,
    body: &[u8],
) -> Result<AuthContext, MatrixRejection> {
    // Server-signed requests carry X-Matrix credentials, not tokens
    let token = if scheme != AuthScheme::ServerSignatures {
        identify(parts, state).await
    } else {
        None
    };

    match (scheme, token) {
        // ====================================================================
        // INVALID TOKEN (applies to most schemes)
        // ====================================================================
        // The federation userinfo endpoint's `access_token` query parameter
        // is an OpenID token, not an access token; its handler redeems it.
        (AuthScheme::None, Some(TokenIdentity::Invalid { .. }))
            if parts.uri.path_or_root() == FEDERATION_OPENID_USERINFO_PATH =>
        {
            Ok(AuthContext::default())
        }
        (_, Some(TokenIdentity::Invalid { soft_logout })) => {
            Err(MatrixRejection::UnknownToken { soft_logout })
        }

        // ====================================================================
        // USER ACCESS TOKENS
        // ====================================================================
        // Endpoints without auth still pass a supplied identity through.
        (
            AuthScheme::AccessToken | AuthScheme::AccessTokenOptional | AuthScheme::None,
            Some(TokenIdentity::User { user_id, device_id }),
        ) => {
            debug!("Authenticated user {} with device {}", user_id, device_id);
            state.user_authenticated(&user_id);
            Ok(AuthContext {
                sender_user: Some(user_id),
                sender_device: Some(device_id),
                ..Default::default()
            })
        }
        (
            AuthScheme::AppserviceToken | AuthScheme::AppserviceTokenOptional,
            Some(TokenIdentity::User { .. }),
        ) => Err(MatrixRejection::Forbidden(
            "Only appservice access tokens should be used on this endpoint.".to_owned(),
        )),

        // ====================================================================
        // APPSERVICE TOKENS
        // ====================================================================
        // Appservices can act on behalf of users via the `user_id` query param.
        (
            AuthScheme::AccessToken
            | AuthScheme::AccessTokenOptional
            | AuthScheme::AppserviceToken
            | AuthScheme::AppserviceTokenOptional,
            Some(TokenIdentity::Appservice { appservice_id }),
        ) => {
            let user_id = appservice_user(parts, &appservice_id)?;
            Ok(AuthContext {
                sender_user: user_id,
                sender_device: None,
                is_appservice: true,
                appservice_id: Some(appservice_id),
                ..Default::default()
            })
        }
        (AuthScheme::None, Some(TokenIdentity::Appservice { appservice_id })) => Ok(AuthContext {
            sender_user: None,
            sender_device: None,
            is_appservice: true,
            appservice_id: Some(appservice_id),
            ..Default::default()
        }),

        // ====================================================================
        // NO TOKEN
        // ====================================================================
        (AuthScheme::AccessToken | AuthScheme::AppserviceToken, None) => {
            Err(MatrixRejection::MissingToken)
        }
        (
            AuthScheme::AccessTokenOptional
            | AuthScheme::AppserviceTokenOptional
            | AuthScheme::None,
            None,
        ) => Ok(AuthContext::default()),

        // ====================================================================
        // SERVER SIGNATURES (Federation)
        // ====================================================================
        (AuthScheme::ServerSignatures, None) => {
            let origin = state.verify_server_signatures(parts, body).await?;
            Ok(AuthContext {
                sender_servername: Some(origin),
                ..Default::default()
            })
        }
        (AuthScheme::ServerSignatures, Some(_)) => Err(MatrixRejection::Forbidden(
            "Only server signatures should be used on this endpoint.".to_owned(),
        )),
    }
}

/// The user an appservice acts as, from the `user_id` query parameter.
///
/// Without one the request acts as the appservice itself (`None`).
fn appservice_user(
    parts: &RequestParts,
    appservice_id: &str,
) -> Result<Option<OwnedUserId>, MatrixRejection> {
    let user_id_str: Option<String> = parts.uri.query().and_then(|query| {
        for pair in query.as_encoded_str().split('&') {
            if let Some((key, value)) = pair.split_once('=')
                && key == "user_id"
                && let Ok(decoded) = percent::decode(value)
            {
                return Some(decoded.into_owned());
            }
        }
        None
    });

    match user_id_str {
        Some(uid_str) => {
            let user_id: OwnedUserId = uid_str
                .parse()
                .map_err(|_| MatrixRejection::Forbidden("Invalid user_id format".to_owned()))?;

            // The appservice is trusted with any user ID: namespace and
            // existence checks are left to the handlers.
            debug!("Appservice {} acting as user {}", appservice_id, user_id);
            Ok(Some(user_id))
        }
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests;
