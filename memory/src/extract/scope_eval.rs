//! Extraction scope eval (MOT-5451): the production prompt and parser
//! ([`extract_items`]) over short EN excerpts, every case a repository
//! session with `default_bank` layered under it. Scores capture, kind, and
//! where each item lands ([`placement`]): what holds for the user wherever
//! they work must reach the default bank, and nothing project-only may.
//!
//! Live, so ignored: `III_URL=ws://127.0.0.1:<port> III_NAMESPACE=<ns>
//! EXTRACTION_MODEL=<id> cargo test -p memory --lib extract::scope_eval --
//! --ignored --nocapture`. Needs an engine with `router::complete` (a
//! router and a provider); no memory worker, no embeddings.

use std::collections::BTreeMap;

use super::*;
use crate::config::WorkerConfig;

const REPEATS: usize = 3;
/// Every excerpt line's date: Wednesday 2026-10-14 (UTC).
const TS: u64 = 1_792_000_000_000;

/// An item the case expects back: the first unused returned item whose
/// lowercased text holds every stem.
struct Want {
    has: &'static [&'static str],
    rule: bool,
    user: bool,
    /// The known memory it corrects, 1-based like the prompt's numbers.
    replaces: Option<usize>,
}

struct Case {
    id: &'static str,
    category: &'static str,
    /// Already remembered, with whether the layer holds it; the bank's own
    /// first, as in production.
    known: &'static [(&'static str, bool)],
    /// `(role, text)` excerpt lines.
    lines: &'static [(&'static str, &'static str)],
    want: &'static [Want],
}

const fn fact(has: &'static [&'static str], user: bool) -> Want {
    Want {
        has,
        rule: false,
        user,
        replaces: None,
    }
}

const fn rule(has: &'static [&'static str], user: bool) -> Want {
    Want {
        has,
        rule: true,
        user,
        replaces: None,
    }
}

const fn case(
    id: &'static str,
    category: &'static str,
    lines: &'static [(&'static str, &'static str)],
    want: &'static [Want],
) -> Case {
    Case {
        id,
        category,
        known: &[],
        lines,
        want,
    }
}

const CASES: &[Case] = &[
    // The user, wherever they work.
    case(
        "mango",
        "user facts",
        &[
            ("user", "I eat a mango every day."),
            ("assistant", "Nice habit. Mangoes are a good source of vitamin C."),
        ],
        &[fact(&["mango"], true)],
    ),
    case(
        "vegetarian",
        "user facts",
        &[
            ("user", "I'm vegetarian, by the way. Anyway: fix the flaky login test."),
            (
                "assistant",
                "Fixed: the login test now waits for the session cookie before it asserts.",
            ),
        ],
        &[fact(&["vegetarian"], true)],
    ),
    case(
        "name",
        "user facts",
        &[
            (
                "user",
                "Rename the env var DB_URL to DATABASE_URL everywhere. Oh, and my name is Rafael Costa.",
            ),
            ("assistant", "Done: DB_URL is now DATABASE_URL in 4 files."),
        ],
        &[fact(&["rafael"], true)],
    ),
    case(
        "dentist",
        "user facts",
        &[
            ("user", "I have a dentist appointment next Friday at 3pm."),
            ("assistant", "Got it, good luck at the dentist."),
        ],
        &[fact(&["dentist"], true)],
    ),
    case(
        "neovim",
        "user facts",
        &[
            ("user", "I use Neovim for everything, on every machine."),
            ("assistant", "Then I'll give editor tips for Neovim."),
        ],
        &[fact(&["neovim"], true)],
    ),
    // The repository at hand.
    case(
        "pnpm",
        "project facts",
        &[
            (
                "user",
                "This repo uses pnpm workspaces, so add zod to the ui package only.",
            ),
            ("assistant", "Added zod with `pnpm --filter ui add zod`."),
        ],
        &[fact(&["pnpm"], false)],
    ),
    case(
        "thursday",
        "project facts",
        &[
            ("user", "Heads up: we deploy this service every Thursday."),
            ("assistant", "Understood, I'll keep risky changes away from Thursdays."),
        ],
        &[fact(&["thursday"], false)],
    ),
    case(
        "marta",
        "project facts",
        &[
            (
                "user",
                "Marta owns the billing module, so ask her before you change it.",
            ),
            ("assistant", "Okay, I'll flag billing changes for Marta."),
        ],
        &[fact(&["marta"], false)],
    ),
    case(
        "postgres",
        "project facts",
        &[
            ("user", "The ledger-sync tests need Postgres 16 running locally."),
            (
                "assistant",
                "I'll start a Postgres 16 container before running them.",
            ),
        ],
        &[fact(&["postgres"], false)],
    ),
    // About the user, but only here.
    Case {
        id: "npm-here",
        category: "traps",
        known: &[("The user prefers npm for JavaScript projects.", true)],
        lines: &[
            ("user", "In this repo I use pnpm, not npm."),
            ("assistant", "Okay, pnpm it is for this repository."),
        ],
        want: &[fact(&["pnpm"], false)],
    },
    case(
        "squash",
        "traps",
        &[
            ("user", "For this project I prefer squash merges."),
            ("assistant", "Noted, I'll squash when merging here."),
        ],
        &[fact(&["squash"], false)],
    ),
    case(
        "release",
        "traps",
        &[
            ("user", "Here I always run the tests with --release."),
            ("assistant", "Okay, `cargo test --release` from now on in this repo."),
        ],
        &[fact(&["release"], false)],
    ),
    case(
        "tabs",
        "traps",
        &[
            ("user", "I prefer tabs in this codebase."),
            ("assistant", "Got it, tabs for indentation here."),
        ],
        &[fact(&["tabs"], false)],
    ),
    // How every reply is written.
    case(
        "portuguese",
        "user rules",
        &[
            ("user", "Always reply to me in Portuguese."),
            ("assistant", "Combinado, vou responder em português."),
        ],
        &[rule(&["portuguese"], true)],
    ),
    case(
        "emoji",
        "user rules",
        &[
            ("user", "Never use emoji in your answers."),
            ("assistant", "Understood, no emoji."),
        ],
        &[rule(&["emoji"], true)],
    ),
    case(
        "short",
        "user rules",
        &[
            ("user", "Keep your answers under five sentences."),
            ("assistant", "Will do."),
        ],
        &[rule(&["sentence"], true)],
    ),
    // How work is done here.
    case(
        "versions",
        "project rules",
        &[
            ("user", "Never bump versions by hand in this repo."),
            ("assistant", "Understood, the release bot bumps them."),
        ],
        &[rule(&["version"], false)],
    ),
    case(
        "fmt",
        "project rules",
        &[
            ("user", "Always run cargo fmt before you commit here."),
            ("assistant", "Okay, cargo fmt before every commit."),
        ],
        &[rule(&["fmt"], false)],
    ),
    // A correction lands, and retires its target, in the bank it belongs to.
    Case {
        id: "porto",
        category: "corrections",
        known: &[("The user lives in Lisbon.", true)],
        lines: &[
            ("user", "I moved to Porto last month."),
            ("assistant", "Congrats on the move!"),
        ],
        want: &[Want {
            has: &["porto"],
            rule: false,
            user: true,
            replaces: Some(1),
        }],
    },
    Case {
        id: "postgres-16",
        category: "corrections",
        known: &[("The ledger service runs on Postgres 15.", false)],
        lines: &[
            (
                "user",
                "We upgraded the ledger service to Postgres 16 yesterday.",
            ),
            ("assistant", "Good to know, I'll target Postgres 16."),
        ],
        want: &[Want {
            has: &["postgres", "16"],
            rule: false,
            user: false,
            replaces: Some(1),
        }],
    },
    // Nothing worth keeping.
    case(
        "flaky",
        "negatives",
        &[
            ("user", "Fix the flaky login test, please."),
            ("assistant", "Done, 214 tests pass."),
        ],
        &[],
    ),
    case(
        "question",
        "negatives",
        &[
            ("user", "What does parse_config return?"),
            ("assistant", "It returns the parsed Config."),
        ],
        &[],
    ),
];

#[derive(Default)]
struct Tally {
    n: usize,
    captured: usize,
    kind_ok: usize,
    main: usize,
    harmful: usize,
}

/// One returned item as a report line: `text / scope / kind / lands`.
fn line(item: &ExtractedItem, main: bool, retires: bool) -> String {
    format!(
        "{} / {} / {} / {}{}",
        item.text,
        item.scope.as_deref().unwrap_or("-"),
        item.kind.as_deref().unwrap_or("-"),
        if main { "main" } else { "repo" },
        match (retires, item.replaces) {
            (true, Some(n)) => format!(" retires [{n}]"),
            _ => String::new(),
        }
    )
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a live engine with router::complete"]
async fn extraction_scope_eval() {
    let url = std::env::var("III_URL").unwrap_or_else(|_| "ws://127.0.0.1:49134".into());
    let iii = iii_sdk::register_worker(&url, iii_sdk::InitOptions::default());
    let model = match std::env::var("EXTRACTION_MODEL") {
        Ok(m) if !m.is_empty() => m,
        _ => resolve_model(&iii, "")
            .await
            .expect("no model (engine up? router running? III_NAMESPACE set?)"),
    };
    let max_memories = WorkerConfig::default().max_memories_per_turn;

    let mut by_category: BTreeMap<&str, Tally> = BTreeMap::new();
    let mut report = Vec::new();
    let (mut expected, mut matched, mut kind_ok) = (0, 0, 0);
    let (mut user_matched, mut user_main, mut harmful) = (0, 0, 0);
    let (mut negatives, mut extras) = (0, 0);
    for case in CASES {
        let known: Vec<Memory> = case
            .known
            .iter()
            .map(|(text, _)| {
                let item = ExtractedItem {
                    text: text.to_string(),
                    entities: vec![],
                    tags: vec![],
                    confidence: None,
                    kind: None,
                    replaces: None,
                    scope: None,
                };
                new_memory(item, case.id, None, TS)
            })
            .collect();
        let layered: HashSet<String> = known
            .iter()
            .zip(case.known)
            .filter(|(_, (_, in_layer))| *in_layer)
            .map(|(m, _)| m.id.clone())
            .collect();
        let entries: Vec<Entry> = case
            .lines
            .iter()
            .enumerate()
            .map(|(i, (role, text))| (format!("e{i}"), role.to_string(), text.to_string(), TS))
            .collect();
        let (transcript, ..) = excerpt(entries, 12);
        let tally = by_category.entry(case.category).or_default();
        for repeat in 1..=REPEATS {
            let tag = format!("{}#{repeat}", case.id);
            let items = extract_items(
                &iii,
                &model,
                max_memories,
                120_000,
                extraction_input(&known, &transcript),
                case.id,
            )
            .await
            .unwrap_or_else(|e| panic!("{tag}: {e}"));
            // Where each item lands, as `try_run` routes it under a layer.
            let lands: Vec<(bool, bool)> = items
                .iter()
                .map(|item| match item.is_rule() {
                    true => (item.is_user(), false),
                    false => placement(
                        item.is_user(),
                        correction_target(&known, item.replaces).map(|t| layered.contains(&t.id)),
                    ),
                })
                .collect();
            let mut want_of: Vec<Option<&Want>> = vec![None; items.len()];
            for want in case.want {
                expected += 1;
                tally.n += 1;
                let hit = (0..items.len()).find(|&i| {
                    let text = items[i].text.to_lowercase();
                    want_of[i].is_none() && want.has.iter().all(|s| text.contains(s))
                });
                let Some(i) = hit else {
                    report.push(format!(
                        "MISS {tag}: {:?} / {} / {} / {}",
                        want.has,
                        if want.user { "user" } else { "project" },
                        if want.rule { "rule" } else { "memory" },
                        if want.user { "main" } else { "repo" },
                    ));
                    continue;
                };
                want_of[i] = Some(want);
                matched += 1;
                tally.captured += 1;
                let main = lands[i].0;
                if items[i].is_rule() == want.rule {
                    kind_ok += 1;
                    tally.kind_ok += 1;
                }
                if main {
                    tally.main += 1;
                }
                if want.user {
                    user_matched += 1;
                    user_main += usize::from(main);
                }
            }
            for (i, item) in items.iter().enumerate() {
                let (main, retires) = lands[i];
                // Retiring a memory the case never asked to correct.
                let stray = retires && !case.want.iter().any(|w| w.replaces == item.replaces);
                // Unmatched too: a second, user-scope copy of a project fact
                // still reaches every repository.
                let leak = main
                    && match want_of[i] {
                        Some(w) => !w.user,
                        None => {
                            let text = item.text.to_lowercase();
                            case.want
                                .iter()
                                .any(|w| !w.user && w.has.iter().all(|s| text.contains(s)))
                        }
                    };
                if leak || stray {
                    harmful += 1;
                    tally.harmful += 1;
                }
                if want_of[i].is_none() {
                    extras += 1;
                    negatives += usize::from(case.want.is_empty());
                }
                let label = match want_of[i] {
                    _ if leak || stray => "HARMFUL",
                    Some(w) if w.rule != item.is_rule() || w.user != main => "WRONG",
                    Some(_) => continue,
                    None => "EXTRA",
                };
                report.push(format!("{label} {tag}: {}", line(item, main, retires)));
            }
        }
    }

    let ratio = |a: usize, b: usize| if b == 0 { 0.0 } else { a as f64 / b as f64 };
    let user_landing = ratio(user_main, user_matched);
    println!(
        "extraction scope eval: model {model}, {} cases x {REPEATS} repeats",
        CASES.len()
    );
    println!(
        "{:<14} {:>4} {:>9} {:>8} {:>5} {:>8}",
        "category", "n", "captured", "kind ok", "main", "harmful"
    );
    for (category, t) in &by_category {
        println!(
            "{category:<14} {:>4} {:>9} {:>8} {:>5} {:>8}",
            t.n, t.captured, t.kind_ok, t.main, t.harmful
        );
    }
    for line in &report {
        println!("{line}");
    }
    println!(
        "capture {:.3} ({matched}/{expected}), kind accuracy {:.3} ({kind_ok}/{matched}), \
         user landing {user_landing:.3} ({user_main}/{user_matched}), harmful {harmful}, \
         negatives {negatives}, extras {extras}",
        ratio(matched, expected),
        ratio(kind_ok, matched),
    );
    assert_eq!(
        harmful, 0,
        "a project-only item reached main, or a stray retire"
    );
    assert!(
        user_landing >= 0.85,
        "user landing {user_landing:.3} < 0.85"
    );
}
