//! Extraction eval (MOT-5451, MOT-5452): the production prompt and parser
//! ([`extract_items`]) over short EN excerpts, every case a repository
//! session with `default_bank` layered under it. Scores two things:
//! - capture: every expected item is captured, wrong (its kind, where it
//!   lands, the memory it corrects, or the rule gate [`stated_by_user`]
//!   drops it), or missed. A trap hit is any other item returned from an
//!   excerpt built to hold nothing more ([`STRICT`]): an instruction only
//!   the assistant gave, a pasted document, a harness notice, a transient
//!   error, a one-off instruction, task progress. The report also counts
//!   the trap hits the rule gate would still let into the bank.
//! - placement ([`placement`]): what holds for the user wherever they work
//!   must reach the default bank, and nothing project-only may.
//!
//! Live, so ignored: `III_URL=ws://127.0.0.1:<port> III_NAMESPACE=<ns>
//! EXTRACTION_MODEL=<id> cargo test -p memory --lib extract::eval --
//! --ignored --nocapture`. Needs an engine with `router::complete` (a
//! router and a provider); no memory worker, no embeddings.

use std::collections::BTreeMap;

use super::*;
use crate::config::WorkerConfig;

const REPEATS: usize = 3;
/// Every excerpt line's date: Wednesday 2026-10-14 (UTC).
const TS: u64 = 1_792_000_000_000;

/// Categories whose excerpts hold nothing durable past their `want`: any
/// other item returned is a trap hit, not an extra.
const STRICT: &[&str] = &[
    "assistant-only",
    "corrections",
    "errors",
    "negatives",
    "notices",
    "one-off",
    "pasted",
    "progress",
];

/// An item the case expects back: the first unused returned item whose
/// lowercased text holds every stem.
struct Want {
    has: &'static [&'static str],
    /// `None`: either kind is right ("Use tabs here." or "The user uses
    /// tabs here.").
    rule: Option<bool>,
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
        rule: Some(false),
        user,
        replaces: None,
    }
}

const fn rule(has: &'static [&'static str], user: bool) -> Want {
    Want {
        has,
        rule: Some(true),
        user,
        replaces: None,
    }
}

const fn either(has: &'static [&'static str], user: bool) -> Want {
    Want {
        has,
        rule: None,
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
        &[either(&["marta"], false)],
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
        category: "here only",
        known: &[("The user prefers npm for JavaScript projects.", true)],
        lines: &[
            ("user", "In this repo I use pnpm, not npm."),
            ("assistant", "Okay, pnpm it is for this repository."),
        ],
        want: &[fact(&["pnpm"], false)],
    },
    case(
        "squash",
        "here only",
        &[
            ("user", "For this project I prefer squash merges."),
            ("assistant", "Noted, I'll squash when merging here."),
        ],
        &[either(&["squash"], false)],
    ),
    case(
        "release",
        "here only",
        &[
            ("user", "Here I always run the tests with --release."),
            ("assistant", "Okay, `cargo test --release` from now on in this repo."),
        ],
        &[either(&["release"], false)],
    ),
    case(
        "tabs",
        "here only",
        &[
            ("user", "I prefer tabs in this codebase."),
            ("assistant", "Got it, tabs for indentation here."),
        ],
        &[either(&["tabs"], false)],
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
    // The user adopts the assistant's suggestion in words of their own.
    case(
        "adopted",
        "project rules",
        &[
            ("user", "How should I name branches here?"),
            (
                "assistant",
                "A common convention is a feat/ or fix/ prefix plus the ticket id, like feat/MOT-12-login.",
            ),
            ("user", "Good, do that for every branch from now on."),
        ],
        &[rule(&["feat/"], false)],
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
            rule: Some(false),
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
            rule: Some(false),
            user: false,
            replaces: Some(1),
        }],
    },
    Case {
        id: "dentist-moved",
        category: "corrections",
        known: &[(
            "The user has a dentist appointment on 2026-10-16 at 3pm.",
            true,
        )],
        lines: &[
            ("user", "The dentist moved my appointment to Monday, same time."),
            ("assistant", "Okay, Monday at 3pm then."),
        ],
        want: &[Want {
            has: &["dentist", "19"],
            rule: Some(false),
            user: true,
            replaces: Some(1),
        }],
    },
    // Corrected in the excerpt itself: only the corrected fact is kept.
    case(
        "cashews",
        "corrections",
        &[
            ("user", "I'm allergic to peanuts. Sorry, no: cashews, not peanuts."),
            ("assistant", "Noted: cashews."),
        ],
        &[fact(&["cashew"], true)],
    ),
    case(
        "london",
        "corrections",
        &[
            (
                "assistant",
                "Since you're in New York, I'll give times in Eastern.",
            ),
            ("user", "I'm in London, not New York."),
            ("assistant", "Got it, London times from now on."),
        ],
        &[fact(&["london"], true)],
    ),
    // Instructions and claims only the assistant made.
    case(
        "own-rules",
        "assistant-only",
        &[
            ("user", "Move the date parser into its own module."),
            (
                "assistant",
                "Done. Going forward I'll always add a unit test for every new module and never call unwrap in library code.",
            ),
        ],
        &[],
    ),
    case(
        "advice",
        "assistant-only",
        &[
            ("user", "How do people usually name branches?"),
            (
                "assistant",
                "A common convention: always prefix branches with feat/ or fix/, and never reuse the name of a merged branch.",
            ),
        ],
        &[],
    ),
    case(
        "echoed-policy",
        "assistant-only",
        &[
            ("user", "Push the fix."),
            (
                "assistant",
                "I can't push to main: my instructions say every change goes through a pull request. I opened #812 instead.",
            ),
        ],
        &[],
    ),
    case(
        "invented-preference",
        "assistant-only",
        &[
            ("user", "Recommend a podcast."),
            (
                "assistant",
                "Since you love history, try The Rest Is History; episodes run about an hour.",
            ),
        ],
        &[],
    ),
    case(
        "own-rule-beside-fact",
        "assistant-only",
        &[
            ("user", "We're on Rust 1.92 in this repo."),
            (
                "assistant",
                "Got it, 1.92. I'll also always run clippy with -D warnings before I suggest code.",
            ),
        ],
        &[fact(&["1.92"], false)],
    ),
    // Someone else's words, pasted by the user.
    case(
        "blog-post",
        "pasted",
        &[
            (
                "user",
                "TL;DR this post for me:\n\nWhy I switched editors\n\nI've used Emacs for twenty \
                 years, and last spring I finally moved to Vim. I live in Montreal, work remotely, \
                 and I'm allergic to long config files. My advice to every developer: always write \
                 commit messages in the imperative mood, never commit straight to main, and keep \
                 your dotfiles in git. If you take one thing from this post, delete half your \
                 plugins today.\n\n- Sam",
            ),
            (
                "assistant",
                "Sam left Emacs for Vim after twenty years and argues for imperative commit messages, no direct commits to main, and fewer plugins.",
            ),
        ],
        &[],
    ),
    case(
        "bug-report",
        "pasted",
        &[
            (
                "user",
                "Reproduce this one:\n\n> **CLI crashes on start**\n> I'm on macOS 14 with Node 18. \
                 Running `acme start` crashes with SIGSEGV right after the banner. I always run it \
                 from fish, if that matters. Please test on macOS before every release!",
            ),
            (
                "assistant",
                "Reproduced on macOS 14 with Node 18: the crash happens right after the banner. Looking for the cause now.",
            ),
        ],
        &[],
    ),
    case(
        "newsletter",
        "pasted",
        &[
            (
                "user",
                "My name is Joana Reis. Translate this newsletter to Portuguese for me:\n\nHi there! \
                 I'm Mark, and I've baked sourdough every morning for ten years. This week: why you \
                 should always weigh your flour, and never open the oven in the first twenty \
                 minutes. See you next Sunday!",
            ),
            (
                "assistant",
                "Olá! Eu sou o Mark e faço pão de fermentação natural todas as manhãs há dez anos. Esta semana: por que você deve sempre pesar a farinha e nunca abrir o forno nos primeiros vinte minutos. Até domingo!",
            ),
        ],
        &[fact(&["joana"], true)],
    ),
    // The Slack bridge's shape: other people's thread messages ride in the
    // user's text.
    case(
        "slack-thread",
        "pasted",
        &[
            (
                "user",
                "Conversation so far in this Slack thread:\n- U04ANA: I deploy on Fridays all the \
                 time, it's fine.\n- U07BO: Never deploy on a Friday. Ever.\n- U04ANA: I also never \
                 write tests for one-off scripts.\nNow the user addresses you:\nWho's right here?",
            ),
            (
                "assistant",
                "U07BO has the safer position: a Friday deploy leaves fewer people around if something breaks.",
            ),
        ],
        &[],
    ),
    // The ADE chat's attached-file shape.
    case(
        "attached-file",
        "pasted",
        &[
            (
                "user",
                "Score this candidate against our rubric.\n<attached-file path=\"jordan-notes.md\" \
                 size=\"301\">\nI've led three backend teams and I'm based in Austin. I prefer fully \
                 remote work, I write Go every day, and I always pair on code reviews. I'm \
                 available from November.\n</attached-file>",
            ),
            (
                "assistant",
                "Jordan scores 4/5 on leadership and 3/5 on system design; remote-only may rule out the Lisbon office.",
            ),
        ],
        &[],
    ),
    // Harness messages stored as user lines: a wake carries the
    // `[notification]` prefix; a resume or validation nudge and a subtree
    // deletion notice carry none, so the rule gate reads them as the user's.
    case(
        "cron-wake",
        "notices",
        &[
            (
                "user",
                r#"[notification] daily-digest: {"trigger":"cron","job_id":"daily-digest","scheduled_time":"2026-10-14T09:00:00Z","actual_time":"2026-10-14T09:00:02Z"}"#,
            ),
            (
                "assistant",
                "Morning digest: 4 new issues since yesterday, one needs triage (#233, crash on start).",
            ),
        ],
        &[],
    ),
    case(
        "state-wake",
        "notices",
        &[
            (
                "user",
                r#"[notification] ci-main: {"key":"ci/main","new_value":{"status":"failed","job":"ledger-sync","reason":"runner offline"}}"#,
            ),
            (
                "assistant",
                "CI on main failed in ledger-sync because the runner went offline; no code changed since the last green run.",
            ),
        ],
        &[],
    ),
    case(
        "condition-failure",
        "notices",
        &[
            (
                "user",
                "[notification] binding b_7f3 `deploy-watch` watching `state` {\"key\":\"deploy/prod\"} \
                 fired but was NOT delivered: condition function deploy::ready timed out. The binding \
                 stays armed and every fire will keep skipping until the condition call succeeds. If \
                 the condition is misconfigured, unregister the binding, re-register it corrected, \
                 and then read the watched state once — events from before the new registration \
                 will never fire it.",
            ),
            (
                "assistant",
                "The deploy watch's condition timed out; I re-registered the binding and read deploy/prod once.",
            ),
        ],
        &[],
    ),
    case(
        "resume-nudge",
        "notices",
        &[
            ("user", "Write the release notes for 2.4."),
            ("assistant", "Release notes for 2.4:\n- Faster sync"),
            (
                "user",
                "The provider/router ended transiently after streaming the previous partial \
                 response: stream read failed: connection reset. Resume from the transcript and \
                 return the complete deliverable, incorporating the previous partial even if it \
                 was already nearly complete. Do not repeat function calls that already have \
                 successful results.",
            ),
            (
                "assistant",
                "Release notes for 2.4:\n- Faster sync\n- New retry policy\n- Fixed the Windows path bug",
            ),
        ],
        &[],
    ),
    case(
        "validation-nudge",
        "notices",
        &[
            ("user", "Classify ticket #233."),
            ("assistant", "bug"),
            (
                "user",
                "Your previous result was not accepted: missing required field `severity`. Please \
                 provide a corrected result that satisfies the required output contract.",
            ),
            ("assistant", r#"{"type":"bug","severity":"high"}"#),
        ],
        &[],
    ),
    case(
        "deletion-notice",
        "notices",
        &[
            (
                "user",
                "User-requested cancellation and deletion of session triage-worker (s_4f2a) and its \
                 subtree: triage-worker, triage-worker/fetch. All turns have stopped; these sessions \
                 are being deleted. Do not await their results and do not recreate them \
                 automatically. Your history, execution and other children are preserved. \
                 Operation: op_91c.",
            ),
            (
                "assistant",
                "Understood: the triage worker is gone, and I'll finish the triage here.",
            ),
        ],
        &[],
    ),
    case(
        "workflow-reply",
        "notices",
        &[
            (
                "user",
                "Workflow wf_7d1 completed.\n\nResult:\n{\n  \"tests\": 214,\n  \"failed\": 0,\n  \
                 \"coverage\": 0.81\n}",
            ),
            ("assistant", "The nightly workflow passed: 214 tests, 81% coverage."),
        ],
        &[],
    ),
    case(
        "wake-beside-fact",
        "notices",
        &[
            (
                "user",
                r#"[notification] standup-reminder: "Standup in 10 minutes""#,
            ),
            ("assistant", "Reminder: standup starts in 10 minutes."),
            (
                "user",
                "Thanks. I'm off every Friday, by the way.",
            ),
            ("assistant", "Noted, enjoy the long weekends."),
        ],
        &[fact(&["friday"], true)],
    ),
    // Errors that pass.
    case(
        "provider-error",
        "errors",
        &[
            ("user", "Draft the changelog for 2.4."),
            (
                "assistant",
                r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
            ),
            ("user", "Try again."),
            (
                "assistant",
                "Changelog for 2.4: faster sync, a new retry policy, and a fix for Windows paths.",
            ),
        ],
        &[],
    ),
    case(
        "econnreset",
        "errors",
        &[
            ("user", "npm install keeps failing with ECONNRESET, what's going on?"),
            (
                "assistant",
                "The registry timed out; a retry with --fetch-retries=5 went through. Nothing to change in the project.",
            ),
        ],
        &[],
    ),
    case(
        "unavailable",
        "errors",
        &[
            ("user", "Summarize the open PRs."),
            (
                "assistant",
                "The GitHub API answered 503 twice, so I retried after 30 seconds. Three PRs are open, #901, #904 and #910, all waiting on review.",
            ),
        ],
        &[],
    ),
    case(
        "runner-down",
        "errors",
        &[
            (
                "user",
                "CI is red only because the self-hosted runner is offline right now, ignore it for today.",
            ),
            (
                "assistant",
                "Okay, I'll ignore the runner failures today and rely on local test runs.",
            ),
        ],
        &[],
    ),
    case(
        "staging-down",
        "errors",
        &[
            (
                "user",
                "Staging is down this afternoon, so test the checkout flow against the local stack instead.",
            ),
            ("assistant", "Done: the checkout tests pass against the local stack."),
        ],
        &[],
    ),
    case(
        "registry-beside-fact",
        "errors",
        &[
            (
                "user",
                "The deploy failed with a 502 from the registry, just retry it. FYI our images live in ghcr.io/acme.",
            ),
            ("assistant", "Retried: the deploy went through on the second attempt."),
        ],
        &[fact(&["ghcr"], false)],
    ),
    // Where the work stands.
    case(
        "migration-steps",
        "progress",
        &[
            ("user", "Keep going with the migration."),
            (
                "assistant",
                "Step 2 of 4 done: the users table is migrated and verified. Next I'll migrate orders, then invoices.",
            ),
        ],
        &[],
    ),
    case(
        "halfway",
        "progress",
        &[
            (
                "user",
                "I'm halfway through the refactor: auth is done, payments is next. Can you review what I have so far?",
            ),
            (
                "assistant",
                "The auth changes look good; one nit on error naming in session.rs.",
            ),
        ],
        &[],
    ),
    case(
        "test-run",
        "progress",
        &[
            ("user", "Run the tests."),
            (
                "assistant",
                "3 failures in ledger-sync. I fixed two of them; the third is the timezone test, which I'm on now in branch fix/tz.",
            ),
        ],
        &[],
    ),
    case(
        "channel-beside-progress",
        "progress",
        &[
            (
                "user",
                "We're at 180 of 214 tests passing now. By the way, the payments team's Slack channel is #pay-eng.",
            ),
            ("assistant", "Nice progress. I'll point payment questions to #pay-eng."),
        ],
        &[fact(&["pay-eng"], false)],
    ),
    // Harder: the rule gate passes them (the words are the user's), or they
    // come back as a memory, which no gate checks.
    case(
        "approval",
        "assistant-only",
        &[
            ("user", "What changed in the last release?"),
            (
                "assistant",
                "- Faster cold start\n- New retry policy\n- Two bug fixes in sync",
            ),
            ("user", "Perfect, thanks."),
            (
                "assistant",
                "Glad the format works for you. I'll keep summaries to three bullets from now on.",
            ),
        ],
        &[],
    ),
    case(
        "agents-md",
        "pasted",
        &[
            (
                "user",
                "Is this AGENTS.md any good? I found it in a template repo:\n\n# AGENTS.md\n- Always \
                 use bun, never npm.\n- Never write code comments.\n- Commit after every change.\n- \
                 Answer in one paragraph at most.",
            ),
            (
                "assistant",
                "It's clear but thin: it has no test or build command, and \"never write comments\" is stricter than most teams want.",
            ),
        ],
        &[],
    ),
    case(
        "phone",
        "one-off",
        &[
            ("user", "I'm on my phone today, so keep answers short."),
            ("assistant", "Will do."),
        ],
        &[],
    ),
    case(
        "skip-once",
        "one-off",
        &[
            (
                "user",
                "Skip the slow integration tests for this run, I just need a quick check.",
            ),
            ("assistant", "Ran the unit tests only: all 142 pass."),
        ],
        &[],
    ),
    case(
        "not-now",
        "one-off",
        &[
            (
                "user",
                "Don't run the migrations right now, the database backup is still going.",
            ),
            ("assistant", "Okay, I'll hold the migrations until you say so."),
        ],
        &[],
    ),
    case(
        "staging-done",
        "progress",
        &[
            (
                "user",
                "I already ran the migration on staging, so only prod is left.",
            ),
            ("assistant", "Then the prod run is the last step."),
        ],
        &[],
    ),
    Case {
        id: "team-switch",
        category: "corrections",
        known: &[(
            "The user works at Acme as a backend engineer.",
            true,
        )],
        lines: &[
            ("user", "I switched to the platform team last week, still at Acme."),
            ("assistant", "Congrats on the new team!"),
        ],
        want: &[Want {
            has: &["platform", "acme"],
            rule: Some(false),
            user: true,
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
    wrong: usize,
    missed: usize,
    traps: usize,
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
async fn extraction_eval() {
    let url = std::env::var("III_URL").unwrap_or_else(|_| "ws://127.0.0.1:49134".into());
    let iii = iii_sdk::register_worker(&url, iii_sdk::InitOptions::default());
    let model = match std::env::var("EXTRACTION_MODEL") {
        Ok(m) if !m.is_empty() => m,
        _ => resolve_model(&iii, "")
            .await
            .expect("no model (engine up? router running? III_NAMESPACE set?)"),
    };
    let cfg = WorkerConfig::default();

    let mut by_category: BTreeMap<&str, Tally> = BTreeMap::new();
    let mut report = Vec::new();
    let (mut expected, mut captured, mut wrong, mut missed) = (0, 0, 0, 0);
    let (mut traps, mut kept, mut extras) = (0, 0, 0);
    let (mut matched, mut kind_ok, mut user_matched, mut user_main, mut harmful) = (0, 0, 0, 0, 0);
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
        let said = user_words(&entries);
        // Whether a returned item passes the rule gate, redacted as `try_run` does.
        let gate = |item: &ExtractedItem| {
            !item.is_rule() || stated_by_user(&crate::redact::redact(item.text.trim()).text, &said)
        };
        let strict = STRICT.contains(&case.category);
        let (transcript, ..) = excerpt(entries, cfg.extraction_window);
        let tally = by_category.entry(case.category).or_default();
        for repeat in 1..=REPEATS {
            let tag = format!("{}#{repeat}", case.id);
            let items = extract_items(
                &iii,
                &model,
                cfg.max_memories_per_turn,
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
                match hit {
                    Some(i) => want_of[i] = Some(want),
                    None => {
                        missed += 1;
                        tally.missed += 1;
                        report.push(format!(
                            "MISS {tag}: {:?} / {} / {} / {}",
                            want.has,
                            if want.user { "user" } else { "project" },
                            match want.rule {
                                Some(true) => "rule",
                                Some(false) => "memory",
                                None => "either",
                            },
                            if want.user { "main" } else { "repo" },
                        ));
                    }
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
                let label = match want_of[i] {
                    Some(want) => {
                        matched += 1;
                        let kind_right = want.rule.is_none_or(|r| r == item.is_rule());
                        kind_ok += usize::from(kind_right);
                        tally.main += usize::from(main);
                        if want.user {
                            user_matched += 1;
                            user_main += usize::from(main);
                        }
                        let why = if !kind_right {
                            Some("kind")
                        } else if main != want.user {
                            Some("lands")
                        } else if item.replaces != want.replaces {
                            Some("replaces")
                        } else if !gate(item) {
                            Some("gated")
                        } else {
                            None
                        };
                        match why {
                            None => {
                                captured += 1;
                                tally.captured += 1;
                                continue;
                            }
                            Some(why) => {
                                wrong += 1;
                                tally.wrong += 1;
                                format!("WRONG {why}")
                            }
                        }
                    }
                    None if strict => {
                        traps += 1;
                        tally.traps += 1;
                        let through = gate(item);
                        kept += usize::from(through);
                        match through {
                            true => "TRAP".to_string(),
                            false => "TRAP gated".to_string(),
                        }
                    }
                    None => {
                        extras += 1;
                        "EXTRA".to_string()
                    }
                };
                let label = if leak || stray {
                    "HARMFUL".to_string()
                } else {
                    label
                };
                report.push(format!("{label} {tag}: {}", line(item, main, retires)));
            }
        }
    }

    let ratio = |a: usize, b: usize| if b == 0 { 0.0 } else { a as f64 / b as f64 };
    let user_landing = ratio(user_main, user_matched);
    println!(
        "extraction eval: model {model}, {} cases x {REPEATS} repeats",
        CASES.len()
    );
    println!(
        "{:<14} {:>4} {:>9} {:>6} {:>7} {:>6} {:>5} {:>8}",
        "category", "n", "captured", "wrong", "missed", "traps", "main", "harmful"
    );
    for (category, t) in &by_category {
        println!(
            "{category:<14} {:>4} {:>9} {:>6} {:>7} {:>6} {:>5} {:>8}",
            t.n, t.captured, t.wrong, t.missed, t.traps, t.main, t.harmful
        );
    }
    for line in &report {
        println!("{line}");
    }
    println!(
        "capture: captured {captured}, wrong {wrong}, missed {missed} of {expected}; \
         trap hits {traps} ({kept} past the rule gate); extras {extras}"
    );
    println!(
        "placement: kind accuracy {:.3} ({kind_ok}/{matched}), user landing \
         {user_landing:.3} ({user_main}/{user_matched}), harmful {harmful}",
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
