// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2025, Abdelkader Boudih

//! State injection layer
//!
//! This layer injects `Arc<S>` into request extensions, making it available
//! to downstream extractors like [`Matrix<T, S>`](crate::Matrix),
//! [`ExtState<S>`](crate::ExtState) and [`RequireUser<S>`](crate::RequireUser).
//!
//! # Example
//!
//! ```rust,ignore
//! use rama_x_ruma::StateLayer;
//! use std::sync::Arc;
//!
//! let layer = StateLayer::new(Arc::new(state));
//! let service = layer.into_layer(router);
//! ```

use std::sync::Arc;

use rama::extensions::{Extension, ExtensionsRef};
use rama::http::Request;
use rama::{Layer, Service};

/// Layer that injects `Arc<S>` into request extensions.
///
/// Required for any service using the [`Matrix<T, S>`](crate::Matrix)
/// extractor on authenticated endpoints.
pub struct StateLayer<S> {
    state: Arc<S>,
}

impl<S> StateLayer<S> {
    /// Create a new StateLayer with the given state.
    pub fn new(state: Arc<S>) -> Self {
        Self { state }
    }
}

impl<S> Clone for StateLayer<S> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
        }
    }
}

impl<Inner, S> Layer<Inner> for StateLayer<S> {
    type Service = StateService<Inner, S>;

    fn layer(&self, inner: Inner) -> Self::Service {
        StateService {
            inner,
            state: self.state.clone(),
        }
    }
}

/// Service wrapper that injects the state into request extensions.
pub struct StateService<Inner, S> {
    inner: Inner,
    state: Arc<S>,
}

impl<Inner: Clone, S> Clone for StateService<Inner, S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            state: self.state.clone(),
        }
    }
}

impl<Inner, S, Body, Output, Error> Service<Request<Body>> for StateService<Inner, S>
where
    Inner: Service<Request<Body>, Output = Output, Error = Error>,
    S: Extension,
    Body: Send + 'static,
    Output: Send + 'static,
    Error: Send + 'static,
{
    type Output = Output;
    type Error = Error;

    async fn serve(&self, req: Request<Body>) -> Result<Self::Output, Self::Error> {
        req.extensions().insert_arc(self.state.clone());
        self.inner.serve(req).await
    }
}

#[cfg(test)]
mod tests;
