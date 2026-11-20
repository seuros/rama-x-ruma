// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2025, Abdelkader Boudih

//! Send ruma requests through a rama HTTP client.
//!
//! [`MatrixClient`] wraps any rama `Service<Request>` (an
//! `EasyHttpWebClient`, or one behind your own layers) with a homeserver
//! base URL, credentials and the homeserver's supported Matrix versions.
//! [`MatrixClient::send`] turns a ruma `OutgoingRequest` into HTTP, picks the
//! token each endpoint's auth scheme wants, reads the response body under a
//! size limit and parses the typed `IncomingResponse`; Matrix errors come
//! back as ruma's error type inside [`ClientError`].
//!
//! ```text
//! R: OutgoingRequest
//!   -> try_into_http_request   path from SupportedVersions, token from Credentials
//!   -> S: Service<Request>     your rama client stack
//!   -> Limited body collect    max_response_body
//!   -> R::IncomingResponse::try_from_http_response
//! ```

mod error;
mod outgoing;

use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use rama::Service;
use rama::bytes::{Bytes, BytesMut};
use rama::error::BoxError;
use rama::http::body::CollectErrorKind;
use rama::http::body::util::{BodyExt, LengthLimitError, Limited};
use rama::http::convert::hyperium::{TryIntoHyperiumHttp, TryIntoRamaHttp};
use rama::http::header::AUTHORIZATION;
use rama::http::{Body, Request, Response, StreamingBody};
use ruma::api::client::discovery::get_supported_versions;
use ruma::api::{
    IncomingResponseExt, MatrixVersion, OutgoingRequest, OutgoingRequestExt, SupportedVersions,
};

pub use error::{ClientError, MatrixErrorBody};
pub use outgoing::{Credentials, OutgoingAuth, OutgoingPath};

/// Response bodies above this many bytes fail with [`ClientError::BodyTooLarge`].
pub const DEFAULT_MAX_RESPONSE_BODY: usize = 64 * 1024 * 1024;

/// A Matrix client over a rama HTTP service. Cheap to clone when `S` is.
///
/// ```rust,no_run
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// use rama::http::client::DefaultHttpWebClient;
/// use rama_x_ruma::client::MatrixClient;
/// use ruma::api::client::account::whoami::v3 as whoami;
///
/// let client = MatrixClient::new(DefaultHttpWebClient::default(), "https://matrix.example.org")
///     .with_access_token("syt_secret");
/// let me = client.send(whoami::Request::new()).await?;
/// println!("{}", me.user_id);
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct MatrixClient<S> {
    service: S,
    config: Arc<Config>,
}

#[derive(Clone)]
struct Config {
    homeserver: String,
    credentials: Credentials,
    versions: SupportedVersions,
    timeout: Option<Duration>,
    max_response_body: usize,
}

impl<S> fmt::Debug for MatrixClient<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MatrixClient")
            .field("homeserver", &self.config.homeserver)
            .field("credentials", &self.config.credentials)
            .field("versions", &self.config.versions.versions)
            .field("timeout", &self.config.timeout)
            .field("max_response_body", &self.config.max_response_body)
            .finish_non_exhaustive()
    }
}

impl<S> MatrixClient<S> {
    /// A client for the homeserver at `homeserver` (e.g. `https://matrix.org`).
    ///
    /// Starts without credentials, without a timeout, and assuming Matrix
    /// 1.1 (`/v3/` paths); see [`Self::with_supported_versions`].
    pub fn new(service: S, homeserver: impl Into<String>) -> Self {
        let mut homeserver = homeserver.into();
        // ruma appends paths starting with `/`
        while homeserver.ends_with('/') {
            homeserver.pop();
        }
        Self {
            service,
            config: Arc::new(Config {
                homeserver,
                credentials: Credentials::None,
                versions: SupportedVersions {
                    versions: [MatrixVersion::V1_1].into(),
                    features: Default::default(),
                },
                timeout: None,
                max_response_body: DEFAULT_MAX_RESPONSE_BODY,
            }),
        }
    }

    fn config_mut(&mut self) -> &mut Config {
        Arc::make_mut(&mut self.config)
    }

    /// Sends `token` as a user access token where endpoints take one.
    #[must_use]
    pub fn with_access_token(mut self, token: impl Into<String>) -> Self {
        self.config_mut().credentials = Credentials::AccessToken(token.into());
        self
    }

    /// Sends `token` as an appservice token, also on appservice-optional
    /// endpoints such as `/login`.
    #[must_use]
    pub fn with_appservice_token(mut self, token: impl Into<String>) -> Self {
        self.config_mut().credentials = Credentials::Appservice(token.into());
        self
    }

    /// Replaces the credentials outright; [`Credentials::None`] logs out.
    #[must_use]
    pub fn with_credentials(mut self, credentials: Credentials) -> Self {
        self.config_mut().credentials = credentials;
        self
    }

    /// The Matrix versions and features ruma picks endpoint paths from,
    /// usually [`Self::fetch_supported_versions`]'s answer.
    #[must_use]
    pub fn with_supported_versions(mut self, versions: SupportedVersions) -> Self {
        self.config_mut().versions = versions;
        self
    }

    /// Bounds every request (send, response, body) unless a call overrides it.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.config_mut().timeout = Some(timeout);
        self
    }

    /// Caps how many response body bytes are read.
    #[must_use]
    pub fn with_max_response_body(mut self, limit: usize) -> Self {
        self.config_mut().max_response_body = limit;
        self
    }

    /// The homeserver base URL, without a trailing slash.
    pub fn homeserver(&self) -> &str {
        &self.config.homeserver
    }

    /// The credentials requests are sent with.
    pub fn credentials(&self) -> &Credentials {
        &self.config.credentials
    }

    /// The versions endpoint paths are picked from.
    pub fn supported_versions(&self) -> &SupportedVersions {
        &self.config.versions
    }

    /// The default per-request timeout, if any.
    pub fn timeout(&self) -> Option<Duration> {
        self.config.timeout
    }

    /// The response body limit in bytes.
    pub fn max_response_body(&self) -> usize {
        self.config.max_response_body
    }

    /// The wrapped rama service.
    pub fn service(&self) -> &S {
        &self.service
    }
}

impl<S> MatrixClient<S> {
    /// `request` as the rama request the service receives.
    fn build_request<R>(&self, request: R) -> Result<Request<Body>, ClientError<R::EndpointError>>
    where
        R: OutgoingRequest,
        R::Authentication: OutgoingAuth,
        R::PathBuilder: OutgoingPath,
    {
        let config = &*self.config;
        let http_request = request
            .try_into_http_request::<BytesMut>(
                &config.homeserver,
                R::Authentication::input(&config.credentials),
                R::PathBuilder::input(Cow::Borrowed(&config.versions)),
            )
            .map_err(ClientError::IntoHttp)?;
        let mut rama_request = http_request
            .map(|body| Body::from(body.freeze()))
            .try_into_rama_http()
            .map_err(|e| ClientError::Convert(e.into()))?;
        // keep the token out of header-logging layers
        if let Some(value) = rama_request.headers_mut().get_mut(AUTHORIZATION) {
            value.set_sensitive(true);
        }
        Ok(rama_request)
    }
}

impl<S, B> MatrixClient<S>
where
    S: Service<Request<Body>, Output = Response<B>>,
    S::Error: Into<BoxError>,
    B: StreamingBody<Error: Into<BoxError>> + Send,
{
    /// Sends `request` and parses the endpoint's typed response.
    pub async fn send<R>(
        &self,
        request: R,
    ) -> Result<R::IncomingResponse, ClientError<R::EndpointError>>
    where
        R: OutgoingRequest,
        R::Authentication: OutgoingAuth,
        R::PathBuilder: OutgoingPath,
    {
        self.send_inner(request, self.config.timeout).await
    }

    /// Like [`Self::send`], bounded by `timeout` instead of the default; a
    /// long-polling `/sync` wants its own `timeout` plus some slack.
    pub async fn send_with_timeout<R>(
        &self,
        request: R,
        timeout: Duration,
    ) -> Result<R::IncomingResponse, ClientError<R::EndpointError>>
    where
        R: OutgoingRequest,
        R::Authentication: OutgoingAuth,
        R::PathBuilder: OutgoingPath,
    {
        self.send_inner(request, Some(timeout)).await
    }

    /// Asks the homeserver which Matrix versions and features it supports.
    pub async fn fetch_supported_versions(&self) -> Result<SupportedVersions, ClientError> {
        let response = self.send(get_supported_versions::Request::new()).await?;
        Ok(response.as_supported_versions())
    }

    async fn send_inner<R>(
        &self,
        request: R,
        timeout: Option<Duration>,
    ) -> Result<R::IncomingResponse, ClientError<R::EndpointError>>
    where
        R: OutgoingRequest,
        R::Authentication: OutgoingAuth,
        R::PathBuilder: OutgoingPath,
    {
        let rama_request = self.build_request(request)?;

        let exchange = self.exchange(rama_request);
        let (parts, bytes) = match timeout {
            Some(limit) => tokio::time::timeout(limit, exchange)
                .await
                .map_err(|_| ClientError::Timeout(limit))??,
            None => exchange.await?,
        };

        let http_response = parts
            .try_into_hyperium_http()
            .map_err(|e| ClientError::Convert(e.into()))?;
        let status = http_response.status();
        R::IncomingResponse::try_from_http_response(http_response.map(|()| bytes.as_ref()))
            .map_err(|error| ClientError::Response { status, error })
    }

    async fn exchange<E>(
        &self,
        request: Request<Body>,
    ) -> Result<(Response<()>, Bytes), ClientError<E>> {
        let response = self
            .service
            .serve(request)
            .await
            .map_err(|e| ClientError::Transport(e.into()))?;
        let (parts, body) = response.into_parts();
        let limit = self.config.max_response_body;
        let bytes = Limited::new(body, limit)
            .collect()
            .await
            .map_err(|e| match e.kind() {
                CollectErrorKind::Stream(err) if err.is::<LengthLimitError>() => {
                    ClientError::BodyTooLarge { limit }
                }
                _ => ClientError::Transport(Box::new(e)),
            })?
            .to_bytes();
        Ok((Response::from_parts(parts, ()), bytes))
    }
}

#[cfg(test)]
mod tests;
