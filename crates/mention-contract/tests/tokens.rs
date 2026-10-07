//! Runs the shared grammar fixtures (`fixtures/tokens.json`), which the
//! console's TypeScript parser runs too.

use mention_contract::token::MAX_ID_LEN;
use mention_contract::{format_mention, parse_mentions, parse_prose_mentions, MentionRef};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixtures {
    parse: Vec<ParseCase>,
    format: Vec<FormatCase>,
}

#[derive(Deserialize)]
struct ParseCase {
    name: String,
    text: String,
    #[serde(default)]
    prose: bool,
    mentions: Vec<Expected>,
}

#[derive(Deserialize)]
struct Expected {
    name: String,
    id: String,
    token: String,
}

#[derive(Deserialize)]
struct FormatCase {
    name: String,
    id: String,
    token: String,
}

fn fixtures() -> Fixtures {
    let raw = include_str!("../fixtures/tokens.json");
    serde_json::from_str(raw).expect("fixtures parse")
}

#[test]
fn parse_fixtures() {
    for case in fixtures().parse {
        let got = if case.prose {
            parse_prose_mentions(&case.text)
        } else {
            parse_mentions(&case.text)
        };
        let want: Vec<MentionRef> = case
            .mentions
            .into_iter()
            .map(|m| MentionRef {
                name: m.name,
                id: m.id,
                token: m.token,
            })
            .collect();
        assert_eq!(got, want, "case {:?}", case.name);
    }
}

#[test]
fn format_fixtures() {
    for case in fixtures().format {
        assert_eq!(format_mention(&case.name, &case.id), case.token);
        // A formatted token always parses back to the same item.
        let parsed = parse_mentions(&case.token);
        assert_eq!(parsed.len(), 1, "token {:?}", case.token);
        assert_eq!(parsed[0].name, case.name);
        assert_eq!(parsed[0].id, case.id);
    }
}

#[test]
fn id_length_is_bounded() {
    let longest = "a".repeat(MAX_ID_LEN);
    assert_eq!(parse_mentions(&format_mention("x", &longest)).len(), 1);
    let too_long = "a".repeat(MAX_ID_LEN + 1);
    assert!(parse_mentions(&format_mention("x", &too_long)).is_empty());
}

#[test]
fn name_length_is_bounded() {
    let longest = "a".repeat(40);
    assert_eq!(parse_mentions(&format_mention(&longest, "1")).len(), 1);
    let too_long = "a".repeat(41);
    assert!(parse_mentions(&format_mention(&too_long, "1")).is_empty());
}

#[test]
fn a_failed_token_does_not_hide_the_next_one() {
    let text = r#"@kanban(id="open @kanban(id="2")"#;
    // The first `@` starts a body that closes at the inner quote and is not
    // followed by `")`; the second `@` is still found.
    let found = parse_mentions(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "2");
}
