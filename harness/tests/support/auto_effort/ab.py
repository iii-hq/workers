"""Level cost A/B: python3 -I ab.py <out_dir> --sha <commit> [--data <stack data dir>] [--pilot] [--run]

ab_tasks.md preregisters the tasks, checks, arms and stop rules. --sha must predate this file, whose checks would hand a
session its answers. A run builds a standalone repo of --sha's files under out_dir/trees (R15), plants task c's bug,
sends the task on a new e2e session scoped to that tree, waits for the turn, checks it and removes the tree. Its answer
is the session's last assistant text and its cost the summed units of all its assistant messages, read from the session
file under --data. --pilot runs each task once on claude-opus-5-5 at medium (R13). Every call writes
out_dir/ab_prompts.jsonl, the A/B prompts for judge.py --rows (R4); without --run it prints the run count and an
estimated cost. A --run rewrites ab.jsonl after each run and ratios.json at the end. It skips the runs ab.jsonl holds,
except those that errored before spending or whose usage never landed (spent()), so a --run after --pilot adds the
rest and only those go again; 3 errored runs in a row stop it. An errored run is a fail.
"""

import argparse
import contextlib
import hashlib
import io
import json
import shutil
import statistics
import subprocess
import sys
import tarfile
import tempfile
from collections import defaultdict
from functools import partial
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import cache_probe  # noqa: E402
import common  # noqa: E402
import extract  # noqa: E402

REPO = cache_probe.REPO
PKG = "harness/tests/support/auto_effort"
MODELS = ("claude-opus-5-5", "claude-sonnet-5-5")
LEVELS = common.LEVELS  # R13: xhigh too, so G3 can price a pick or a corpus row at xhigh
RUNS = 3
TASKS = {
    "a": "What does `confident_choice` in harness/src/reconcile.rs return when the choice is `none`?",
    "b": "Rename `units` to `cost_units_of` in harness/tests/support/auto_effort/common.py and update every caller.",
    "c": "test_p95 in harness/tests/support/auto_effort fails; find and fix the bug without changing the test.",
    "d": "Why does a harness send that omits thinking_level keep the previous level? Cite file:line.",
}
FUNCTIONS = (  # file read and edit, a shell for the tests, and contracts, which a narrowed policy denies unless listed
    "coder::read-file",
    "coder::search",
    "coder::tree",
    "coder::list-folder",
    "coder::update-file",
    "shell::exec",
    "engine::functions::info",
)
# -B: a .pyc written for the planted p95 would pass for fresh after a same-size fix within the same second
TEST_CMD = (sys.executable, "-I", "-B", "-m", "unittest", "discover", "-s", PKG, "-t", PKG, "-p", "test_*.py")
BUG = ("// 100) - 1]", "// 100) - 2]")  # analyze.p95, one rank low
TURN_S = 1800  # a turn still running after this is stopped, so it stops spending
RUN_UNITS = 100_000  # ponytail: between the corpus's child tasks and opus top turns; the pilot measures it
NO_USAGE = "no usage after 10 s"  # converse: the turn completed, but its last message's usage never landed


def spent(row: dict) -> bool:
    """Whether a run is a cost sample, kept and never re-run: no error, or an error after it spent (R17). Timeouts
    cluster at high effort, so dropping or replacing them would bias the ratios low. One that spent nothing, a turn
    that never started, is no sample and goes again, as does a NO_USAGE one: its turn completed, but its cost is
    short of its last message and its fail is the session file's, not the turn's."""
    return not row.get("error") or (row["cost_units"] > 0 and row["error"] != NO_USAGE)


def ratios(rows: list[dict]) -> dict[str, dict[str, float]]:
    """Mean cost_units per (model, level) over the spent() runs, passing and failing alike, over the model's medium
    mean."""
    costs = defaultdict(list)
    for row in filter(spent, rows):
        costs[row["model"], row["level"]].append(row["cost_units"])
    means = {key: statistics.fmean(cost) for key, cost in costs.items()}
    out = {}
    for (model, level), mean in means.items():
        if medium := means.get((model, "medium")):
            out.setdefault(model, {})[level] = mean / medium
    return out


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def tests_pass(repo: Path, *args: str) -> bool:
    """TEST_CMD in repo, with extra unittest arguments."""
    try:
        return subprocess.run([*TEST_CMD, *args], cwd=repo, capture_output=True, timeout=600).returncode == 0
    except subprocess.TimeoutExpired:
        return False


def plant(task: str, repo: Path) -> None:
    """Task c's bug, and the test file's sha256 in <repo>.test-sha256, outside the session's scope (R6)."""
    if task != "c":
        return
    pkg = repo / PKG
    Path(f"{repo}.test-sha256").write_text(sha256(pkg / "test_auto_effort.py"), encoding="utf-8")
    source = pkg / "analyze.py"
    text = source.read_text(encoding="utf-8")
    if text.count(BUG[0]) != 1:
        raise ValueError(f"{source}: no single p95 line to plant the bug in")
    source.write_text(text.replace(*BUG), encoding="utf-8")


def check(task: str, answer: str, repo: Path) -> bool:
    """Whether a run passed its task's check in ab_tasks.md: a and d read the answer, b and c the tree."""
    pkg = repo / PKG
    if task == "a":
        return "None" in answer
    if task == "b":
        source = pkg / "common.py"
        return source.is_file() and "def units(" not in source.read_text(encoding="utf-8") and tests_pass(repo)
    if task == "c":  # test_p95 only: the whole suite would run test_plant_then_check_fix, and so this check, again
        test, planted = pkg / "test_auto_effort.py", Path(f"{repo}.test-sha256").read_text(encoding="utf-8")
        return test.is_file() and sha256(test) == planted and tests_pass(repo, "-k", "test_p95")
    return "inherit_prior_reasoning" in answer and "reasoning_fields_omitted" in answer


def git(*args: str) -> str:
    return subprocess.run(["git", "-C", str(REPO), *args], check=True, capture_output=True, encoding="utf-8").stdout


def build_tree(repo: Path, sha: str, tree: Path) -> None:
    """A standalone repo at tree holding sha's files in one commit (R15). Its git dir is its own .git, so a session in
    it reaches no other branch, stash or hook of repo."""
    archive = subprocess.run(["git", "-C", str(repo), "archive", "--format=tar", sha], check=True, capture_output=True)
    # ponytail: the whole archive (140 MB at 38193a004) sits in memory; stream it through Popen and mode "r|" if needed
    with tarfile.open(fileobj=io.BytesIO(archive.stdout)) as tar:
        tar.extractall(tree, filter="data")
    commit = ("-c", "user.name=ab", "-c", "user.email=ab@localhost", "commit", "-qm", "base")
    for args in (("init", "-q"), ("add", "-A"), commit):
        subprocess.run(["git", "-C", str(tree), *args], check=True, capture_output=True)


def replies(path: Path) -> list[dict]:
    """The session's assistant messages; [] while the file is missing or a line is caught mid-append."""
    try:
        return [m for m in extract.messages(common.read_jsonl(path)) if m.get("role") == "assistant"]
    except (OSError, ValueError):
        return []


def settled(path: Path) -> list[dict] | None:
    """replies(path) once the last one carries its final usage; None until then."""
    steps = replies(path)
    return steps if steps and steps[-1].get("usage") else None


def converse(payload: dict, sessions: Path) -> tuple[list[dict], str | None]:
    """Send one turn and wait as cache_probe does: the session's assistant messages, and why the turn did not
    complete, if it did not."""
    try:
        sent = common.trigger("harness::send", payload, timeout_s=30)
    except common.TriggerError as err:
        return [], str(err)
    sid, tid = sent["session_id"], sent["turn_id"]
    why = "interrupted"  # a Ctrl-C lands in the wait, where a run spends its time: the turn is stopped all the same
    try:
        why = cache_probe.wait(sid, tid, within=TURN_S)
    except common.TriggerError as err:
        why = str(err)
    finally:
        if why:  # a turn still running keeps spending; stopping one that ended changes nothing
            with contextlib.suppress(common.TriggerError):
                common.trigger("harness::stop", {"session_id": sid, "turn_id": tid}, timeout_s=10)
    path = sessions / f"{sid}.jsonl"
    steps = cache_probe.poll(partial(settled, path), 0.5, 10)
    if steps is None:
        return replies(path), why or NO_USAGE
    return steps, why


def measure(item: tuple[str, str, str, int], sha: str, trees: Path, sessions: Path) -> dict:
    """One ab.jsonl row: the task in a standalone repo of sha's files, on a new session scoped to it."""
    model, level, task, run = item
    # a name of its own for every run, a retry too, so a tree a failed removal left behind never blocks a later one
    tree = Path(tempfile.mkdtemp(prefix=f"ab-{model}-{level}-{task}-{run}-", dir=trees))
    try:
        build_tree(REPO, sha, tree)
        plant(task, tree)
        options = {
            "thinking_level": level,
            "functions": {"allow": list(FUNCTIONS)},
            "metadata": {"fs_scope": {"root": str(tree)}},
        }
        payload = {
            "message": TASKS[task],
            "model": model,
            "provider": "anthropic",
            "session": {"kind": "e2e"},
            "options": options,
        }
        steps, error = converse(payload, sessions)
        # an errored run is a fail: a stopped or failed turn did not finish the task, whatever its tree or text holds
        passed = not error and check(task, extract.text(steps[-1]) if steps else "", tree)
    finally:
        try:
            shutil.rmtree(tree)
        except OSError as err:  # a leftover tree costs disk only, never the paid row
            print(f"could not remove {tree}: {err}", file=sys.stderr)
        Path(f"{tree}.test-sha256").unlink(missing_ok=True)
    cost = sum((common.units(m.get("usage")) for m in steps), 0.0)
    return {
        "model": model,
        "level": level,
        "task": task,
        "run": run,
        "pass": passed,
        "cost_units": cost,
        "steps": len(steps),
        "error": error,
    }


def run_key(row: dict) -> tuple[str, str, str, int]:
    return row["model"], row["level"], row["task"], row["run"]


def main(argv: list[str]) -> None:
    parser = argparse.ArgumentParser(prog="ab.py")
    parser.add_argument("out_dir", type=Path)
    parser.add_argument("--sha", required=True, help="the commit whose files every run's tree holds")
    parser.add_argument("--data", type=Path, help="the stack's data dir, which holds session-manager/; --run needs it")
    parser.add_argument("--pilot", action="store_true", help=f"only {MODELS[0]} at medium, each task once")
    parser.add_argument("--run", action="store_true", help="drive the stack and write ab.jsonl and ratios.json")
    args = parser.parse_args(argv)
    if args.run and not (args.data and (args.data / "session-manager").is_dir()):
        parser.error("--run needs --data <dir> holding session-manager/")
    try:  # resolved once, so every run checks out the same commit
        sha = git("rev-parse", "--verify", "--quiet", f"{args.sha}^{{commit}}").strip()
    except subprocess.CalledProcessError:
        parser.error(f"--sha {args.sha} names no commit")
    with contextlib.suppress(subprocess.CalledProcessError):
        git("cat-file", "-e", f"{sha}:{PKG}/ab.py")
        parser.error(f"--sha {args.sha} holds ab.py, whose checks and tests would hand the sessions their answers")
    prompts = [
        {"id": f"ab-{model}-{task}", "slice": "ab", "model": model, "level": "medium", "context": "", "message": text}
        for model in MODELS
        for task, text in TASKS.items()
    ]
    common.write_jsonl(args.out_dir / "ab_prompts.jsonl", prompts)
    if args.pilot:  # R13: the read, edit and exec paths once each, before anything is paid at other levels
        items = [(MODELS[0], "medium", task, 0) for task in TASKS]
    else:  # levels innermost, so drift over the hours of a run hits them alike
        items = [(m, lv, t, r) for m in MODELS for t in TASKS for r in range(RUNS) for lv in LEVELS]
    path = args.out_dir / "ab.jsonl"
    rows = common.read_jsonl(path) if path.exists() else []
    done = {run_key(row) for row in filter(spent, rows)}
    todo = [item for item in items if item not in done]
    if not args.run:
        print(f"{len(items)} runs at {sha[:12]}, {len(todo)} still to run")
        for model in dict.fromkeys(item[0] for item in todo):  # units are per model, so never summed across them
            n = sum(item[0] == model for item in todo)
            print(f"{model}: {n} runs, about {n * RUN_UNITS:,} units if a run costs about {RUN_UNITS:,}")
        return
    trees = args.out_dir.resolve() / "trees"  # absolute: fs_scope wants a root
    trees.mkdir(parents=True, exist_ok=True)
    errored = 0  # runs in a row with an error: a broken setup (a wrong --data, a down stack) must not pay for all
    for item in todo:
        row = measure(item, sha, trees, args.data / "session-manager")
        rows = [old for old in rows if run_key(old) != item] + [row]  # a retry replaces a row that spent nothing
        common.write_jsonl(path, rows)  # after each run, so a stopped --run keeps what it paid for
        outcome = f"{'pass' if row['pass'] else 'fail'}, {row['cost_units']:,.0f} units, {row['steps']} steps"
        print(f"{' '.join(map(str, item))}: {outcome}" + (f"; {row['error']}" if row["error"] else ""), flush=True)
        errored = errored + 1 if row["error"] else 0
        if errored == 3:
            sys.exit(f"stopped: 3 runs in a row errored ({row['error']}); {path} keeps the runs so far")
    measured = ratios(rows)
    (args.out_dir / "ratios.json").write_text(json.dumps(measured, indent=2) + "\n", encoding="utf-8")
    for model, levels in measured.items():
        print(f"{model}: " + ", ".join(f"{level} {ratio:.2f}" for level, ratio in levels.items()))
    print(f"wrote {path} and {args.out_dir / 'ratios.json'}")


if __name__ == "__main__":
    main(sys.argv[1:])
