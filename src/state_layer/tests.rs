use super::*;

use crate::ExtState;
use rama::http::Body;
use rama::http::service::web::Router;

#[test]
fn test_state_layer_clone() {
    // StateLayer is Clone whatever the state type
    fn assert_clone<T: Clone>() {}
    assert_clone::<StateLayer<std::cell::Cell<u8>>>();
}

#[test]
fn test_state_service_clone() {
    // StateService should be Clone when inner is Clone
    fn assert_clone<T: Clone>() {}
    assert_clone::<StateService<(), std::cell::Cell<u8>>>();
}

#[derive(Debug)]
struct Counter(u32);
impl Extension for Counter {}

/// The layer's state reaches `ExtState` in the handler.
#[tokio::test]
async fn injected_state_reaches_ext_state() {
    async fn handler(ExtState(state): ExtState<Counter>) -> String {
        state.0.to_string()
    }

    let router = Router::new().with_get("/count", handler);
    let svc = StateLayer::new(Arc::new(Counter(42))).into_layer(router);
    let request = Request::builder()
        .uri("/count")
        .body(Body::empty())
        .expect("request");
    let response = svc.serve(request).await.expect("routed");
    let body = rama::http::BodyExtractExt::try_into_string(response)
        .await
        .expect("body");
    assert_eq!(body, "42");
}
