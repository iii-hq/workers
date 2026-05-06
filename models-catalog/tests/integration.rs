//! Smoke tests that run without an iii engine connection.

#[test]
fn embedded_list_is_non_empty() {
    let filter = models_catalog::ListFilter::default();
    let models = models_catalog::list(&filter);
    assert!(
        !models.is_empty(),
        "embedded baseline should ship at least one model"
    );
}

#[test]
fn embedded_get_returns_known_model() {
    let filter = models_catalog::ListFilter::default();
    let models = models_catalog::list(&filter);
    let first = models.first().expect("at least one model");
    let got = models_catalog::get(&first.provider, &first.id);
    assert!(got.is_some());
}
