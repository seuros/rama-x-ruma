use super::*;

use rama::http::service::web::response::IntoResponse;

#[test]
fn test_ext_state_missing_display() {
    let err = ExtStateMissing;
    assert!(err.to_string().contains("StateLayer"));
}

#[derive(Debug)]
struct Unit;
impl Extension for Unit {}

#[tokio::test]
async fn missing_state_is_a_500() {
    let (parts, ()) = rama::http::Request::builder()
        .uri("/")
        .body(())
        .expect("request")
        .into_parts();
    let Err(err) = ExtState::<Unit>::from_parts_state_ref_pair(&parts, &()).await else {
        panic!("no StateLayer, no state");
    };
    assert_eq!(
        err.into_response().status(),
        rama::http::StatusCode::INTERNAL_SERVER_ERROR
    );
}
