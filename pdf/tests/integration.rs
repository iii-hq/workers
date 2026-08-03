//! The worker's surface driven over a real engine.
//!
//! These assert at the wire, not in Rust. A response type can be correct and
//! still lose a field on the way out, because serde skips what it is told to
//! skip and a rename never fails to compile. Every assertion here reads the
//! JSON a caller would actually receive.
//!
//! Self-skips when no `iii` binary is available.

mod support;

use serde_json::json;
use support::engine::{fixture_path, with_stack};

#[tokio::test(flavor = "multi_thread")]
async fn classify_answers_over_the_bus() {
    with_stack(|stack| async move {
        let out = stack
            .call(
                "pdf::classify",
                json!({ "path": fixture_path("text-two-page.pdf") }),
            )
            .await
            .expect("pdf::classify");

        assert_eq!(out["document_type"], json!("text_based"));
        assert_eq!(out["page_count"], json!(2));
        assert_eq!(out["source"], json!("text-two-page.pdf"));
        assert_eq!(out["pages_needing_ocr"], json!([]));
        assert!(out["confidence"].is_number());
        assert!(out["elapsed_ms"].is_number());
    })
    .await;
}

/// The per-page OCR verdict is the routing signal, and it has to survive
/// serialization intact — including the reason strings, which the console and
/// the agent guidance both key off.
#[tokio::test(flavor = "multi_thread")]
async fn the_ocr_verdict_reaches_the_caller_with_its_reasons() {
    with_stack(|stack| async move {
        let out = stack
            .call(
                "pdf::classify",
                json!({ "path": fixture_path("no-text.pdf") }),
            )
            .await
            .expect("pdf::classify");

        assert_eq!(out["pages_needing_ocr"], json!([1]));
        let reasons = out["ocr_reasons"].as_array().expect("ocr_reasons array");
        assert_eq!(reasons.len(), 1);
        assert_eq!(reasons[0]["page"], json!(1));
        assert_eq!(reasons[0]["reasons"], json!(["no_text"]));
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn markdown_crosses_the_wire_with_its_body_envelope() {
    with_stack(|stack| async move {
        let out = stack
            .call(
                "pdf::to-markdown",
                json!({ "path": fixture_path("text-two-page.pdf"), "max_chars": 0 }),
            )
            .await
            .expect("pdf::to-markdown");

        let text = out["body"]["text"].as_str().expect("body.text");
        assert!(text.contains("# Quarterly Report"), "{text}");
        assert_eq!(out["body"]["truncated"], json!(false));
        assert_eq!(out["body"]["chars"], out["body"]["total_chars"]);
        // Nothing was withheld, so there is nothing to preview.
        assert!(out["body"].get("preview").is_none());
    })
    .await;
}

/// A truncated body must carry the numbers that let a caller notice. If
/// `total_chars` were dropped on the wire, a fragment would look like a whole
/// document.
#[tokio::test(flavor = "multi_thread")]
async fn a_truncated_body_reports_the_full_size_over_the_wire() {
    with_stack(|stack| async move {
        let out = stack
            .call(
                "pdf::to-markdown",
                json!({ "path": fixture_path("text-two-page.pdf"), "max_chars": 15 }),
            )
            .await
            .expect("pdf::to-markdown");

        assert_eq!(out["body"]["truncated"], json!(true));
        assert_eq!(out["body"]["chars"], json!(15));
        assert!(out["body"]["total_chars"].as_u64().unwrap() > 15);
        assert!(out["body"]["preview"].is_string());
    })
    .await;
}

/// Page numbers are 1-indexed on the wire in both directions. The parser's own
/// per-page entry point counts from zero, so this is the assertion that catches
/// a conversion going missing in a refactor.
#[tokio::test(flavor = "multi_thread")]
async fn page_numbers_stay_one_indexed_across_the_wire() {
    with_stack(|stack| async move {
        let out = stack
            .call(
                "pdf::to-markdown",
                json!({
                    "path": fixture_path("text-two-page.pdf"),
                    "pages": [2],
                    "per_page": true,
                    "max_chars": 0
                }),
            )
            .await
            .expect("pdf::to-markdown");

        let pages = out["pages"].as_array().expect("per-page results");
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0]["page"], json!(2));
        assert!(pages[0]["markdown"].as_str().unwrap().contains("Appendix"));
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn items_carry_their_geometry_and_state_their_origin() {
    with_stack(|stack| async move {
        let out = stack
            .call(
                "pdf::extract-items",
                json!({ "path": fixture_path("text-two-page.pdf"), "pages": [1] }),
            )
            .await
            .expect("pdf::extract-items");

        assert_eq!(out["coordinate_origin"], json!("pdf-points, bottom-left"));
        assert_eq!(out["total_count"], json!(3));
        assert_eq!(out["truncated"], json!(false));

        let items = out["items"].as_array().expect("items array");
        let heading = items
            .iter()
            .find(|i| i["text"].as_str().unwrap_or_default().contains("Quarterly"))
            .expect("the heading survives serialization");
        assert_eq!(heading["page"], json!(1));
        assert_eq!(heading["x"], json!(72.0));
        assert_eq!(heading["y"], json!(700.0));
        assert_eq!(heading["font_size"], json!(24.0));
        assert_eq!(heading["kind"], json!("text"));
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn regions_read_the_box_and_state_the_opposite_origin() {
    with_stack(|stack| async move {
        let out = stack
            .call(
                "pdf::extract-regions",
                json!({
                    "path": fixture_path("text-two-page.pdf"),
                    "regions": [{ "page": 1, "boxes": [[0.0, 0.0, 612.0, 200.0]] }]
                }),
            )
            .await
            .expect("pdf::extract-regions");

        assert_eq!(out["coordinate_origin"], json!("pdf-points, top-left"));
        assert_eq!(out["region_count"], json!(1));
        let pages = out["pages"].as_array().expect("pages array");
        assert_eq!(pages[0]["page"], json!(1));
        assert!(pages[0]["regions"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Quarterly Report"));
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn plain_text_extraction_crosses_the_wire() {
    with_stack(|stack| async move {
        let out = stack
            .call(
                "pdf::extract-text",
                json!({ "path": fixture_path("text-two-page.pdf"), "max_chars": 0 }),
            )
            .await
            .expect("pdf::extract-text");

        let text = out["body"]["text"].as_str().expect("body.text");
        assert!(text.contains("Quarterly Report"));
        assert!(text.contains("Appendix"));
    })
    .await;
}

/// A caller mistake must come back as an error the caller can act on, not as an
/// empty success that reads like a document with nothing in it.
#[tokio::test(flavor = "multi_thread")]
async fn caller_mistakes_come_back_as_errors() {
    with_stack(|stack| async move {
        let err = stack
            .call("pdf::classify", json!({}))
            .await
            .expect_err("no source given");
        assert!(err.contains("path"), "{err}");

        let err = stack
            .call(
                "pdf::extract-items",
                json!({ "path": fixture_path("text-two-page.pdf"), "pages": [0] }),
            )
            .await
            .expect_err("page 0");
        assert!(err.contains("1-indexed"), "{err}");

        let err = stack
            .call(
                "pdf::extract-regions",
                json!({
                    "path": fixture_path("text-two-page.pdf"),
                    "regions": [{ "page": 1, "boxes": [[300.0, 300.0, 10.0, 10.0]] }]
                }),
            )
            .await
            .expect_err("inverted box");
        assert!(err.contains("inverted"), "{err}");
    })
    .await;
}

/// The guidance hook is what makes an agent aware of this worker at all. It has
/// to be registered and callable, and it must preserve a prompt it is handed.
#[tokio::test(flavor = "multi_thread")]
async fn the_guidance_hook_appends_without_replacing() {
    with_stack(|stack| async move {
        let out = stack
            .call(
                "pdf::inject-guidance",
                json!({ "generate": { "system_prompt": "BASE PROMPT" } }),
            )
            .await
            .expect("pdf::inject-guidance");

        let prompt = out["mutations"]["system_prompt"]
            .as_str()
            .expect("a system_prompt mutation");
        assert!(prompt.starts_with("BASE PROMPT\n\n"), "the base was lost");
        assert!(prompt.contains("pdf::classify"));

        // An empty base must yield NO key at all, so the harness keeps its own
        // assembled prompt rather than having it replaced by the guidance.
        let out = stack
            .call("pdf::inject-guidance", json!({ "generate": {} }))
            .await
            .expect("pdf::inject-guidance");
        assert_eq!(out, json!({ "mutations": {} }));
    })
    .await;
}
