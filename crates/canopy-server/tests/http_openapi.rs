use std::collections::BTreeSet;

use serde_json::Value;

#[test]
fn openapi_documents_only_real_canopy_http_routes() {
    let document: Value = serde_json::from_str(include_str!("../../../docs/openapi.json"))
        .expect("docs/openapi.json must be valid JSON");
    assert_eq!(document["openapi"], "3.1.0");

    let paths = document["paths"]
        .as_object()
        .expect("OpenAPI paths must be an object");
    let actual = paths.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = BTreeSet::from([
        "/internal/stream/authorize",
        "/nginx-health",
        "/openapi.json",
        "/stream/{capability}",
    ]);

    assert_eq!(actual, expected);
    assert!(paths.keys().all(|path| !path.starts_with("/canopy.")));
    assert_eq!(
        paths["/internal/stream/authorize"]["get"]["x-internal"],
        true
    );
}
