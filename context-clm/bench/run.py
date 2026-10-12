"""Drive context-clm bench episodes through the isolated iii stack (boot.sh), grade them, estimate their cost.

  python run.py grid regression|long-pilot|long [--provider P --model M [--model2 M2]] > episodes.json
  python run.py estimate episodes.json [--prior RUN ...]
  python run.py verify-long RUN
  python run.py run RUN episodes.json --yes [--workers 6] [--prior RUN ...]

Episode: {arm: clm|sum|base|default, task, level, seed, model, provider?, budget?, effort?, max_cost_usd?, tag?}
(provider defaults to anthropic).
Tasks: kv|log|needle (ContextBench-style; level = size), fields (level 24 = ide functions, 40 = ide + harness),
http-brace|ide-update-op (long coding tasks). A strategy arm opts its session in through session metadata
{context_strategy, context_budget_tokens}; `default` sets neither. Results: RUN/results/<sid>.json.
"""

import argparse
import fcntl
import json
import os
import re
import shutil
import signal
import statistics
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from urllib.parse import urlparse

from ctxbench import CLAUDE_PER_O200K, END, GEN, LIMIT, answer_re, gen
from skills import skill

HERE = Path(__file__).resolve().parent
REPO = Path(os.environ.get("BENCH_REPO") or HERE.parents[1])
ENGINE = os.environ.get("BENCH_ENGINE", "ws://127.0.0.1:49250")
if urlparse(ENGINE).port in (None, 49134):  # no port means the CLI's default, 49134
    raise SystemExit(f"BENCH_ENGINE={ENGINE} is the live engine's port 49134; the bench stack listens on :49250")
III = [os.environ.get("III_BIN", str(Path.home() / ".local/bin/iii")), "trigger", "--engine", ENGINE]
S55, O55 = "claude-sonnet-5-5", "claude-opus-5-5"
MTAG = {O55: "o55", S55: "s55"}
SYSTEM = "You are an assistant working in a Linux shell through tools."
DROP = "anthropic dropped replayed thinking blocks"
ANSI = re.compile(r"\x1b\[[0-9;]*m")
TRUTH = [json.loads(line) for line in (HERE / "truth-fields.jsonl").read_text().splitlines()]
FIELDS_SNAPSHOT = "d3f9bcb8a"
FIELDS_TASK = (
    "Your working directory contains {dirs}, the Rust source of {what}. For each of the {n} bus functions listed "
    "below, find the request type its handler deserializes and list the TOP-LEVEL JSON field names a caller can send: "
    "apply serde renames and rename_all, skip fields marked #[serde(skip)] or #[schemars(skip)], and for "
    "#[serde(flatten)] include the flattened struct's fields. Work only from the source code.\n\nWhen you are done, "
    'reply with one JSON object {{"<function id>": ["field", ...], ...}} (each array sorted) as your final message; '
    "that message is what is graded.\n\nFunctions: {fns}"
)
SHELL_HINT = (
    "shell::exec runs no shell and starts in your working directory: pass cwd for a subdirectory, and for pipes, "
    'redirection or heredocs run {"command":"bash","args":["-c","<command>"]}.'
)
LONG = {
    "http-brace": {
        "base": "e5775dbf9",
        "fix": "6183f131d",
        "paths": ["http", "crates", "rust-toolchain.toml"],
        "tests": ["http/src/trigger.rs", "http/tests/e2e_routing.rs", "http/tests/webhook_listener.rs"],
        "splice_tests": ["http/src/trigger.rs"],
        "solution": ["http/src/types.rs"],
        "required": [
            "trigger::tests::register_trigger_with_brace_parameters_normalizes_and_matches",
            "trigger::tests::register_trigger_with_multiple_brace_parameters_extracts_all_params",
            "trigger::tests::register_trigger_brace_and_colon_routes_conflict",
            "trigger::tests::register_trigger_with_brace_parameters_unregisters_cleanly",
        ],
        "cwd": "http",
        "grade": [
            ["cargo", "test", "--locked", "--lib"],
            ["cargo", "test", "--locked", "--test", "webhook_listener", "--test", "e2e_routing"],
        ],
        "prompt": (
            "Your working directory contains `http/`, the Rust source of the iii HTTP worker, and `crates/`, shared "
            "crates it builds against. HTTP triggers register a route from `api_path` in the trigger config; a path "
            "parameter is written `:name` (for example `/tasks/:tid`). Add support for the OpenAPI spelling: a whole "
            "path segment `{name}` must behave exactly like `:name` (`/tasks/{tid}` registers, conflicts and matches "
            "like `/tasks/:tid`, and the parameter is extracted as `tid`). A segment that is not exactly `{name}` "
            "(such as `{tid`, `tid}`, `{}` or `{a{b}`) stays a literal. Tests describing this are already in the tree "
            "and fail today: `register_trigger_with_brace_parameters_*`, `register_trigger_with_multiple_brace_*` and "
            "`register_trigger_brace_and_colon_routes_conflict` in `http/src/trigger.rs`, and "
            "`brace_parameter_route_serves_and_extracts_path_params` in `http/tests/webhook_listener.rs`. Do not "
            "change those tests. Run tests from `http/` with `cargo test --locked` (a cold build takes about a "
            "minute). When `cargo test --locked --lib` and `cargo test --locked --test webhook_listener` pass, reply "
            "with a short summary of your change."
        ),
    },
    "ide-update-op": {
        "base": "0466e008a",
        "fix": "325a19129",
        "paths": ["ide", "crates", "rust-toolchain.toml"],
        "exclude": ["ide/ui"],
        "stubs": ["ide/ui/dist/page.js", "ide/ui/dist/styles.css", "ide/ui/dist/xterm.js"],
        "tests": ["ide/tests/code_update_ops.rs", "ide/tests/code_golden_errors.rs"],
        "solution": [
            "ide/src/code/functions/create_file.rs",
            "ide/src/code/functions/mod.rs",
            "ide/src/code/functions/update_file.rs",
        ],
        # NOTE: the 6 protected integration tests plus the 71 `update_file` unit tests that pass at both the base and
        # the fix commit. The 8 unit tests the fix adds live in the solution files, so they are not required.
        "required": [
            "batch_with_mix_of_success_and_failure_preserves_originals",
            "bottom_up_application_e2e",
            "crlf_line_endings_preserved_after_update",
            "error_message_formats_match_golden",
            "missing_op_is_inferred_through_the_handler",
            "regex_replace_e2e",
            "code::functions::read_window::tests::count_lines_matches_update_file_convention",
            "code::functions::update_file::handler_tests::a1_multiline_collapsing_regex_does_not_panic",
            "code::functions::update_file::handler_tests::a2_regex_sites_report_post_apply_positions",
            "code::functions::update_file::handler_tests::a3_sequential_regex_ops_attribute_sites_to_own_input",
            "code::functions::update_file::handler_tests::a4_insert_echo_shifts_through_regex_deltas",
            "code::functions::update_file::handler_tests::c211_wording_identical_for_missing_and_glob_denied",
            "code::functions::update_file::handler_tests::echo_budget_truncation_sets_flag",
            "code::functions::update_file::handler_tests::echo_insert_appears_in_handler_result",
            "code::functions::update_file::handler_tests::echo_insert_correct_post_apply_region",
            "code::functions::update_file::handler_tests::echo_pathological_1000_line_op_stays_bounded",
            "code::functions::update_file::handler_tests::echo_regex_replace_site_appears_in_result",
            "code::functions::update_file::handler_tests::echo_remove_shows_context_around_gap",
            "code::functions::update_file::handler_tests::echo_two_line_ops_offset_correctness",
            "code::functions::update_file::handler_tests::echo_update_lines_correct_content_and_position",
            "code::functions::update_file::handler_tests::empty_files_array_rejected",
            "code::functions::update_file::handler_tests::end_to_end_regex_replace",
            "code::functions::update_file::handler_tests::end_to_end_single_file_update_lines_writes_atomically",
            "code::functions::update_file::handler_tests::jail_escape_aborts_batch_with_top_level_c215",
            "code::functions::update_file::handler_tests::mixed_update_lines_then_regex_replace",
            "code::functions::update_file::handler_tests::non_accessible_path_yields_per_file_error_without_failing_batch",
            "code::functions::update_file::handler_tests::original_file_untouched_when_ops_invalid",
            "code::functions::update_file::handler_tests::pattern_snippet_truncates_long_patterns",
            "code::functions::update_file::handler_tests::r1_dollar_dollar_escape_writes_literal_dollar",
            "code::functions::update_file::handler_tests::r1_production_repro_template_literal_ref_rejected_pre_write",
            "code::functions::update_file::handler_tests::r1_unbraced_longest_run_gotcha_dollar_1a",
            "code::functions::update_file::handler_tests::r1_undefined_ref_rejected_even_with_expect_matches_zero",
            "code::functions::update_file::handler_tests::r1_valid_capture_rewrites_still_apply",
            "code::functions::update_file::handler_tests::r4_multiline_replace_echo_shows_first_and_last_lines",
            "code::functions::update_file::handler_tests::r4_single_line_replace_echo_unchanged",
            "code::functions::update_file::handler_tests::r4_two_line_replace_echo_shows_both_lines_no_elision",
            "code::functions::update_file::handler_tests::regex_replace_introducing_newline_updates_line_count",
            "code::functions::update_file::handler_tests::s3_dot_default_false_multiline_pattern_matches_nothing",
            "code::functions::update_file::handler_tests::s3_dot_matches_newline_replaces_region_with_correct_echoes",
            "code::functions::update_file::handler_tests::s3_expect_matches_assert_absent_mismatch_routes_to_search",
            "code::functions::update_file::handler_tests::s3_expect_matches_exact_count_applies",
            "code::functions::update_file::handler_tests::s3_expect_matches_mismatch_fails_file_other_file_applies",
            "code::functions::update_file::handler_tests::s3_expect_matches_zero_actual_routes_to_search",
            "code::functions::update_file::handler_tests::s3_second_op_expect_mismatch_rolls_back_whole_file",
            "code::functions::update_file::handler_tests::success_includes_echo_not_full_body",
            "code::functions::update_file::handler_tests::times_pluralizes_match_counts",
            "code::functions::update_file::handler_tests::wire_always_emits_echo_fields",
            "code::functions::update_file::tests::bottom_up_apply_preserves_original_line_numbers",
            "code::functions::update_file::tests::build_line_echo_clamps_out_of_range_regions",
            "code::functions::update_file::tests::build_line_echo_elides_large_regions",
            "code::functions::update_file::tests::build_site_echo_clamps_out_of_range_regions",
            "code::functions::update_file::tests::duplicate_insert_at_same_line_rejected",
            "code::functions::update_file::tests::empty_ops_rejected",
            "code::functions::update_file::tests::insert_at_1_prepends",
            "code::functions::update_file::tests::insert_at_end_plus_one_appends",
            "code::functions::update_file::tests::insert_inside_remove_range_rejected",
            "code::functions::update_file::tests::insert_multiline",
            "code::functions::update_file::tests::map_through_events_sequential_frames",
            "code::functions::update_file::tests::map_through_events_shifts_only_positions_at_or_below",
            "code::functions::update_file::tests::out_of_range_remove_rejected",
            "code::functions::update_file::tests::overlapping_remove_and_update_lines_rejected",
            "code::functions::update_file::tests::r1_validator_matches_expand_semantics",
            "code::functions::update_file::tests::regex_dot_does_not_cross_newlines_by_default",
            "code::functions::update_file::tests::regex_dot_matches_newline_crosses_lines",
            "code::functions::update_file::tests::regex_expect_matches_exact_count_ok",
            "code::functions::update_file::tests::regex_expect_matches_mismatch_is_c210",
            "code::functions::update_file::tests::regex_replace_all_matches",
            "code::functions::update_file::tests::regex_replace_capture_groups",
            "code::functions::update_file::tests::regex_replace_empty_pattern_rejected",
            "code::functions::update_file::tests::regex_replace_ignore_case",
            "code::functions::update_file::tests::regex_replace_invalid_pattern_rejected",
            "code::functions::update_file::tests::regex_replace_no_match_is_noop",
            "code::functions::update_file::tests::remove_range",
            "code::functions::update_file::tests::split_file_detects_crlf",
            "code::functions::update_file::tests::split_file_no_trailing_newline_preserved",
            "code::functions::update_file::tests::update_lines_range_multiline",
            "code::functions::update_file::tests::zero_line_rejected",
        ],
        "cwd": "ide",
        "grade": [
            ["cargo", "test", "--locked", "--test", "code_update_ops", "--test", "code_golden_errors"],
            ["cargo", "test", "--locked", "--lib", "update_file"],
        ],
        "prompt": (
            "Your working directory contains `ide/`, the Rust source of the iii ide worker, and `crates/`, shared "
            "crates it builds against. Bug report: models sometimes leave out the `op` tag on an entry of `ops[]` in "
            "a `coder::update-file` request, and the whole request is rejected. Fix it: when an op has no `op` key, "
            "infer the op only if its keys fit exactly one `UpdateOp` variant (every required key of that variant "
            "present and no key outside it); otherwise reject the request with an error that names the entry as "
            "`ops[i]`. Record the inferred ops on `UpdateFileSpec` as `inferred_ops` and report them in that file's "
            "result as `notice`. The failing tests are `ide/tests/code_update_ops.rs` (its "
            "`missing_op_is_inferred_through_the_handler` pins the exact notice text) and "
            "`ide/tests/code_golden_errors.rs`; both fail to compile today. Do not change those tests. From `ide/`, "
            "`cargo test --locked --test code_update_ops --test code_golden_errors` must pass and "
            "`cargo test --locked --lib update_file` must keep passing. When they do, reply with a short summary of "
            "your change."
        ),
    },
}
# Mean cost per episode in the spike's run-03 report, keyed arm|task|level|model.
PRIORS = {
    f"{arm}|{task}|{level}|{model}": cost
    for model, rows in {
        S55: [
            ("kv", 1000, 0.33, 0.55, 2.28),
            ("kv", 4000, 0.33, 1.46, 6.02),
            ("log", 1600, 0.37, 0.98, 2.57),
            ("log", 3200, 0.38, 1.65, 4.83),
            ("needle", 24, 0.28, 0.88, 1.40),
            ("needle", 60, 0.28, 2.43, 4.51),
            ("fields", 24, 0.05, 0.23, 0.21),
        ],
        O55: [
            ("kv", 4000, 0.61, 2.49, 20.91),
            ("log", 3200, 0.57, 2.03, 9.44),
            ("needle", 60, 0.59, 5.55, 9.12),
            ("fields", 24, 0.10, 0.30, 0.31),
        ],
    }.items()
    for task, level, *costs in rows
    for arm, cost in zip(("base", "clm", "sum"), costs, strict=True)
}


def kind(task):
    return "ctx" if task in GEN else "fields" if task == "fields" else "code"


def cap_of(ep):
    return ep.get("max_cost_usd") or {"ctx": 15, "fields": 5, "code": 10}[kind(ep["task"])]


def ep_key(ep):
    return f"{ep['arm']}|{ep['task']}|{ep.get('level', 0)}|{ep['model']}"


def provider_of(ep):
    return ep.get("provider", "anthropic")


def budget_of(ep):
    """The session's context budget. ContextBench limits are o200k tokens: Claude counts more tokens for the same text
    (CLAUDE_PER_O200K), a non-Anthropic provider is taken as o200k-native (ratio 1.0)."""
    if ep.get("budget"):
        return ep["budget"]
    if kind(ep["task"]) != "ctx":
        return 65536
    return round(LIMIT * CLAUDE_PER_O200K[ep["task"]]) if provider_of(ep) == "anthropic" else LIMIT


def sid_of(ep):
    m = MTAG.get(ep["model"]) or re.sub(r"[^A-Za-z0-9.-]", "-", ep["model"])
    return f"{ep['arm']}-{ep['task']}{ep.get('level', 0)}-s{ep['seed']}-{m}{ep.get('tag', '')}"


def grid(name, provider="anthropic", model=None, model2=None):
    """The named grid. With `model`, the Sonnet slots run `model` and the Opus slots `model2` (dropped without it)."""
    eps = _grid(name)
    if model is None:
        if provider != "anthropic" or model2:
            raise SystemExit("grid: --model is required with --model2 or a provider other than anthropic")
        return eps
    if model2 == model:
        raise SystemExit("grid: --model2 must differ from --model (their episodes would share session ids)")
    if model2 is None:
        n = sum(ep["model"] == O55 for ep in eps)
        print(f"grid {name}: no --model2, dropped {n} episodes of the Opus-only groups", file=sys.stderr)
    slot = {S55: model, O55: model2}
    extra = {} if provider == "anthropic" else {"provider": provider}
    return [{**ep, "model": slot[ep["model"]], **extra} for ep in eps if slot[ep["model"]]]


def _grid(name):
    eps = []
    if name == "regression":
        for seed in (0, 1):
            for task, level in (
                ("kv", 1000),
                ("kv", 4000),
                ("log", 1600),
                ("log", 3200),
                ("needle", 24),
                ("needle", 60),
            ):
                arms = ["clm", "sum", "base"] + (["default"] if level in (1000, 1600) else [])
                eps += [{"arm": a, "task": task, "level": level, "seed": seed, "model": S55} for a in arms]
        for task, level in (("kv", 4000), ("log", 3200), ("needle", 60)):
            eps += [
                {"arm": a, "task": task, "level": level, "seed": 0, "model": O55, "max_cost_usd": 30}
                for a in ("clm", "sum", "base")
            ]
        for model in (S55, O55):
            for seed in (0, 1):
                eps += [
                    {"arm": a, "task": "fields", "level": 24, "seed": seed, "model": model, "budget": 16384}
                    for a in ("clm", "sum", "base", "default")
                ]
    elif name in ("long", "long-pilot"):
        pilot = name == "long-pilot"
        for model in (S55,) if pilot else (S55, O55):
            for seed in (0,) if pilot else (0, 1, 2):
                for task, level in (("http-brace", 0), ("ide-update-op", 0), ("fields", 40)):
                    eps.append(
                        {"arm": "clm", "task": task, "level": level, "seed": seed, "model": model, "budget": 65536}
                    )
                    eps.append({"arm": "default", "task": task, "level": level, "seed": seed, "model": model})
    else:
        raise SystemExit(f"unknown grid {name!r}: regression | long-pilot | long")
    return eps


def load_priors(dirs):
    priors, seen = dict(PRIORS), {}
    for d in dirs:
        for p in Path(d, "results").glob("*.json"):
            r = json.loads(p.read_text())
            if "cost_usd" in r:
                seen.setdefault(ep_key(r), []).append(r["cost_usd"])
    priors.update({k: statistics.mean(v) for k, v in seen.items()})
    return priors


def estimate(eps, priors):
    """Print expected cost (prior mean, or the cap when no prior) and the cap total; return (expected, cap)."""
    groups = {}
    for ep in eps:
        g = groups.setdefault(ep_key(ep), {"n": 0, "cap": 0.0, "prior": priors.get(ep_key(ep))})
        g["n"] += 1
        g["cap"] += cap_of(ep)
    expected = sum(g["n"] * g["prior"] if g["prior"] is not None else g["cap"] for g in groups.values())
    cap = sum(g["cap"] for g in groups.values())
    print("| group | n | prior $/ep | cap $ |\n|---|---|---|---|")
    for k, g in sorted(groups.items()):
        prior = "none (cap used)" if g["prior"] is None else f"{g['prior']:.2f}"
        print(f"| {k} | {g['n']} | {prior} | {g['cap']:.0f} |")
    print(f"\nepisodes: {len(eps)}; expected ~${expected:.2f}; harness caps total ${cap:.0f}")
    print("The sum arm's summariser calls are outside the harness cap (its priors include them).")
    if any(provider_of(ep) != "anthropic" for ep in eps):
        print(
            "A subscription provider may report cost_usd 0: the harness cap may then never bind, and max_turns is "
            "what bounds those episodes."
        )
    return expected, cap


def call(fn, payload, timeout=60):
    out = subprocess.run([*III, fn, "--json", json.dumps(payload)], capture_output=True, text=True, timeout=timeout)
    if out.returncode:
        raise RuntimeError(f"{fn}: {out.stderr.strip()[:500]}")
    return json.loads(out.stdout or "null")


def send(sid, message, options, first=None):
    return call("harness::send", {"session_id": sid, "message": message, "options": options, **(first or {})})


def wait(sid, turn_id, deadline_s=1800):
    end = time.time() + deadline_s
    while time.time() < end:
        st = call("harness::status", {"session_id": sid, "verbose": True})
        done = st.get("status") in ("completed", "failed", "cancelled") and not st.get("expects_wake")
        if st.get("turn_id") == turn_id and done:
            return st
        time.sleep(2)
    raise TimeoutError(f"{sid} turn {turn_id} did not finish")


def messages(sid):
    out, cursor = [], None
    while True:
        r = call("session::messages", {"session_id": sid, "limit": 500, **({"cursor": cursor} if cursor else {})})
        out += r.get("messages", [])
        cursor = r.get("next_cursor")
        if not cursor:
            return out


def strings(x):
    if isinstance(x, str):
        try:
            j = json.loads(x)
        except ValueError:
            j = None
        if isinstance(j, (dict, list)):
            yield from strings(j)
        else:
            yield x
    elif isinstance(x, dict):
        for v in x.values():
            yield from strings(v)
    elif isinstance(x, list):
        for v in x:
            yield from strings(v)


def turn_text(entries):
    """Everything the agent added during a turn: assistant text, call arguments, result text."""
    parts = []
    for e in entries:
        m = e.get("message") or {}
        if m.get("role") not in ("assistant", "function_result"):
            continue
        for b in m.get("content") or []:
            if b.get("type") == "text":
                parts += list(strings(b.get("text", "")))
            elif b.get("type") == "function_call":
                parts += list(strings(b.get("arguments")))
    return "\n".join(parts)


def last_json(text):
    """The last top-level JSON object in a reply (a model may print a draft, then the corrected answer)."""
    dec, found, i = json.JSONDecoder(), {}, 0
    while (i := text.find("{", i)) >= 0:
        try:
            obj, end = dec.raw_decode(text, i)
            if isinstance(obj, dict):
                found = obj
            i = end
        except ValueError:
            i += 1
    return {k: set(v) for k, v in found.items() if isinstance(v, list)}


def score_fields(final, truth):
    ans = last_json(final)
    tp = sum(len(set(t["fields"]) & ans.get(t["id"], set())) for t in truth)
    fp = sum(len(ans.get(t["id"], set()) - set(t["fields"])) for t in truth)
    fn = sum(len(set(t["fields"]) - ans.get(t["id"], set())) for t in truth)
    return {
        "score": round(sum(ans.get(t["id"]) == set(t["fields"]) for t in truth) / len(truth), 4),
        "precision": round(tp / max(tp + fp, 1), 3),
        "recall": round(tp / max(tp + fn, 1), 3),
    }


def answer_score(by_turn, op_turn, first_query_op, pairs, attr):
    ok = 0
    for q, (key, val) in enumerate(pairs):
        text = turn_text(by_turn.get(op_turn(first_query_op + q), []))
        got = re.findall(answer_re(attr, key), text, flags=re.S)
        ok += any(x.strip() == val for x in got)
    return ok / len(pairs)


def needle_score(window, needles):
    lines = {line.rstrip() for line in window.splitlines()}
    return sum(n in lines for n in needles) / len(needles)


def drop_events(run, sid):
    log = run / "logs" / "provider-anthropic.log"
    if not log.exists():
        return None
    mark = f"session_id={sid} "
    with log.open(errors="replace") as f:
        lines = (ANSI.sub("", line) for line in f)  # a coloured log splits "session_id=" with escapes
        return sum(1 for line in lines if DROP in line and mark in line)


def strategy_status(arm, sid, budget):
    """context-<arm>::status in the shape report.py reads; raises on anything else instead of grading blanks. clm may
    enforce less than `budget`: it lowers the budget to the model's usable room (the row keeps the status)."""
    st = call(f"context-{arm}::status", {"session_id": sid})
    stats, got = st.get("stats"), st.get("budget_tokens")
    fits = isinstance(got, int) and 0 < got <= budget if arm == "clm" else got == budget
    if not (st.get("found") and fits and isinstance(stats, dict) and "peak_tc" in stats):
        want = f"0 < budget_tokens <= {budget}" if arm == "clm" else f"budget_tokens: {budget}"
        raise RuntimeError(f"{sid}: context-{arm}::status is not {{found, {want}, stats}}: {st}")
    return st


def git(*args):
    return subprocess.run(["git", "-C", str(REPO), *args], check=True, capture_output=True).stdout


def untar(blob, root, exclude=()):
    # -m: files get the extraction time, not the commit time, so cargo never reuses a build of an edited file
    subprocess.run(["tar", "-x", "-m", "-C", str(root), *[f"--exclude={x}" for x in exclude]], input=blob, check=True)


def build_snapshot(name, root):
    t = LONG[name]
    untar(git("archive", t["base"], *t["paths"]), root, t.get("exclude", ()))
    for f in t["tests"]:
        (root / f).write_bytes(git("show", f"{t['fix']}:{f}"))
    for f in t.get("stubs", ()):
        (root / f).parent.mkdir(parents=True, exist_ok=True)
        (root / f).touch()


TEST_MOD = "#[cfg(test)]\nmod tests"


def splice_test_module(current, fix):
    """The fix's `mod tests` (through the end of the file) after the solution's own prefix."""
    tail = fix[fix.index(TEST_MOD) :]
    i = current.find(TEST_MOD)
    head = current[:i] if i >= 0 else current + ("" if current.endswith("\n") or not current else "\n")
    return head + tail


def missing_tests(stdout, required):
    """Required tests without a `test <name> ... ok` line (failed, ignored and absent tests are all missing)."""
    ok = set(re.findall(r"^test (.+?)(?: - should panic)? \.\.\. ok$", stdout, re.M))
    return [n for n in required if n not in ok]


# The grader runs model-written code (tests, build scripts) unconfined: it gets no API keys or other driver env.
GRADE_ENV = ("PATH", "HOME", "CARGO_HOME", "RUSTUP_HOME", "LANG")


def cargo_config():
    """The user's cargo config files (None when absent). Cargo applies a config there (rustflags, runner, alias) to
    every grade. Episode execs cannot write it (boot.sh's exec_writable), but anything else on the host can."""
    home = Path(os.environ.get("CARGO_HOME") or Path.home() / ".cargo")
    return {n: (home / n).read_bytes() if (home / n).is_file() else None for n in ("config", "config.toml")}


CARGO_CONFIG = cargo_config()  # as the run started


def run_cargo(cmd, cwd, target, timeout=900):
    """(returncode, stdout, stderr), building in `target`. A timeout kills the whole process group and returns 124."""
    env = {k: os.environ[k] for k in GRADE_ENV if k in os.environ} | {"CARGO_TARGET_DIR": str(target)}
    p = subprocess.Popen(
        cmd, cwd=cwd, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True
    )
    try:
        out, err = p.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(p.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        out, err = p.communicate()
        return 124, out or "", err or ""
    return p.returncode, out, err


TOOLCHAIN = ("rust-toolchain", "rust-toolchain.toml")  # rustup reads either, in the cwd and every parent


def first_link(root, f):
    """The first path from `root` down to `root / f` that is a symlink (lstat: never followed), or None."""
    p = root
    for part in Path(f).parts:
        p = p / part
        if p.is_symlink():
            return str(p.relative_to(root))
    return None


def restore_protected(t, root):
    """Put back what steers the graded tests (unit-test modules, tests/, manifests, build.rs, the toolchain file) from
    the fix commit and drop `.cargo/` dirs, `build.rs` and toolchain files that the fix commit does not have. Returns
    why the attempt must not be graded, or None: a path it would write (or a parent dir) is a symlink, so it writes
    nothing, or the snapshot held cargo config files (`.cargo/config{,.toml}`) that the fix commit does not have."""
    fix, crate = t["fix"], t["cwd"]
    in_fix = set(git("ls-tree", "-r", "-z", "--name-only", fix, *t["paths"]).decode().split("\0"))
    files = [f for f in (f"{crate}/Cargo.toml", f"{crate}/Cargo.lock", f"{crate}/build.rs", *TOOLCHAIN) if f in in_fix]
    tests = [f for f in in_fix if f.startswith(f"{crate}/tests/")]
    restored = {*t.get("splice_tests", ()), *t["tests"], *files, *tests}
    # the grader runs unconfined: a link would let it read and write files outside the snapshot
    if linked := sorted({x for f in restored if (x := first_link(root, f))}):
        return f"the snapshot links graded paths out of place: {', '.join(linked)}"
    for f in t.get("splice_tests", ()):
        cur = (root / f).read_text() if (root / f).exists() else ""
        (root / f).parent.mkdir(parents=True, exist_ok=True)
        (root / f).write_text(splice_test_module(cur, git("show", f"{fix}:{f}").decode()))
    untar(git("archive", fix, "--", f"{crate}/tests", *files), root)
    planted = []
    for d, dirs, names in os.walk(root):
        rel = Path(d).relative_to(root)
        dirs[:] = [x for x in dirs if x != "target"]
        for x in [x for x in dirs if x == ".cargo"]:
            dirs.remove(x)
            here = Path(d) / x
            config = [str(rel / x / n) for n in ("config", "config.toml") if (here / n).exists()]
            planted += [f for f in config if f not in in_fix]
            if not any(f.startswith(f"{rel / x}/") for f in in_fix):
                here.unlink() if here.is_symlink() else shutil.rmtree(here)
        for x in ("build.rs", *TOOLCHAIN):
            if x in names and str(rel / x) not in in_fix:
                (Path(d) / x).unlink()
    return f"the snapshot holds cargo config the fix commit does not have: {', '.join(planted)}" if planted else None


CONFIG_CHANGED = "~/.cargo/config{,.toml} changed since the run started"


def refuse(why, log=()):
    """grade_code's answer for an episode it does not grade."""
    return False, [], "\n".join([*log, f"not graded: {why}"]), why


def grade_code(name, root):
    """Restore the protected files, then run the grading commands: (passed, missing required tests, log, refusal).
    `refusal` (None when graded) says why the episode was not graded: the user's cargo config differs from the one the
    run started with, before cargo runs or after it (a parallel episode may write it meanwhile), or restore_protected
    refused the snapshot (a graded path linked out of place, or a planted cargo config, which it deleted)."""
    root = Path(root).absolute()  # cargo resolves a relative CARGO_TARGET_DIR against its own cwd
    if cargo_config() != CARGO_CONFIG:
        return refuse(CONFIG_CHANGED)
    t = LONG[name]
    if why := restore_protected(t, root):
        return refuse(why)
    cwd, log, stdout, ok = root / t["cwd"], [], "", True
    # root is <RUN>/workspace/<sid> or <RUN>/verify/<task>. Cargo builds in a dir the episode's Landlock scope cannot
    # write, never in the <crate>/target the episode built (a binary planted there would run as the graded test).
    target = root.parents[1] / "grade-target" / name
    target.mkdir(parents=True, exist_ok=True)
    with (target / "grade.lock").open("w") as lock:
        # Grades of a task share the dir, one at a time. Cargo reuses a build whose sources are older than it (an
        # episode that started earlier has older files), so date this workspace's files now: cargo then rebuilds
        # its crates and reuses only the registry deps.
        fcntl.flock(lock, fcntl.LOCK_EX)
        for d, dirs, names in os.walk(root):
            dirs[:] = [x for x in dirs if x != "target"]
            for x in names:
                os.utime(Path(d) / x, follow_symlinks=False)
        for cmd in t["grade"]:
            rc, out, err = run_cargo(cmd, cwd, target)
            stdout += out
            log.append(f"$ {' '.join(cmd)} -> rc={rc}{' (timeout)' if rc == 124 else ''}\n{(out + err)[-3000:]}")
            ok = ok and rc == 0
            if rc == 124:
                break
    if cargo_config() != CARGO_CONFIG:
        return refuse(f"{CONFIG_CHANGED}, during this grade", log)
    missing = missing_tests(stdout, t["required"])
    return ok and not missing, missing, "\n".join(log), None


def verify_long(run):
    good = True
    for name, t in LONG.items():
        root = run / "verify" / name
        shutil.rmtree(root, ignore_errors=True)
        root.mkdir(parents=True)
        build_snapshot(name, root)
        before, _, log_before, _ = grade_code(name, root)
        for f in t["solution"]:
            (root / f).write_bytes(git("show", f"{t['fix']}:{f}"))
        after, missing, log_after, _ = grade_code(name, root)
        print(json.dumps({"task": name, "fails_before": not before, "passes_after": after, "missing": missing}))
        if before or not after:
            good = False
            print(log_before if before else log_after)
    return 0 if good else 1


def episode(run, ep):
    arm, task, model, level = ep["arm"], ep["task"], ep["model"], ep.get("level", 0)
    sid = sid_of(ep)
    res_path = run / "results" / f"{sid}.json"
    if res_path.exists():
        return json.loads(res_path.read_text())
    root = run / "workspace" / sid
    if root.exists():
        raise RuntimeError(f"{sid}: stale attempt (workspace exists, no result); use a new tag")
    root.mkdir(parents=True)
    k, provider, budget = kind(task), provider_of(ep), budget_of(ep)
    meta = {} if arm == "default" else {"context_strategy": arm, "context_budget_tokens": budget}
    mirror = f".clm/{sid}.ctx"
    first = {"model": model, "provider": provider, "session": {"title": sid, "kind": "automation", "metadata": meta}}
    per = {"max_turns": {"ctx": 40, "fields": 80, "code": 200}[k], "max_output_tokens": 16000}
    per["metadata"] = {"fs_scope": {"root": str(root)}}
    allow = ["shell::exec"] + {
        "ctx": [],
        "fields": ["coder::read-file"],
        "code": ["coder::read-file", "coder::search", "coder::update-file", "coder::create-file"],
    }[k]
    opts = {
        **per,
        "system_prompt": SYSTEM,
        "system_prompt_strategy": "override",
        "thinking_level": ep.get("effort", "medium"),
        "max_cost_usd": cap_of(ep),
        "functions": {"allow": allow, "expose": "native"},
    }
    t0 = time.time()
    if k == "ctx":
        g = gen(task, level, ep["seed"])
        msgs = [g["instruction"] + "\n\n" + skill(arm, task, mirror, budget), *g["ops"], END]
    elif k == "fields":
        truth = [t for t in TRUTH if level == 40 or t["scope"] == "ide"]
        dirs = ["ide/src", "ide/Cargo.toml"] + (["harness/src", "harness/Cargo.toml"] if level == 40 else [])
        untar(git("archive", FIELDS_SNAPSHOT, *dirs), root)
        what = "two iii workers (ide and harness)" if level == 40 else "an iii worker (ide)"
        top = "`ide/` and `harness/`" if level == 40 else "`ide/`"
        fns = " ".join(sorted(t["id"] for t in truth))
        msgs = [FIELDS_TASK.format(dirs=top, what=what, n=len(truth), fns=fns) + "\n\n" + SHELL_HINT]
    else:
        build_snapshot(task, root)
        msgs = [LONG[task]["prompt"] + "\n\n" + SHELL_HINT]

    turns, stops, status, err = [], [], "completed", None
    for i, m in enumerate(msgs):
        r = send(sid, m, opts if i == 0 else per, first if i == 0 else None)
        st = wait(sid, r["turn_id"], deadline_s=1800 if k == "ctx" else 5400)
        if i == 0:
            cs = st.get("context_strategy") or {}
            if arm != "default" and not cs.get("effective"):
                raise RuntimeError(f"{sid}: strategy not effective: {cs}")
            if arm == "clm" and not (root / mirror).exists():
                raise RuntimeError(f"{sid}: no mirror at {mirror} (does safe_id change this session id?)")
            if arm != "default":
                strategy_status(arm, sid, budget)
        turns.append(r["turn_id"])
        stops.append(st.get("stop_reason"))
        if st.get("status") != "completed":
            status, err = st.get("status"), st.get("result_error")
            break

    entries = messages(sid)
    out = {"sid": sid, **{x: ep.get(x) for x in ("arm", "task", "level", "seed", "model")}, "tag": ep.get("tag", "")}
    out["provider"] = provider
    out.update({"status": status, "error": err, "budget": budget if arm != "default" else None})
    if k == "ctx":
        by_turn = {}
        for e in entries:
            by_turn.setdefault((e.get("origin") or {}).get("turn_id"), []).append(e)

        def op_turn(op):
            return turns[op + 1] if op + 1 < len(turns) else None

        t = g["truth"]
        if t["kind"] == "kv":
            score = answer_score(by_turn, op_turn, t["first_query_op"], t["gets"], "key")
        elif t["kind"] == "log":
            score = answer_score(by_turn, op_turn, t["first_query_op"], t["answers"], "qid")
        else:
            window = root / mirror if arm == "clm" else run / "arms" / "window" / f"{sid}.txt"
            score = needle_score(window.read_text(errors="replace"), t["needles"])
        out.update({"score": round(score, 4), "ops_total": len(g["ops"]), "end_sent": len(turns) == len(msgs)})
        out["ops_delivered"] = min(max(0, len(turns) - 1), len(g["ops"]))
        out["max_turns_hits"] = sum(1 for x in stops if x == "max_turns")
    elif k == "fields":
        final = ""
        for e in entries:
            m = e.get("message") or {}
            if m.get("role") == "assistant":
                text = "".join(b.get("text", "") for b in m.get("content") or [] if b.get("type") == "text").strip()
                final = text or final
        out.update(score_fields(final, truth))
    else:
        passed, missing, log, refused = grade_code(task, root)
        out.update({"score": 1.0 if passed else 0.0, "missing_tests": missing})
        if refused:  # not graded: report.py counts it apart and --check fails until it is cleared (README)
            out.update({"grade_refused": True, "grade_refused_reason": refused})
        (run / "results" / f"{sid}.grade.log").write_text(log)

    usage = [(e.get("message") or {}).get("usage") or {} for e in entries]
    usage = [u for u in usage if u]
    strategy = {} if arm == "default" else strategy_status(arm, sid, budget)
    summary_cost = (strategy.get("stats") or {}).get("summary_cost_usd") or 0
    out.update(
        {
            "cost_usd": round(sum(u.get("cost_usd") or 0 for u in usage) + summary_cost, 4),
            "llm_calls": len(usage),
            "input": sum(u.get("input") or 0 for u in usage),
            "cache_read": sum(u.get("cache_read") or 0 for u in usage),
            "cache_write": sum(u.get("cache_write") or 0 for u in usage),
            "output": sum(u.get("output") or 0 for u in usage),
            "peak_prompt": max(
                ((u.get("input") or 0) + (u.get("cache_read") or 0) + (u.get("cache_write") or 0) for u in usage),
                default=0,
            ),
            "strategy": strategy,
            # the thinking-drop warning is provider-anthropic's; other providers' drops are unknown (report: n/a)
            "drops": drop_events(run, sid) if provider == "anthropic" else None,
            "wall_s": round(time.time() - t0),
        }
    )
    res_path.write_text(json.dumps(out, indent=1))
    return out


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    g = sub.add_parser("grid")
    g.add_argument("name")
    g.add_argument("--provider", default="anthropic")
    g.add_argument("--model", help="runs the Sonnet slots (default: the Claude models as listed)")
    g.add_argument("--model2", help="runs the Opus slots; without it they are dropped")
    e = sub.add_parser("estimate")
    e.add_argument("episodes")
    e.add_argument("--prior", nargs="*", default=[])
    sub.add_parser("verify-long").add_argument("run")
    r = sub.add_parser("run")
    r.add_argument("run")
    r.add_argument("episodes")
    r.add_argument("--workers", type=int, default=6)
    r.add_argument("--prior", nargs="*", default=[])
    r.add_argument("--yes", action="store_true", help="spend money: run after printing the estimate")
    a = ap.parse_args(argv)
    if a.cmd == "grid":
        print(json.dumps(grid(a.name, a.provider, a.model, a.model2), indent=1))
        return 0
    if a.cmd == "verify-long":
        return verify_long(Path(a.run).resolve())
    eps = json.loads(Path(a.episodes).read_text())
    if a.cmd == "run":  # episodes with a result are skipped, so they are not priced either
        done = {p.stem for p in Path(a.run, "results").glob("*.json")}
        eps = [ep for ep in eps if sid_of(ep) not in done]
    _, cap = estimate(eps, load_priors(a.prior))
    if a.cmd == "estimate":
        return 0
    if not a.yes:
        print(f"Not started. Re-run with --yes to run {len(eps)} episodes (harness caps total ${cap:.0f}).")
        return 2
    run = Path(a.run).resolve()
    (run / "results").mkdir(parents=True, exist_ok=True)

    def safe(ep):
        try:
            return episode(run, ep)
        except Exception as ex:  # one broken episode must not stop the batch
            return {"episode": ep, "sid": sid_of(ep), "crash": repr(ex)[:500]}

    with ThreadPoolExecutor(a.workers) as pool, (run / "episodes.out").open("a") as log:
        for res in pool.map(safe, eps):
            line = json.dumps(res)
            print(line, flush=True)
            log.write(line + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
