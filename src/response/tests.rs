use super::*;

#[test]
fn test_matrix_response_from() {
    struct MockResponse;
    let _response: MatrixResponse<MockResponse> = MockResponse.into();
}

#[test]
fn test_matrix_response_deref() {
    let inner = 42u32;
    let response = MatrixResponse(inner);
    assert_eq!(*response, 42);
}

#[test]
fn test_matrix_response_into_inner() {
    let inner = String::from("test");
    let response = MatrixResponse::new(inner.clone());
    assert_eq!(response.into_inner(), inner);
}

#[test]
fn test_ruma_response_alias() {
    // Verify the backwards compat alias works
    struct MockResponse;
    let _response: RumaResponse<MockResponse> = MockResponse.into();
}
