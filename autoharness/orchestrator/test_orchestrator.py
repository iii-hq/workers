"""
Integration tests for autoharness orchestrator.

Requires iii-engine running at localhost:3111 (REST) / localhost:49134 (WS).
Start with: iii --config iii-config.yaml
Then:       uv run orchestrator.py &
Then:       python test_orchestrator.py
"""

import json
import os
import sys
import time
import urllib.request

BASE = "http://localhost:3111"
TAG = f"test-{int(time.time())}"
PASS = 0
FAIL = 0


def api(path, data=None, method="POST"):
    url = f"{BASE}{path}"
    body = json.dumps(data).encode() if data else None
    headers = {"Content-Type": "application/json"} if body else {}
    token = os.environ.get("AUTOHARNESS_AUTH_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(url, data=body, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            return json.loads(resp.read())
    except urllib.error.HTTPError as e:
        return json.loads(e.read())
    except Exception as e:
        return {"error": str(e)}


def check(name, condition, detail=""):
    global PASS, FAIL
    if condition:
        PASS += 1
        print(f"  PASS  {name}")
    else:
        FAIL += 1
        print(f"  FAIL  {name} — {detail}")


def test_experiment_lifecycle():
    print("\n--- Experiment Lifecycle ---")

    r = api("/api/experiment/setup", {"tag": TAG})
    body = r.get("body", r)
    check("setup returns tag", body.get("tag", {}).get("name") == TAG, body)

    r = api("/api/experiment/setup", {"tag": TAG})
    body = r.get("body", r)
    check("duplicate tag rejected", "error" in body or r.get("statusCode") == 400, body)

    r = api("/api/experiment/register", {
        "tag": TAG,
        "hypothesis": "Adding file_read tool helps code inspection tasks",
        "description": "add file_read tool",
        "category": "tools",
        "commit_sha": "abc1234",
        "diff_summary": "Added read_file tool",
    })
    body = r.get("body", r)
    exp_id = body.get("experiment_id", "")
    check("register returns experiment_id", exp_id.startswith("exp-"), body)

    r = api("/api/experiment/complete", {
        "experiment_id": exp_id,
        "passed": 7,
        "total_tasks": 10,
        "aggregate_score": 0.75,
        "task_scores": {"task1": 1.0, "task2": 0.5, "task3": 1.0},
        "duration_seconds": 120.5,
        "tokens_used": 45000,
        "estimated_cost": 0.15,
    })
    body = r.get("body", r)
    check("first complete is always keep", body.get("status") == "keep", body)
    check("improved is true for first", body.get("improved") is True, body)

    r = api("/api/experiment/register", {
        "tag": TAG,
        "hypothesis": "Worse experiment",
        "description": "break everything",
        "category": "other",
        "commit_sha": "def5678",
    })
    body = r.get("body", r)
    exp_id_2 = body.get("experiment_id", "")

    r = api("/api/experiment/complete", {
        "experiment_id": exp_id_2,
        "passed": 5,
        "total_tasks": 10,
        "aggregate_score": 0.50,
        "task_scores": {"task1": 0.0, "task2": 0.5, "task3": 1.0},
        "duration_seconds": 100,
    })
    body = r.get("body", r)
    check("worse experiment is discard", body.get("status") == "discard", body)
    check("action is git_reset", body.get("action") == "git_reset", body)

    r = api("/api/experiment/register", {
        "tag": TAG,
        "hypothesis": "Better experiment",
        "description": "improve everything",
        "category": "system_prompt",
        "commit_sha": "ghi9012",
    })
    body = r.get("body", r)
    exp_id_3 = body.get("experiment_id", "")

    r = api("/api/experiment/complete", {
        "experiment_id": exp_id_3,
        "passed": 8,
        "total_tasks": 10,
        "aggregate_score": 0.85,
        "task_scores": {"task1": 1.0, "task2": 1.0, "task3": 0.5},
        "duration_seconds": 130,
    })
    body = r.get("body", r)
    check("better experiment is keep", body.get("status") == "keep", body)
    check("delta_passed is +1", body.get("delta_passed") == 1, body)

    return exp_id, exp_id_2, exp_id_3


def test_crash_tracking():
    print("\n--- Crash Tracking ---")

    r = api("/api/experiment/register", {"tag": TAG, "description": "crash1", "category": "other"})
    eid = r.get("body", r).get("experiment_id")
    r = api("/api/experiment/crash", {"experiment_id": eid, "error": "OOM"})
    body = r.get("body", r)
    check("crash recorded", body.get("status") == "crash", body)
    check("consecutive=1", body.get("consecutive_crashes") == 1, body)
    check("should_abort=false", body.get("should_abort") is False, body)

    for i in range(2):
        r = api("/api/experiment/register", {"tag": TAG, "description": f"crash{i+2}", "category": "other"})
        eid = r.get("body", r).get("experiment_id")
        r = api("/api/experiment/crash", {"experiment_id": eid, "error": "OOM again"})
    body = r.get("body", r)
    check("3 crashes triggers abort", body.get("should_abort") is True, body)


def test_query_endpoints(exp_id, exp_id_2, exp_id_3):
    print("\n--- Query Endpoints ---")

    r = api("/api/experiment/best", {"tag": TAG})
    body = r.get("body", r)
    check("best returns exp_id_3", body.get("best", {}).get("experiment_id") == exp_id_3, body)
    check("best passed=8", body.get("best", {}).get("passed") == 8, body)

    r = api("/api/experiment/history", {"tag": TAG, "status": "keep"})
    body = r.get("body", r)
    kept = body.get("experiments", [])
    check("history returns 2 kept", len(kept) == 2, f"got {len(kept)}")

    r = api("/api/experiment/near-misses", {"tag": TAG})
    body = r.get("body", r)
    check("near-misses endpoint works", "near_misses" in body, body)


def test_search_functions():
    print("\n--- Search Strategy ---")

    r = api("/api/search/strategy", {"tag": TAG})
    body = r.get("body", r)
    check("strategy returns mode", "mode" in body, body)

    r = api("/api/search/suggest", {"tag": TAG})
    body = r.get("body", r)
    check("suggest returns strategy", "strategy" in body, body)
    check("suggest returns suggestions", isinstance(body.get("suggestions"), list), body)
    check("suggest returns categories", "category_stats" in body, body)

    r = api("/api/search/set-strategy", {"tag": TAG, "mode": "exploit", "reason": "test"})
    body = r.get("body", r)
    check("set-strategy works", body.get("mode") == "exploit", body)


def test_harness_functions():
    print("\n--- Harness Management ---")

    r = api("/api/harness/read", method="GET")
    body = r.get("body", r)
    check("read returns content", "content" in body, body)
    check("read returns line count", body.get("lines", 0) > 0, body)

    r = api("/api/harness/diff", method="GET")
    body = r.get("body", r)
    check("diff endpoint works", "diff" in body or "has_changes" in body, body)

    r = api("/api/harness/snapshot", {"name": "test-snap", "commit_sha": "abc123"})
    body = r.get("body", r)
    check("snapshot saved", body.get("saved") is True, body)

    r = api("/api/harness/snapshots", method="GET")
    body = r.get("body", r)
    snaps = body.get("snapshots", [])
    check("list snapshots has test-snap", any(s.get("name") == "test-snap" for s in snaps), snaps)


def test_report_functions():
    print("\n--- Reports ---")

    r = api("/api/report/summary", {"tag": TAG})
    body = r.get("body", r)
    check("summary returns stats", "stats" in body, body)
    check("summary total > 0", body.get("stats", {}).get("total", 0) > 0, body)
    check("summary has best", body.get("best") is not None, body)

    r = api("/api/report/tsv", {"tag": TAG})
    body = r.get("body", r)
    check("tsv returns data", body.get("count", 0) > 0, body)
    check("tsv has header", "commit" in body.get("tsv", ""), body)

    r = api("/api/report/leaderboard", {"tag": TAG, "limit": 5})
    body = r.get("body", r)
    check("leaderboard returns entries", len(body.get("leaderboard", [])) > 0, body)

    r = api("/api/report/tags", method="GET")
    body = r.get("body", r)
    tag_names = [t.get("name") for t in body.get("tags", [])]
    check("tags includes test tag", TAG in tag_names, tag_names)


def test_task_list():
    print("\n--- Task Functions ---")

    r = api("/api/task/list", method="GET")
    body = r.get("body", r)
    check("task list works", "tasks" in body, body)


def test_path_traversal():
    print("\n--- Path Traversal Protection (C1) ---")

    r = api("/api/experiment/register", {"tag": TAG, "description": "traversal test", "category": "other"})
    eid = r.get("body", r).get("experiment_id", "test")

    r = api("/api/task/run", {"task_name": "../../etc/passwd", "experiment_id": eid})
    body = r.get("body", r)
    check("path traversal blocked (..)", "error" in body or r.get("statusCode") == 400, body)

    r = api("/api/task/run", {"task_name": "foo/bar", "experiment_id": eid})
    body = r.get("body", r)
    check("slash in task_name blocked", "error" in body or r.get("statusCode") == 400, body)

    r = api("/api/task/run", {"task_name": ".hidden", "experiment_id": eid})
    body = r.get("body", r)
    check("dot-prefix blocked", "error" in body or r.get("statusCode") == 400, body)


def test_input_validation():
    print("\n--- Input Validation (W5/W6) ---")

    r = api("/api/search/set-strategy", {"tag": TAG, "mode": "invalid_mode"})
    body = r.get("body", r)
    check("invalid strategy mode rejected", "error" in body, body)

    r = api("/api/search/set-strategy", {"tag": TAG})
    body = r.get("body", r)
    check("missing mode rejected", "error" in body, body)

    r = api("/api/experiment/register", {})
    body = r.get("body", r)
    check("register without tag rejected", "error" in body, body)

    r = api("/api/experiment/complete", {"experiment_id": "nonexistent-id"})
    body = r.get("body", r)
    check("complete with bad id returns 404", r.get("statusCode") == 404 or "not found" in str(body).lower(), body)


def test_search_adapt_transitions():
    print("\n--- Search Adapt Transitions (T3) ---")

    adapt_tag = f"adapt-{int(time.time())}"
    api("/api/experiment/setup", {"tag": adapt_tag})

    r = api("/api/search/adapt", {"tag": adapt_tag})
    body = r.get("body", r)
    check("adapt with <5 exps stays explore", body.get("mode") == "explore", body)

    for i in range(6):
        r = api("/api/experiment/register", {"tag": adapt_tag, "description": f"exp{i}", "category": "tools"})
        eid = r.get("body", r).get("experiment_id")
        api("/api/experiment/crash", {"experiment_id": eid, "error": "test crash"})

    r = api("/api/search/adapt", {"tag": adapt_tag})
    body = r.get("body", r)
    check("high crash rate -> exploit", body.get("mode") == "exploit", body)


def test_report_diff():
    print("\n--- Report Diff (T6) ---")

    diff_tag = f"diff-{int(time.time())}"
    api("/api/experiment/setup", {"tag": diff_tag})

    r = api("/api/experiment/register", {"tag": diff_tag, "description": "exp-a", "category": "tools"})
    eid_a = r.get("body", r).get("experiment_id")
    api("/api/experiment/complete", {
        "experiment_id": eid_a, "passed": 5, "total_tasks": 10, "aggregate_score": 0.5,
        "task_scores": {"t1": 1.0, "t2": 0.0, "t3": 1.0},
    })

    r = api("/api/experiment/register", {"tag": diff_tag, "description": "exp-b", "category": "tools"})
    eid_b = r.get("body", r).get("experiment_id")
    api("/api/experiment/complete", {
        "experiment_id": eid_b, "passed": 6, "total_tasks": 10, "aggregate_score": 0.6,
        "task_scores": {"t1": 1.0, "t2": 1.0, "t3": 0.0},
    })

    r = api("/api/report/diff", {"experiment_a": eid_a, "experiment_b": eid_b})
    body = r.get("body", r)
    check("diff returns delta_passed", body.get("delta_passed") == 1, body)
    check("diff detects regressions", len(body.get("regressions", [])) > 0, body)
    check("diff detects improvements", len(body.get("improvements", [])) > 0, body)


def test_harness_restore():
    print("\n--- Harness Restore (T1) ---")

    r = api("/api/harness/snapshot", {"name": "restore-test-snap"})
    body = r.get("body", r)
    check("snapshot for restore test", body.get("saved") is True, body)

    r = api("/api/harness/restore", {"name": "restore-test-snap"})
    body = r.get("body", r)
    check("restore works", body.get("restored") is True, body)

    r = api("/api/harness/restore", {"name": "nonexistent-snap"})
    check("restore nonexistent fails", r.get("statusCode") == 404 or "not found" in str(r).lower(), r)


def main():
    global PASS, FAIL

    print(f"autoharness orchestrator tests (tag={TAG})")
    print(f"API: {BASE}")

    try:
        urllib.request.urlopen(f"{BASE}/api/report/tags", timeout=5)
    except Exception as e:
        print(f"\nERROR: Cannot reach {BASE} — is iii-engine + orchestrator running?")
        print(f"  Start: iii --config iii-config.yaml")
        print(f"  Then:  cd workers/orchestrator && uv run orchestrator.py")
        sys.exit(1)

    exp_id, exp_id_2, exp_id_3 = test_experiment_lifecycle()
    test_crash_tracking()
    test_query_endpoints(exp_id, exp_id_2, exp_id_3)
    test_search_functions()
    test_harness_functions()
    test_report_functions()
    test_task_list()
    test_path_traversal()
    test_input_validation()
    test_search_adapt_transitions()
    test_report_diff()
    test_harness_restore()

    print(f"\n{'='*50}")
    print(f"Results: {PASS} passed, {FAIL} failed, {PASS+FAIL} total")
    print(f"{'='*50}")

    sys.exit(0 if FAIL == 0 else 1)


if __name__ == "__main__":
    main()
