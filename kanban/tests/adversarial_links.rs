//! Adversarial tests for `kanban::ticket::links` (`board::ticket_links`,
//! `cited_keys`, `key_number`). Engine-free, like `tests/board.rs`.
//!
//! The last section is the regression suite of the defects the review confirmed
//! (finding number N); the rest pin behaviour that held under attack.
//!
//! Run: `SKIP_UI_BUILD=1 cargo test --test adversarial_links -- --nocapture`
//! Perf (ignored by default): add `-- --ignored`.

use std::collections::BTreeSet;
use std::time::Instant;

use iii_kanban::board::{TicketRow, delete_ticket, find_ticket, restore_ticket, row, ticket_links};
use iii_kanban::config::normalize;
use iii_kanban::functions::TicketLinks;
use iii_kanban::store::{Board, Comment, Ticket};
use serde_json::{Value, json};

/* ── helpers ──────────────────────────────────────────────────────────── */

fn comment(ticket: &str, n: usize, body: &str) -> Comment {
    Comment {
        id: format!("c-{ticket}-{n}"),
        ticket_id: ticket.into(),
        parent_id: None,
        body: body.into(),
        author: "user".into(),
        created_at: "2026-01-01T00:00:00.000Z".into(),
    }
}

/// A ticket built directly (uuid `id-<key>`), so any prefix/number/text is possible.
fn mk(prefix: &str, number: u64, title: &str, description: &str, comments: &[&str]) -> Ticket {
    let key = format!("{prefix}-{number}");
    let id = format!("id-{key}");
    Ticket {
        comments: comments
            .iter()
            .enumerate()
            .map(|(n, body)| comment(&id, n, body))
            .collect(),
        id,
        key,
        title: title.into(),
        description: description.into(),
        status: "backlog".into(),
        priority: "low".into(),
        assignee: None,
        labels: vec![],
        order: number,
        created_at: "2026-01-01T00:00:00.000Z".into(),
        updated_at: "2026-01-01T00:00:00.000Z".into(),
        deleted_at: None,
        created_by: "user".into(),
        activity: vec![],
    }
}

fn board_of(tickets: Vec<Ticket>) -> Board {
    let mut board = Board::empty();
    for ticket in tickets {
        board.tickets.insert(ticket.id.clone(), ticket);
    }
    board
}

fn keys(tickets: Vec<&Ticket>) -> Vec<String> {
    tickets.into_iter().map(|t| t.key.clone()).collect()
}

/// `(mentions, mentioned_by)` of `subject` as key lists.
fn links(board: &Board, subject: &str, prefix: &str) -> (Vec<String>, Vec<String>) {
    let (a, b) = ticket_links(board, subject, prefix).expect("subject exists");
    (keys(a), keys(b))
}

/// Two tickets: KAN-1 (subject) and KAN-2 whose description is `text`.
/// Returns whether KAN-2 counts as mentioning KAN-1.
fn mentioned_in(text: &str) -> bool {
    let board = board_of(vec![
        mk("KAN", 1, "Subject", "", &[]),
        mk("KAN", 2, "Citing", text, &[]),
    ]);
    !links(&board, "KAN-1", "KAN").1.is_empty()
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/* ── independent reference implementation ─────────────────────────────── */

/// Rule, written as a naive per-position scan over `Vec<char>` (no `find`, no
/// byte offsets): `<PREFIX>-<ascii digits>` where neither neighbour is a
/// Unicode letter/digit/combining mark, `/` or `-`.
fn reference_cited(ticket: &Ticket, prefix: &str) -> BTreeSet<String> {
    let mark = |c: char| {
        [
            0x300..=0x36F,
            0x1AB0..=0x1AFF,
            0x1DC0..=0x1DFF,
            0x20D0..=0x20FF,
            0xFE20..=0xFE2F,
        ]
        .iter()
        .any(|range| range.contains(&(c as u32)))
    };
    let word = |c: char| c.is_alphabetic() || c.is_numeric() || mark(c) || c == '/' || c == '-';
    let head: Vec<char> = prefix.chars().collect();
    let mut out = BTreeSet::new();
    let texts = std::iter::once(&ticket.title)
        .chain(std::iter::once(&ticket.description))
        .chain(ticket.comments.iter().map(|c| &c.body));
    for text in texts {
        let cs: Vec<char> = text.chars().collect();
        for i in 0..cs.len() {
            let dash = i + head.len();
            if dash >= cs.len() || cs[dash] != '-' {
                continue;
            }
            if !head
                .iter()
                .enumerate()
                .all(|(k, p)| cs[i + k].eq_ignore_ascii_case(p))
            {
                continue;
            }
            let start = dash + 1;
            let mut end = start;
            while end < cs.len() && cs[end].is_ascii_digit() {
                end += 1;
            }
            if end == start {
                continue;
            }
            let before_ok = i == 0 || !word(cs[i - 1]);
            let after_ok = end == cs.len() || !word(cs[end]);
            if before_ok && after_ok {
                let digits: String = cs[start..end].iter().collect();
                out.insert(format!("{}-{digits}", prefix.to_ascii_lowercase()));
            }
        }
    }
    out
}

fn number(ticket: &Ticket) -> u64 {
    ticket.key.rsplit_once('-').unwrap().1.parse().unwrap()
}

fn reference_links(board: &Board, subject: &Ticket, prefix: &str) -> (Vec<String>, Vec<String>) {
    let cites = reference_cited(subject, prefix);
    let own = subject.key.to_lowercase();
    let live = |t: &&Ticket| t.deleted_at.is_none() && t.id != subject.id;
    let mut mentions: Vec<&Ticket> = board
        .tickets
        .values()
        .filter(live)
        .filter(|t| cites.contains(&t.key.to_lowercase()))
        .collect();
    let mut mentioned_by: Vec<&Ticket> = board
        .tickets
        .values()
        .filter(live)
        .filter(|t| reference_cited(t, prefix).contains(&own))
        .collect();
    mentions.sort_by_key(|t| number(t));
    mentioned_by.sort_by_key(|t| number(t));
    (keys(mentions), keys(mentioned_by))
}

/* ── fuzz: no panic, agreement with the reference ─────────────────────── */

/// `{P}` upper, `{p}` lower, `{m}` mixed-case prefix.
const NOISE: &[&str] = &[
    "{P}-",
    "{p}-",
    "{P}",
    "-",
    "/",
    "_",
    ".",
    ",",
    "(",
    ")",
    " ",
    "\n",
    "\t",
    "\0",
    "x",
    "9",
    "1",
    "K",
    "é",
    "e\u{301}",
    "ß",
    "İ",
    "ı",
    "ǅ",
    "ﬃ",
    "\u{301}",
    "\u{200D}",
    "\u{200B}",
    "\u{202E}",
    "\u{FEFF}",
    "😀",
    "👨\u{200D}👩\u{200D}👧",
    "٣",
    "２",
    "ⓐ",
    "\u{93E}",
    "\u{212A}",
    "ſ",
    "中",
    "ｋａｎ-1",
    "{P}-01",
    "{P}-007",
    "{P}-0",
    "{P}-99999999999999999999999999",
    "{P}-٣",
    "{P}-²",
    "{P}-１",
    "{P}--1",
    "{P}-{P}-1",
    "{P}-1{P}-2",
    "\u{212A}AN-1",
    "AN-1",
    "KA-1",
    "-1",
];

fn render(piece: &str, prefix: &str) -> String {
    let mixed: String = prefix
        .chars()
        .enumerate()
        .map(|(i, c)| {
            if i % 2 == 0 {
                c.to_ascii_lowercase()
            } else {
                c
            }
        })
        .collect();
    piece
        .replace("{P}", prefix)
        .replace("{p}", &prefix.to_ascii_lowercase())
        .replace("{m}", &mixed)
}

fn fuzz_text(rng: &mut Rng, prefix: &str, max_pieces: usize) -> String {
    let mut out = String::new();
    for _ in 0..rng.below(max_pieces + 1) {
        if rng.below(10) < 4 {
            let n = 1 + rng.below(14);
            let variant = ["{P}", "{p}", "{m}"][rng.below(3)];
            out.push_str(&render(&format!("{variant}-{n}"), prefix));
        } else {
            out.push_str(&render(NOISE[rng.below(NOISE.len())], prefix));
        }
    }
    out
}

fn fuzz_board(rng: &mut Rng, prefix: &str, count: u64) -> Board {
    let mut tickets = Vec::new();
    for n in 1..=count {
        let title = fuzz_text(rng, prefix, 6);
        let description = fuzz_text(rng, prefix, 30);
        let bodies: Vec<String> = (0..rng.below(4))
            .map(|_| fuzz_text(rng, prefix, 12))
            .collect();
        let refs: Vec<&str> = bodies.iter().map(String::as_str).collect();
        let mut ticket = mk(prefix, n, &title, &description, &refs);
        if rng.below(6) == 0 {
            ticket.deleted_at = Some("2026-01-02T00:00:00.000Z".into());
        }
        tickets.push(ticket);
    }
    board_of(tickets)
}

#[test]
fn fuzz_unicode_heavy_text_agrees_with_the_reference_and_never_panics() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut mentions_seen = 0usize;
    for prefix in ["KAN", "K2", "A1B", "X", "KANBAN", "Z9"] {
        for _ in 0..250 {
            let board = fuzz_board(&mut rng, prefix, 14);
            for subject in board.tickets.values() {
                let got = links(&board, &subject.id, prefix);
                let want = reference_links(&board, subject, prefix);
                assert_eq!(
                    got,
                    want,
                    "prefix {prefix}, subject {}\n title {:?}\n desc {:?}\n comments {:?}",
                    subject.key,
                    subject.title,
                    subject.description,
                    subject.comments.iter().map(|c| &c.body).collect::<Vec<_>>()
                );
                mentions_seen += got.0.len() + got.1.len();
            }
        }
    }
    assert!(
        mentions_seen > 5_000,
        "the fuzz must actually produce links ({mentions_seen})"
    );
}

#[test]
fn fuzz_by_uuid_and_by_key_resolve_the_same_subject() {
    let mut rng = Rng(0xDEAD_BEEF_CAFE_F00D);
    for _ in 0..100 {
        let board = fuzz_board(&mut rng, "KAN", 8);
        for subject in board.tickets.values() {
            let by_uuid = links(&board, &subject.id, "KAN");
            assert_eq!(by_uuid, links(&board, &subject.key, "KAN"));
            assert_eq!(by_uuid, links(&board, &subject.key.to_lowercase(), "KAN"));
            assert_eq!(
                by_uuid,
                links(&board, &format!("  {}\n", subject.key), "KAN")
            );
        }
    }
}

/* ── targeted attacks that held ───────────────────────────────────────── */

#[test]
fn multibyte_neighbours_do_not_slice_inside_a_char() {
    // 2-, 3- and 4-byte chars right before and after the key; none are letters/digits.
    for (before, after, found) in [
        ("«", "»", true),
        ("“", "”", true),
        ("€", "€", true),
        ("😀", "😀", true),
        ("\u{200D}", "\u{200D}", true),
        ("\u{202E}", "\u{202E}", true),
        ("\0", "\0", true),
        ("é", "", false),
        ("", "é", false),
        ("中", "", false),
        ("", "中", false),
        ("ß", "", false),
        ("", "ı", false),
        ("İ", "", false),
        ("٣", "", false),
        ("", "２", false),
    ] {
        assert_eq!(
            mentioned_in(&format!("{before}KAN-1{after}")),
            found,
            "{before:?} KAN-1 {after:?}"
        );
        assert_eq!(
            mentioned_in(&format!("{before}kan-1{after}")),
            found,
            "{before:?} kan-1 {after:?}"
        );
    }
}

#[test]
fn leading_zeros_never_alias_another_ticket() {
    assert!(!mentioned_in("KAN-01"));
    assert!(!mentioned_in("KAN-001 and KAN-0001"));
    assert!(!mentioned_in("KAN-0"));
    assert!(!mentioned_in("KAN-10 KAN-100 KAN-1x"));
    // ... and a stored key that really is zero-padded is only reachable verbatim.
    let board = board_of(vec![
        mk("KAN", 1, "One", "", &[]),
        mk("KAN", 2, "Two", "see KAN-01 and KAN-1", &[]),
    ]);
    assert_eq!(links(&board, "KAN-2", "KAN").0, ["KAN-1"]);
}

#[test]
fn huge_digit_runs_neither_overflow_nor_panic() {
    let huge = "9".repeat(5_000);
    let text = format!("KAN-{huge} KAN-18446744073709551616 KAN-18446744073709551615 KAN-2");
    let board = board_of(vec![
        mk("KAN", 1, "Subject", &text, &[]),
        mk("KAN", 2, "Two", "", &[]),
        mk("KAN", u64::MAX, "Max", "", &[]),
    ]);
    // KAN-18446744073709551615 is u64::MAX: parsed, sorted last, no overflow.
    assert_eq!(
        links(&board, "KAN-1", "KAN").0,
        ["KAN-2", "KAN-18446744073709551615"]
    );
    // A key past u64 (only reachable by hand-editing board.json) must not panic either.
    let big = board_of(vec![
        mk("KAN", 1, "S", "KAN-18446744073709551616", &[]),
        mk("KAN", 2, "T", "", &[]),
    ]);
    let mut odd = big.clone();
    odd.tickets.values_mut().nth(1).unwrap().key = "KAN-18446744073709551616".into();
    assert_eq!(links(&odd, "KAN-1", "KAN").0, ["KAN-18446744073709551616"]);
}

#[test]
fn prefix_with_digits_or_regex_metacharacters_is_matched_literally() {
    let board = board_of(vec![
        mk("A1", 1, "S", "", &[]),
        mk("A1", 2, "T", "a1-1 and A1-1x, 1A1-1, A11-1, A1-1", &[]),
        mk("A1", 3, "U", "(A1-1)", &[]),
    ]);
    assert_eq!(links(&board, "A1-1", "A1").1, ["A1-2", "A1-3"]);
    // config.rs never lets these through, but board::ticket_links is pub: no regex semantics.
    for hostile in [".*", "A|B", "(", "[", "\\", "A.", "", "-", "Ünï", "İ"] {
        let board = board_of(vec![
            mk("KAN", 1, "S", "", &[]),
            // No KAN-1 here: the board's own prefixes are always scanned.
            mk("KAN", 2, "T", "ABC-1 A-1 B-1 Ünï-1 i̇-1", &[]),
        ]);
        let (mentions, mentioned_by) = links(&board, "KAN-1", hostile);
        assert!(
            mentions.is_empty() && mentioned_by.is_empty(),
            "prefix {hostile:?}"
        );
    }
    for bad in ["a.b", "A+", "A B", "A_B", "1AB", "", "ÀB", "A-B"] {
        assert_eq!(
            normalize(&json!({ "id_prefix": bad })).id_prefix,
            "KAN",
            "{bad:?}"
        );
    }
    assert_eq!(normalize(&json!({ "id_prefix": " b2c " })).id_prefix, "B2C");
}

#[test]
fn a_stored_key_in_another_case_still_resolves_both_ways() {
    let mut board = board_of(vec![mk("KAN", 7, "Seven", "see Kan-8 and KAN-9", &[])]);
    board.tickets.values_mut().next().unwrap().key = "kan-7".into();
    board.tickets.insert("id-8".into(), {
        let mut t = mk("KAN", 8, "Eight", "back to KAN-7", &[]);
        t.id = "id-8".into();
        t.key = "Kan-8".into();
        t
    });
    assert_eq!(
        links(&board, "KAN-7", "KAN"),
        (vec!["Kan-8".to_string()], vec!["Kan-8".to_string()])
    );
    assert_eq!(links(&board, "KAN-8", "KAN").0, ["kan-7"]);
}

#[test]
fn self_mention_in_any_case_and_any_field_is_excluded() {
    let board = board_of(vec![
        mk("KAN", 5, "KAN-5", "kan-5 Kan-5", &["kAn-5", "see KAN-5"]),
        mk("KAN", 6, "other", "", &[]),
    ]);
    assert_eq!(links(&board, "KAN-5", "KAN"), (vec![], vec![]));
    assert_eq!(links(&board, "kan-5", "KAN"), (vec![], vec![]));
}

#[test]
fn deleted_tickets_never_appear_but_restore_brings_them_back() {
    let mut board = board_of(vec![
        mk("KAN", 1, "Subject", "cites KAN-2 and KAN-3", &[]),
        mk("KAN", 2, "Deleted, cites KAN-1", "", &[]),
        mk("KAN", 3, "Live", "", &["KAN-1 in a comment"]),
    ]);
    delete_ticket(&mut board, "KAN-2", "user", "2026-01-03T00:00:00.000Z").unwrap();
    assert_eq!(
        links(&board, "KAN-1", "KAN"),
        (vec!["KAN-3".to_string()], vec!["KAN-3".to_string()])
    );
    restore_ticket(&mut board, "KAN-2", "user", "2026-01-04T00:00:00.000Z").unwrap();
    assert_eq!(
        links(&board, "KAN-1", "KAN"),
        (
            vec!["KAN-2".to_string(), "KAN-3".to_string()],
            vec!["KAN-2".to_string(), "KAN-3".to_string()]
        )
    );
}

#[test]
fn a_deleted_subject_resolves_like_ticket_get_and_still_reports_its_links() {
    // `kanban::ticket::get` returns deleted tickets (find_ticket does not filter),
    // so `links` must not turn that into "not found" either.
    let mut board = board_of(vec![
        mk("KAN", 1, "S", "KAN-2", &[]),
        mk("KAN", 2, "T", "KAN-1", &[]),
    ]);
    delete_ticket(&mut board, "KAN-1", "user", "2026-01-03T00:00:00.000Z").unwrap();
    assert!(find_ticket(&board, "KAN-1").is_some());
    assert_eq!(
        links(&board, "KAN-1", "KAN"),
        (vec!["KAN-2".to_string()], vec!["KAN-2".to_string()])
    );
    for missing in ["KAN-404", "", "   ", "kan-", "id-nope"] {
        assert_eq!(
            ticket_links(&board, missing, "KAN").is_none(),
            find_ticket(&board, missing).is_none(),
            "{missing:?}"
        );
    }
}

#[test]
fn cycles_and_cliques_terminate_with_each_ticket_once() {
    let n = 30u64;
    let all: String = (1..=n).map(|i| format!("KAN-{i} ")).collect();
    let tickets: Vec<Ticket> = (1..=n)
        .map(|i| mk("KAN", i, &all, &all, &[&all, &all]))
        .collect();
    let board = board_of(tickets);
    let want: Vec<String> = (1..=n)
        .filter(|i| *i != 7)
        .map(|i| format!("KAN-{i}"))
        .collect();
    assert_eq!(links(&board, "KAN-7", "KAN"), (want.clone(), want));
    // A ring A -> B -> C -> A is asymmetric.
    let ring = board_of(vec![
        mk("KAN", 1, "", "KAN-2", &[]),
        mk("KAN", 2, "", "", &["KAN-3"]),
        mk("KAN", 3, "KAN-1", "", &[]),
    ]);
    assert_eq!(
        links(&ring, "KAN-1", "KAN"),
        (vec!["KAN-2".to_string()], vec!["KAN-3".to_string()])
    );
}

#[test]
fn results_are_ordered_by_number_not_by_insertion_or_text() {
    let mut tickets = vec![
        mk("KAN", 100, "", "", &[]),
        mk("KAN", 9, "", "", &[]),
        mk("KAN", 10, "", "", &[]),
    ];
    tickets.push(mk("KAN", 2, "KAN-100 KAN-10 KAN-9 KAN-2", "", &[]));
    tickets.push(mk("KAN", 1, "", "", &[]));
    let board = board_of(tickets);
    assert_eq!(
        links(&board, "KAN-2", "KAN").0,
        ["KAN-9", "KAN-10", "KAN-100"]
    );
    assert_eq!(links(&board, "KAN-9", "KAN").1, ["KAN-2"]);
}

#[test]
fn pathological_long_texts_stay_linear() {
    let started = Instant::now();
    let megabyte = 1 << 20;
    for text in [
        "kan-".repeat(megabyte / 4),
        "KAN-1 ".repeat(megabyte / 6),
        format!("KAN-{}", "1".repeat(megabyte)),
        "\0".repeat(megabyte),
        "😀KAN-1".repeat(megabyte / 8),
        "KAN-".repeat(megabyte / 4) + "1",
        "-".repeat(megabyte) + "KAN-1",
    ] {
        let board = board_of(vec![
            mk("KAN", 1, "S", "", &[]),
            mk("KAN", 2, "T", &text, &[&text]),
        ]);
        let _ = links(&board, "KAN-1", "KAN");
        let _ = links(&board, "KAN-2", "KAN");
    }
    println!("pathological 1 MB texts x7: {:?}", started.elapsed());
    assert!(started.elapsed().as_secs() < 30);
}

/* ── response shape ───────────────────────────────────────────────────── */

#[test]
fn rows_serialize_exactly_like_the_ui_ticket_summary() {
    // kanban/ui/src/shared.tsx `TicketSummary`.
    let expected: BTreeSet<&str> = [
        "id",
        "key",
        "title",
        "status",
        "priority",
        "assignee",
        "labels",
        "order",
        "created_at",
        "updated_at",
        "deleted_at",
        "comment_count",
    ]
    .into_iter()
    .collect();
    let mut cited = mk("KAN", 2, "Two", "", &["a", "b"]);
    cited.assignee = Some("tech-lead".into());
    cited.labels = vec!["bug".into()];
    let board = board_of(vec![mk("KAN", 1, "One", "KAN-2", &[]), cited]);
    let (mentions, mentioned_by) = ticket_links(&board, "KAN-1", "KAN").unwrap();
    let payload = serde_json::to_value(TicketLinks {
        mentions: mentions.into_iter().map(|t| row(t, false)).collect(),
        mentioned_by: mentioned_by.into_iter().map(|t| row(t, false)).collect(),
    })
    .unwrap();
    assert_eq!(payload["mentioned_by"], json!([]));
    let item = payload["mentions"][0].as_object().unwrap();
    assert_eq!(
        item.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        expected
    );
    assert_eq!(item["comment_count"], json!(2));
    assert_eq!(item["assignee"], json!("tech-lead"));
    assert_eq!(item["deleted_at"], Value::Null);
    assert!(item["order"].is_u64() && item["labels"].is_array());
    assert!(matches!(
        row(find_ticket(&board, "KAN-2").unwrap(), false),
        TicketRow::Summary(_)
    ));
    let schema = serde_json::to_value(schemars::schema_for!(TicketLinks)).unwrap();
    assert_eq!(schema["required"], json!(["mentioned_by", "mentions"]));
}

/* ── regressions of confirmed defects ────────────────────────────────── */

// #1 — keys already handed out keep the prefix they were numbered with when
// `id_prefix` is edited, so the scanner reads every prefix on the board plus
// the configured one; otherwise every cross-reference to a pre-change ticket
// disappears, in both directions.
#[test]
fn links_survive_an_id_prefix_change() {
    let board = board_of(vec![
        mk("KAN", 1, "Old subject", "see KAN-2", &[]),
        mk("KAN", 2, "Old", "", &[]),
        mk("PROJ", 3, "New, cites KAN-1", "", &[]),
    ]);
    // Config now says PROJ.
    let (mentions, mentioned_by) = links(&board, "KAN-1", "PROJ");
    assert_eq!(
        mentions,
        ["KAN-2"],
        "KAN-1 -> KAN-2 is lost once the prefix is PROJ"
    );
    assert_eq!(
        mentioned_by,
        ["PROJ-3"],
        "PROJ-3 -> KAN-1 is lost once the prefix is PROJ"
    );
    assert_eq!(links(&board, "PROJ-3", "PROJ").0, ["KAN-1"]);
}

#[test]
fn a_prefix_scan_does_not_double_count_or_cross_prefixes() {
    // `A-1` sits inside `BA-1`: the `A-` head must not claim it (the `BA` head does).
    let board = board_of(vec![
        mk("A", 1, "Subject", "", &[]),
        mk("BA", 1, "Other", "", &[]),
        mk("BA", 2, "Citing", "BA-1 and A-1, again BA-1", &[]),
    ]);
    assert_eq!(links(&board, "BA-1", "A").1, ["BA-2"]);
    assert_eq!(links(&board, "A-1", "A").1, ["BA-2"]);
    assert_eq!(links(&board, "BA-2", "A").0, ["A-1", "BA-1"]);
}

// #2 — the same visible text gave two answers depending on Unicode
// normalization: precomposed `ç` blocks the key, but `c` + U+0327 (NFD, what
// macOS pasteboards/filenames produce) did not, because combining marks are
// neither letters nor digits.
#[test]
fn nfd_and_nfc_spellings_of_the_same_word_behave_alike() {
    assert!(!mentioned_in("\u{e7}KAN-1"), "precomposed: blocked (held)");
    assert!(!mentioned_in("c\u{327}KAN-1"), "NFD before the key");
    assert!(
        !mentioned_in("KAN-1\u{301}"),
        "a digit with a combining accent is not the key"
    );
    // One mark from each range the scanner lists.
    for mark in [
        '\u{300}', '\u{36F}', '\u{1AB0}', '\u{1DC0}', '\u{20D0}', '\u{FE20}',
    ] {
        assert!(
            !mentioned_in(&format!("{mark}KAN-1")),
            "U+{:04X} before",
            mark as u32
        );
        assert!(
            !mentioned_in(&format!("KAN-1{mark}")),
            "U+{:04X} after",
            mark as u32
        );
    }
}

// #3 — the agreed rule: a neighbour blocks the key when Rust's
// `char::is_alphanumeric` (Alphabetic or Numeric) says so, which is the UI's
// `\p{Alphabetic}|\p{N}`, plus the combining marks above. That includes the
// Other_Alphabetic symbols and vowel signs that are not `\p{L}`: circled
// letters, Indic matras, Hebrew/Arabic points.
#[test]
fn boundary_chars_match_the_ui_unicode_letter_digit_classes() {
    for c in [
        '\u{24D0}', '\u{24B6}', '\u{93E}', '\u{5B0}', '\u{64B}', '\u{345}',
    ] {
        assert!(c.is_alphanumeric(), "U+{:04X} is Alphabetic", c as u32);
        assert!(
            !mentioned_in(&format!("KAN-1{c}")),
            "U+{:04X} after the key",
            c as u32
        );
        assert!(
            !mentioned_in(&format!("{c}KAN-1")),
            "U+{:04X} before the key",
            c as u32
        );
    }
    // Currency, punctuation and emoji do not block; circled digits (category No) do.
    for (c, blocks) in [
        ('\u{20AC}', false),
        ('\u{2014}', false),
        ('\u{1F600}', false),
        ('\u{24EA}', true),
        ('\u{2460}', true),
    ] {
        assert_eq!(c.is_alphanumeric(), blocks, "U+{:04X}", c as u32);
        assert_eq!(
            mentioned_in(&format!("KAN-1{c}")),
            !blocks,
            "U+{:04X} after",
            c as u32
        );
        assert_eq!(
            mentioned_in(&format!("{c}KAN-1")),
            !blocks,
            "U+{:04X} before",
            c as u32
        );
    }
}

/* ── perf: one `ticket::links` call per ticket open ───────────────────── */

fn filler(rng: &mut Rng, bytes: usize, count: u64) -> String {
    let words = [
        "the",
        "quick",
        "brown",
        "fox",
        "über",
        "naïve",
        "日本語",
        "😀",
        "fix",
        "bug",
        "kan",
        "see",
    ];
    let mut out = String::with_capacity(bytes + 64);
    while out.len() < bytes {
        if rng.below(40) == 0 {
            out.push_str(&format!("KAN-{} ", 1 + rng.below(count as usize)));
        } else {
            out.push_str(words[rng.below(words.len())]);
            out.push(' ');
        }
    }
    out
}

#[test]
#[ignore = "perf: run with -- --ignored --nocapture"]
fn perf_ten_thousand_tickets_of_five_kilobytes() {
    let count = 10_000u64;
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    let tickets: Vec<Ticket> = (1..=count)
        .map(|n| {
            let title = filler(&mut rng, 100, count);
            let description = filler(&mut rng, 3_000, count);
            let (a, b) = (filler(&mut rng, 1_000, count), filler(&mut rng, 900, count));
            mk("KAN", n, &title, &description, &[&a, &b])
        })
        .collect();
    let board = board_of(tickets);
    let bytes: usize = board
        .tickets
        .values()
        .map(|t| {
            t.title.len()
                + t.description.len()
                + t.comments.iter().map(|c| c.body.len()).sum::<usize>()
        })
        .sum();
    println!("board: {count} tickets, {} MB of text", bytes / 1_000_000);

    let mut times = Vec::new();
    let mut shape = (0, 0);
    for subject in ["KAN-1", "KAN-5000", "KAN-10000"] {
        let started = Instant::now();
        let (a, b) = ticket_links(&board, subject, "KAN").unwrap();
        shape = (a.len(), b.len());
        times.push(started.elapsed());
        println!(
            "ticket_links({subject}): {:?} -> {} mentions, {} mentioned_by",
            started.elapsed(),
            shape.0,
            shape.1
        );
    }
    assert!(shape.0 > 0 && shape.1 > 0);

    let started = Instant::now();
    let body = serde_json::to_string_pretty(&board).unwrap();
    println!(
        "board.json: {} MB, serialize {:?}",
        body.len() / 1_000_000,
        started.elapsed()
    );
    let started = Instant::now();
    let parsed: Board = serde_json::from_str(&body).unwrap();
    println!(
        "read_board parse (done on every handler call): {:?}",
        started.elapsed()
    );
    assert_eq!(parsed.tickets.len(), count as usize);
}
