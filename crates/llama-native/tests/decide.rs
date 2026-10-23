//! The ordered-decision forward over judge-clef's tiny clef GGUF (random
//! weights, byte vocabulary, n_embd 32) on the CPU.
use iii_llama_native::Model;
use std::path::Path;

#[test]
fn decides_finite_deterministic_scores() {
    let gguf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../judge-clef/tests/fixtures/tiny/model.gguf");
    let model = Model::load(&gguf, Some(0), 2, false).expect("tiny GGUF loads");
    assert_eq!(model.device, "CPU");
    assert_eq!(model.architecture().as_deref(), Some("clef"));
    // A choice question (order 2) and its three options (order 4), as bytes.
    let (mut ids, mut orders) = (Vec::new(), Vec::new());
    for (text, order) in [
        ("STATE: the site is down.\nINSTRUCTION: ", 0),
        ("Which team?", 2),
        ("\nOPTION 1: ", 0),
        ("billing", 4),
        ("\nOPTION 2: ", 0),
        ("sales", 4),
        ("\nOPTION 3: ", 0),
        ("support", 4),
        ("\nEND FIELD", 0),
    ] {
        ids.extend(text.bytes().map(i32::from));
        orders.extend(std::iter::repeat_n(order, text.len()));
    }

    let scores = model.decide(&ids, &orders, 3).unwrap();
    assert_eq!(scores.len(), 3);
    assert!(scores.iter().all(|score| score.is_finite()), "{scores:?}");
    assert_eq!(model.decide(&ids, &orders, 3).unwrap(), scores);
    // The shim refuses a score count that does not match the options.
    assert!(model.decide(&ids, &orders, 2).is_err());
}
