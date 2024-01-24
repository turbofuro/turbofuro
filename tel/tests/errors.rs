use tel::TelError;

#[test]
fn test_error_serialization() {
    let result = serde_json::to_string(&TelError::IndexOutOfBounds { index: 2, max: 1 })
        .expect("Could not serialize TelError");

    assert_eq!(
        result,
        r#"{"code":"INDEX_OUT_OF_BOUNDS","index":2,"max":1}"#
    );
}
