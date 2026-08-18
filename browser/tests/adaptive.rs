use std::time::{SystemTime, UNIX_EPOCH};

use browser::scrapling::{adaptive, dispatch_op};
use rusqlite::Connection;
use serde_json::{json, Value};

fn call(id: &str, payload: Value) -> Value {
    dispatch_op(id, &payload).unwrap()
}

#[test]
fn adaptive_tracking_matches_the_standalone_wrapper_contract() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir =
        std::env::temp_dir().join(format!("browser-adaptive-{}-{unique}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("elements.db");
    adaptive::configure(&db).unwrap();

    let domain = "https://shop.example.com";
    let v1 =
        r#"<html><body><div class="price-box"><span class="amount">$42</span></div></body></html>"#;
    let v2 = r#"<html><body><section class="pricing"><span class="new-amount">$42</span></section></body></html>"#;

    assert_eq!(
        call(
            "browser::css",
            json!({"html": v1, "query": "span.amount", "adaptive": true,
                   "adaptive_domain": domain, "identifier": "css-price"}),
        ),
        json!({"result": ["$42"]})
    );
    assert_eq!(
        call(
            "browser::css",
            json!({"html": v2, "query": "span.amount", "adaptive": true,
                   "adaptive_domain": domain, "identifier": "css-price"}),
        ),
        json!({"result": ["$42"]})
    );

    assert_eq!(
        call(
            "browser::xpath",
            json!({"html": v1, "query": "//span[@class='amount']", "adaptive": true,
                   "adaptive_domain": domain, "identifier": "xpath-price"}),
        ),
        json!({"result": ["$42"]})
    );
    assert_eq!(
        call(
            "browser::xpath",
            json!({"html": v2, "query": "//span[@class='amount']", "adaptive": true,
                   "adaptive_domain": domain, "identifier": "xpath-price"}),
        ),
        json!({"result": ["$42"]})
    );

    let ties = r#"<html><body><section><span class="new">$42</span><span class="new">$42</span></section></body></html>"#;
    call(
        "browser::css",
        json!({"html": v1, "query": "span.amount", "adaptive": true,
               "adaptive_domain": domain, "identifier": "ties"}),
    );
    assert_eq!(
        call(
            "browser::css",
            json!({"html": ties, "query": "span.amount", "adaptive": true,
                   "adaptive_domain": domain, "identifier": "ties"}),
        ),
        json!({"result": ["$42", "$42"]})
    );

    let grouped = r#"<html><body><p class="a">A</p><p class="b">B</p></body></html>"#;
    assert_eq!(
        call(
            "browser::css",
            json!({"html": grouped, "query": "p.b, p.a", "adaptive": true,
                   "adaptive_domain": domain, "identifier": "grouped"}),
        ),
        json!({"result": ["B", "A"]})
    );

    // The wrapper evaluates adaptive comma groups in selector order and uses
    // the caller's identifier for every group. Each direct hit overwrites the
    // preceding group's identity, so the last group is the persisted one.
    assert_eq!(
        call(
            "browser::css",
            json!({"html": "<p class='changed-a'>A</p><p class='changed-b'>B</p>",
                   "query": "p.b, p.a", "adaptive": true,
                   "adaptive_domain": domain, "identifier": "grouped"}),
        ),
        json!({"result": ["A", "A"]})
    );

    // Direct results always win over a stored identity, and only the first
    // direct match is saved.
    call(
        "browser::css",
        json!({"html": "<p class='many'>first</p><p class='many'>second</p>",
               "query": "p.many", "adaptive": true,
               "adaptive_domain": domain, "identifier": "first-only"}),
    );
    let connection = Connection::open(&db).unwrap();
    let first_only: Vec<u8> = connection
        .query_row(
            "SELECT element_data FROM storage WHERE identifier = 'first-only'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&first_only).unwrap()["text"],
        "first"
    );
    drop(connection);

    // `auto_save: null` is false in the Python wrapper, while an omitted
    // value defaults true for css/xpath and to `adaptive` for extract.
    call(
        "browser::css",
        json!({"html": v1, "query": "span.amount", "adaptive": true, "auto_save": null,
               "adaptive_domain": domain, "identifier": "null-not-saved"}),
    );
    assert_eq!(
        call(
            "browser::css",
            json!({"html": v2, "query": "span.amount", "adaptive": true,
                   "adaptive_domain": domain, "identifier": "null-not-saved"}),
        ),
        json!({"result": []})
    );

    call(
        "browser::css",
        json!({"html": v1, "query": "span.amount", "adaptive": true, "auto_save": false,
               "adaptive_domain": domain, "identifier": "not-saved"}),
    );
    assert_eq!(
        call(
            "browser::css",
            json!({"html": v2, "query": "span.amount", "adaptive": true,
                   "adaptive_domain": domain, "identifier": "not-saved"}),
        ),
        json!({"result": []})
    );

    let selectors = json!([{"name": "price", "css": "span.amount"}]);
    call(
        "browser::extract",
        json!({"html": v1, "selectors": selectors, "adaptive": true, "adaptive_domain": domain}),
    );
    assert_eq!(
        call(
            "browser::extract",
            json!({"html": v2, "selectors": selectors, "adaptive": true, "adaptive_domain": domain}),
        ),
        json!({"extracted": {"price": "$42"}})
    );

    call(
        "browser::extract",
        json!({"html": v1, "selectors": [{"name": "extract-null", "css": "span.amount"}],
               "adaptive": true, "auto_save": null, "adaptive_domain": domain}),
    );
    assert_eq!(
        call(
            "browser::extract",
            json!({"html": v2, "selectors": [{"name": "extract-null", "css": "span.amount"}],
                   "adaptive": true, "adaptive_domain": domain}),
        ),
        json!({"extracted": {"extract-null": null}})
    );

    // These domains distinguish tld 0.13.2's frozen data from the newer PSL
    // bundled by the Rust psl crate. `file.core.windows.net` was a suffix in
    // the oracle, while `12chars.dev` was not yet one.
    call(
        "browser::css",
        json!({"html": v1, "query": "span.amount", "adaptive": true,
               "adaptive_domain": "https://a.file.core.windows.net", "identifier": "added-rule"}),
    );
    assert_eq!(
        call(
            "browser::css",
            json!({"html": v2, "query": "span.amount", "adaptive": true,
                   "adaptive_domain": "https://b.file.core.windows.net", "identifier": "added-rule"}),
        ),
        json!({"result": []})
    );
    call(
        "browser::css",
        json!({"html": v1, "query": "span.amount", "adaptive": true,
               "adaptive_domain": "https://a.12chars.dev", "identifier": "removed-rule"}),
    );
    assert_eq!(
        call(
            "browser::css",
            json!({"html": v2, "query": "span.amount", "adaptive": true,
                   "adaptive_domain": "https://b.12chars.dev", "identifier": "removed-rule"}),
        ),
        json!({"result": ["$42"]})
    );

    let connection = Connection::open(&db).unwrap();
    assert_eq!(
        connection
            .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT typeof(element_data) FROM storage WHERE identifier = 'css-price'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "blob"
    );
    connection
        .execute(
            "UPDATE storage SET element_data = CAST(element_data AS TEXT) WHERE identifier = 'css-price'",
            [],
        )
        .unwrap();
    drop(connection);
    assert_eq!(
        call(
            "browser::css",
            json!({"html": v2, "query": "span.amount", "adaptive": true,
                   "adaptive_domain": domain, "identifier": "css-price"}),
        ),
        json!({"result": ["$42"]})
    );

    std::fs::remove_dir_all(dir).unwrap();
}
