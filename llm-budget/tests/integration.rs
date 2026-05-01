//! Smoke tests that run without an iii engine connection.

#[test]
fn period_key_is_stable() {
    let key = llm_budget::period_key(llm_budget::Period::Day, 1_700_000_000_000);
    assert!(!key.is_empty());
}

#[test]
fn period_start_aligns_to_period() {
    let ms = 1_700_000_000_000;
    let start = llm_budget::period_start(llm_budget::Period::Day, ms);
    assert!(start <= ms);
    let next = llm_budget::next_period_start(llm_budget::Period::Day, start);
    assert!(next > start);
}
