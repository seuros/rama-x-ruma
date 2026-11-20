// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2025, Abdelkader Boudih

//! What ruma needs from the client to build a request: the auth scheme's
//! input and the path builder's input, chosen per endpoint type.

use std::borrow::Cow;
use std::fmt;

use ruma::api::SupportedVersions;
use ruma::api::auth_scheme::{
    AccessToken, AccessTokenOptional, AppserviceToken, AppserviceTokenOptional, AuthScheme,
    NoAccessToken, NoAuthentication, SendAccessToken,
};
use ruma::api::path_builder::{PathBuilder, SinglePath, VersionHistory};

/// The token a [`MatrixClient`](super::MatrixClient) authenticates with.
#[derive(Clone, Default, PartialEq, Eq)]
pub enum Credentials {
    /// No token; endpoints that require one fail to build.
    #[default]
    None,
    /// A user's access token, sent where the endpoint requires one.
    AccessToken(String),
    /// An appservice's `as_token`, also sent on appservice-optional endpoints.
    Appservice(String),
}

impl Credentials {
    /// The token ruma adds per the endpoint's scheme.
    pub fn send_access_token(&self) -> SendAccessToken<'_> {
        match self {
            Self::None => SendAccessToken::None,
            Self::AccessToken(token) => SendAccessToken::IfRequired(token),
            Self::Appservice(token) => SendAccessToken::Appservice(token),
        }
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // never print the token
        f.write_str(match self {
            Self::None => "None",
            Self::AccessToken(_) => "AccessToken(<redacted>)",
            Self::Appservice(_) => "Appservice(<redacted>)",
        })
    }
}

/// An auth scheme the client can satisfy from its [`Credentials`].
///
/// Implemented for every client-facing scheme in ruma; implement it for a
/// custom scheme to send its endpoints.
pub trait OutgoingAuth: AuthScheme {
    /// The scheme's input for `credentials`.
    fn input(credentials: &Credentials) -> Self::Input<'_>;
}

impl OutgoingAuth for NoAuthentication {
    fn input(_credentials: &Credentials) -> Self::Input<'_> {}
}

macro_rules! send_access_token_schemes {
    ($($scheme:ty),+ $(,)?) => {$(
        impl OutgoingAuth for $scheme {
            fn input(credentials: &Credentials) -> Self::Input<'_> {
                credentials.send_access_token()
            }
        }
    )+};
}

send_access_token_schemes!(
    NoAccessToken,
    AccessToken,
    AccessTokenOptional,
    AppserviceToken,
    AppserviceTokenOptional,
);

/// A path builder the client can feed from the homeserver's
/// [`SupportedVersions`].
pub trait OutgoingPath: PathBuilder {
    /// The builder's input for `versions`.
    fn input(versions: Cow<'_, SupportedVersions>) -> Self::Input<'_>;
}

impl OutgoingPath for SinglePath {
    fn input(_versions: Cow<'_, SupportedVersions>) -> Self::Input<'_> {}
}

impl OutgoingPath for VersionHistory {
    fn input(versions: Cow<'_, SupportedVersions>) -> Self::Input<'_> {
        versions
    }
}
