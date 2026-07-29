//! Public-contract tests. CI's validate_worker.py requires a non-empty
//! tests/ suite for source-changed workers; the deep edge-case coverage
//! lives in the unit tests under src/.

use fp::{pipe, util};
use serde_json::json;

#[test]
fn transforms_cover_the_lodash_surface() {
    assert_eq!(
        util::get(&json!({"a": {"b": 1}}), "/a/b").unwrap(),
        json!(1)
    );
    assert_eq!(
        util::pick(json!({"a": 1, "b": 2}), &["a".into()]).unwrap(),
        json!({"a": 1})
    );
    assert_eq!(
        util::omit(json!({"a": 1, "b": 2}), &["a".into()]).unwrap(),
        json!({"b": 2})
    );
    assert_eq!(util::take(json!("héllo"), 2).unwrap(), json!("hé"));
    assert_eq!(util::drop(json!([1, 2, 3]), 2).unwrap(), json!([3]));
    assert_eq!(
        util::map(json!([{"id": 1}, {"id": 2}]), "/id").unwrap(),
        json!([1, 2])
    );
    let matches: serde_json::Map<String, serde_json::Value> =
        serde_json::from_value(json!({"s": "x"})).unwrap();
    assert_eq!(
        util::filter(json!([{"s": "x"}, {"s": "y"}]), &matches).unwrap(),
        json!([{"s": "x"}])
    );
    assert_eq!(util::split(json!("a,b"), ",").unwrap(), json!(["a", "b"]));
    assert_eq!(util::join(json!(["a", "b"]), "-").unwrap(), json!("a-b"));
    assert_eq!(util::uniq(json!([1, 1, 2])).unwrap(), json!([1, 2]));
    assert_eq!(util::size(&json!("héllo")).unwrap(), json!(5));
    assert_eq!(util::compact(json!([0, null, ""])).unwrap(), json!([0, ""]));
    assert_eq!(util::nth(json!(["a", "b"]), -1).unwrap(), json!("b"));
    assert_eq!(
        util::get_or(&json!({}), "/x", json!("d")).unwrap(),
        json!("d")
    );
    assert_eq!(util::flatten(json!([1, [2]])).unwrap(), json!([1, 2]));
    assert_eq!(
        util::sort_by(json!([{"s": 2}, {"s": 1}]), "/s").unwrap(),
        json!([{"s": 1}, {"s": 2}])
    );
    assert_eq!(util::reverse(json!([1, 2])).unwrap(), json!([2, 1]));
    assert_eq!(util::sum(json!([1, 2, 3]), None).unwrap(), json!(6));
    assert_eq!(
        util::sum(json!([{"amount": 3}, {"amount": 4}]), Some("/amount")).unwrap(),
        json!(7)
    );
    assert_eq!(util::mean(json!([2, 4, 6]), None).unwrap(), json!(4));
    assert_eq!(util::min(json!([3, 1, 2]), None).unwrap(), json!(1));
    assert_eq!(util::max(json!([3, 1, 2]), None).unwrap(), json!(3));
    assert_eq!(
        util::count_by(json!([{"w": "a"}, {"w": "b"}, {"w": "a"}]), "/w").unwrap(),
        json!({ "a": 2, "b": 1 })
    );
    assert_eq!(
        util::group_by(json!([{"w": "a", "n": 1}, {"w": "b", "n": 2}]), "/w").unwrap(),
        json!({ "a": [{"w": "a", "n": 1}], "b": [{"w": "b", "n": 2}] })
    );

    // misses and type mismatches are teachable errors, never silent garbage
    assert!(util::get(&json!({"a": 1}), "/b").unwrap_err().contains("a"));
    assert!(util::pick(json!([1]), &["a".into()])
        .unwrap_err()
        .contains("object"));
}

#[test]
fn pipe_contract_validates_and_refuses() {
    // The canonical live shape, engine-injected _caller_worker_id included.
    let req: pipe::PipeRequest = serde_json::from_value(json!({
        "through": [
            { "function": "scrapling::fetch", "payload": { "url": "u", "format": "markdown" } },
            { "function": "fp::get", "payload": { "path": "/content" } },
            { "function": "fp::take", "payload": { "n": 6000 } },
            { "function": "state::set", "payload": { "scope": "s", "key": "k" } },
        ],
        "_caller_worker_id": "harness",
    }))
    .expect("live shape parses");
    assert!(pipe::validate(&req).is_ok());

    // Worker-authority-sensitive classes are refused as steps.
    for forbidden in [
        "configuration::get",
        "harness::send",
        "router::chat",
        "session::append",
        "fp::pipe",
    ] {
        let req: pipe::PipeRequest =
            serde_json::from_value(json!({ "through": [{ "function": forbidden }] })).unwrap();
        assert!(pipe::validate(&req).is_err(), "{forbidden}");
    }
}
