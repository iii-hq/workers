import os
import re
import json
import time
import hmac
import asyncio
import signal
import hashlib
import subprocess
from collections import OrderedDict
from datetime import datetime, timezone
from pathlib import Path
from iii import InitOptions, OtelConfig, Logger, TriggerAction, register_worker

logger = Logger("orchestrator")

VERSION = "0.1.0"
WS_URL = os.environ.get("III_WS_URL", "ws://localhost:49134")
WORKER_NAME = "autoharness-orchestrator"
MAX_CONSECUTIVE_CRASHES = int(os.environ.get("MAX_CONSECUTIVE_CRASHES", "3"))
NEAR_MISS_THRESHOLD = float(os.environ.get("NEAR_MISS_THRESHOLD", "0.02"))
MAX_EXPERIMENTS = int(os.environ.get("MAX_EXPERIMENTS", "200"))
HARNESS_PATH = os.environ.get("HARNESS_PATH", os.path.join(os.path.dirname(__file__), "..", "..", "agent.py"))
TASKS_DIR = os.environ.get("TASKS_DIR", os.path.join(os.path.dirname(__file__), "..", "..", "tasks"))
HARBOR_TIMEOUT = int(os.environ.get("HARBOR_TIMEOUT", "600"))
HARBOR_CONCURRENCY = int(os.environ.get("HARBOR_CONCURRENCY", "4"))
AUTH_TOKEN = os.environ.get("AUTOHARNESS_AUTH_TOKEN", "")
VALID_STRATEGIES = {"explore", "exploit", "combine", "ablation"}

_SAFE_NAME_RE = re.compile(r"^[a-zA-Z0-9][a-zA-Z0-9._-]{0,127}$")

SCOPES = {
    "experiments": "experiments",
    "lineage": "lineage",
    "best": "best",
    "near_misses": "near_misses",
    "strategy": "strategy",
    "tags": "tags",
    "crashes": "crashes",
    "snapshots": "snapshots",
    "task_results": "task_results",
    "task_scores": "task_scores",
}

ALL_CATEGORIES = [
    "system_prompt", "tools", "orchestration", "model_selection",
    "error_handling", "context_management", "output_parsing",
    "multi_step_planning", "simplification", "combination",
    "ablation", "other",
]


def experiment_id():
    t = int(time.time() * 1000)
    r = os.urandom(4).hex()
    return f"exp-{t:x}-{r}"


def _validate_safe_name(name):
    if not name or not _SAFE_NAME_RE.match(name):
        return False
    if ".." in name or "/" in name or "\\" in name:
        return False
    return True


def _check_auth(data):
    if not AUTH_TOKEN:
        return True
    inp = data if isinstance(data, dict) else {}
    headers = inp.get("headers", {})
    token = headers.get("authorization", "").removeprefix("Bearer ").strip()
    if not token:
        token = headers.get("x-auth-token", "")
    return hmac.compare_digest(token, AUTH_TOKEN)


def _is_near_miss(improved, best, delta_passed, delta_score):
    return (
        not improved
        and best is not None
        and abs(delta_passed) <= 1
        and abs(delta_score) <= NEAR_MISS_THRESHOLD
    )


def _reg_fn(sdk, fn_id, handler, description=""):
    sdk.register_function({"id": fn_id, "description": description}, handler)


def _reg_trigger(sdk, trigger_type, function_id, config=None):
    sdk.register_trigger({"type": trigger_type, "function_id": function_id, "config": config or {}})


async def _trigger(sdk, function_id, payload=None):
    return await sdk.trigger_async({"function_id": function_id, "payload": payload or {}})


async def _trigger_void(sdk, function_id, payload=None):
    await sdk.trigger_async({"function_id": function_id, "payload": payload or {}, "action": TriggerAction.Void()})


class StateKV:
    def __init__(self, sdk):
        self.sdk = sdk

    async def get(self, scope, key):
        try:
            return await _trigger(self.sdk, "state::get", {"scope": scope, "key": key})
        except KeyError:
            return None
        except Exception as e:
            logger.error("state::get failed", {"scope": scope, "key": key, "error": str(e)})
            raise

    async def set(self, scope, key, value):
        await _trigger(self.sdk, "state::set", {"scope": scope, "key": key, "value": value})

    async def list(self, scope):
        try:
            return await _trigger(self.sdk, "state::list", {"scope": scope})
        except KeyError:
            return []
        except Exception as e:
            logger.error("state::list failed", {"scope": scope, "error": str(e)})
            raise

    async def delete(self, scope, key):
        await _trigger(self.sdk, "state::delete", {"scope": scope, "key": key})


_tag_locks = OrderedDict()
_TAG_LOCK_MAX = 256


def _tag_lock(tag):
    if tag not in _tag_locks:
        if len(_tag_locks) >= _TAG_LOCK_MAX:
            _tag_locks.popitem(last=False)
        _tag_locks[tag] = asyncio.Lock()
    else:
        _tag_locks.move_to_end(tag)
    return _tag_locks[tag]


def _unwrap_input(data):
    if isinstance(data, dict) and "body" in data:
        data = data["body"]
    if isinstance(data, str):
        try:
            data = json.loads(data)
        except (json.JSONDecodeError, TypeError):
            pass
    if not isinstance(data, dict):
        return {}
    return data


def _ok(body):
    return {"statusCode": 200, "body": body}


def _err(body, status=400):
    return {"statusCode": status, "body": body}


# ---------------------------------------------------------------------------
# Experiment functions — track harness modification experiments
# ---------------------------------------------------------------------------

def register_experiment_functions(sdk, kv):

    async def setup(data):
        inp = _unwrap_input(data)
        tag = inp.get("tag")
        if not tag:
            return _err({"error": "tag is required"})
        existing = await kv.get(SCOPES["tags"], tag)
        if existing:
            return _err({"error": f"Tag '{tag}' already exists", "existing": existing})

        tag_data = {
            "name": tag,
            "branch": f"autoharness/{tag}",
            "created_at": datetime.now(timezone.utc).isoformat(),
            "best_passed": 0,
            "best_score": 0.0,
            "total_experiments": 0,
            "kept_experiments": 0,
        }
        await kv.set(SCOPES["tags"], tag, tag_data)

        strategy = {
            "mode": "explore",
            "explore_ratio": 0.7,
            "temperature": 1.0,
            "updated_at": datetime.now(timezone.utc).isoformat(),
            "reason": "initial exploration phase",
        }
        await kv.set(SCOPES["strategy"], tag, strategy)
        return _ok({"tag": tag_data, "branch": tag_data["branch"]})

    async def register(data):
        inp = _unwrap_input(data)
        if not inp.get("tag"):
            return _err({"error": "tag is required"})
        eid = experiment_id()
        experiment = {
            "id": eid,
            "tag": inp["tag"],
            "parent_id": inp.get("parent_id"),
            "commit_sha": inp.get("commit_sha", "unknown"),
            "description": inp.get("description", ""),
            "hypothesis": inp.get("hypothesis", ""),
            "category": inp.get("category", "other"),
            "passed": 0,
            "total_tasks": 0,
            "aggregate_score": 0.0,
            "task_scores": {},
            "duration_seconds": 0,
            "tokens_used": 0,
            "estimated_cost": 0.0,
            "status": "running",
            "runner_id": inp.get("runner_id"),
            "started_at": datetime.now(timezone.utc).isoformat(),
            "finished_at": None,
            "diff_summary": inp.get("diff_summary", ""),
            "error": None,
        }
        await kv.set(SCOPES["experiments"], eid, experiment)

        async with _tag_lock(inp["tag"]):
            lineage = await kv.get(SCOPES["lineage"], inp["tag"]) or []
            lineage.append(eid)
            await kv.set(SCOPES["lineage"], inp["tag"], lineage)

        return _ok({"experiment_id": eid, "status": "registered"})

    async def complete(data):
        inp = _unwrap_input(data)
        eid = inp.get("experiment_id")
        exp = await kv.get(SCOPES["experiments"], eid)
        if not exp:
            return _err({"error": f"Experiment {eid} not found"}, 404)
        if exp.get("status") in ("keep", "discard", "crash"):
            return _ok({"experiment_id": eid, "status": exp["status"], "action": "no_op", "reason": "already terminal"})

        async with _tag_lock(exp["tag"]):
            best = await kv.get(SCOPES["best"], exp["tag"])

            passed = inp.get("passed", 0)
            total = inp.get("total_tasks", 0)
            score = inp.get("aggregate_score", 0.0)

            improved = False
            if not best:
                improved = True
            elif passed > best["passed"]:
                improved = True
            elif passed == best["passed"] and score > best["aggregate_score"]:
                improved = True

            delta_passed = passed - (best["passed"] if best else 0)
            delta_score = score - (best["aggregate_score"] if best else 0)

            exp["passed"] = passed
            exp["total_tasks"] = total
            exp["aggregate_score"] = score
            exp["task_scores"] = inp.get("task_scores", {})
            exp["duration_seconds"] = inp.get("duration_seconds", 0)
            exp["tokens_used"] = inp.get("tokens_used", 0)
            exp["estimated_cost"] = inp.get("estimated_cost", 0.0)
            exp["status"] = "keep" if improved else "discard"
            exp["finished_at"] = datetime.now(timezone.utc).isoformat()
            await kv.set(SCOPES["experiments"], exp["id"], exp)

            if improved:
                await kv.set(SCOPES["best"], exp["tag"], {
                    "experiment_id": exp["id"],
                    "passed": passed,
                    "aggregate_score": score,
                    "commit_sha": exp["commit_sha"],
                    "updated_at": datetime.now(timezone.utc).isoformat(),
                })

            if _is_near_miss(improved, best, delta_passed, delta_score):
                await kv.set(SCOPES["near_misses"], exp["id"], {
                    "experiment_id": exp["id"],
                    "tag": exp["tag"],
                    "passed": passed,
                    "aggregate_score": score,
                    "delta_passed": delta_passed,
                    "delta_score": delta_score,
                    "hypothesis": exp["hypothesis"],
                    "category": exp["category"],
                    "diff_summary": exp["diff_summary"],
                })

            tag = await kv.get(SCOPES["tags"], exp["tag"])
            if tag:
                tag["total_experiments"] += 1
                if improved:
                    tag["kept_experiments"] += 1
                    tag["best_passed"] = passed
                    tag["best_score"] = score
                await kv.set(SCOPES["tags"], exp["tag"], tag)

            await kv.delete(SCOPES["crashes"], exp["tag"])

        await _trigger_void(sdk, "search::adapt", {"tag": exp["tag"]})

        return _ok({
            "experiment_id": exp["id"],
            "status": exp["status"],
            "passed": passed,
            "total_tasks": total,
            "aggregate_score": score,
            "improved": improved,
            "delta_passed": delta_passed,
            "delta_score": delta_score,
            "best_passed": passed if improved else (best["passed"] if best else 0),
            "best_score": score if improved else (best["aggregate_score"] if best else 0),
            "action": "keep_commit" if improved else "git_reset",
        })

    async def crash(data):
        inp = _unwrap_input(data)
        eid = inp.get("experiment_id")
        exp = await kv.get(SCOPES["experiments"], eid)
        if not exp:
            return _err({"error": f"Experiment {eid} not found"}, 404)
        if exp.get("status") in ("keep", "discard", "crash"):
            return _ok({"experiment_id": eid, "status": exp["status"], "action": "no_op", "reason": "already terminal"})

        exp["status"] = "crash"
        exp["error"] = inp.get("error", "unknown")
        exp["finished_at"] = datetime.now(timezone.utc).isoformat()
        await kv.set(SCOPES["experiments"], exp["id"], exp)

        async with _tag_lock(exp["tag"]):
            crashes = await kv.get(SCOPES["crashes"], exp["tag"]) or 0
            consecutive = crashes + 1
            await kv.set(SCOPES["crashes"], exp["tag"], consecutive)

            tag = await kv.get(SCOPES["tags"], exp["tag"])
            if tag:
                tag["total_experiments"] += 1
                await kv.set(SCOPES["tags"], exp["tag"], tag)

        await _trigger_void(sdk, "search::adapt", {"tag": exp["tag"]})

        return _ok({
            "experiment_id": exp["id"],
            "status": "crash",
            "consecutive_crashes": consecutive,
            "should_abort": consecutive >= MAX_CONSECUTIVE_CRASHES,
            "action": "git_reset",
        })

    async def history(data):
        inp = _unwrap_input(data)
        all_exps = await kv.list(SCOPES["experiments"])
        filtered = [e for e in all_exps if e.get("tag") == inp.get("tag")]
        if inp.get("status"):
            filtered = [e for e in filtered if e.get("status") == inp["status"]]
        filtered.sort(key=lambda e: e.get("started_at", ""))
        limit = inp.get("limit")
        if limit:
            filtered = filtered[-limit:]
        return _ok({"experiments": filtered, "total": len(filtered)})

    async def best(data):
        inp = _unwrap_input(data)
        b = await kv.get(SCOPES["best"], inp.get("tag"))
        if not b:
            return _err({"error": "No results yet", "tag": inp.get("tag")}, 404)
        exp = await kv.get(SCOPES["experiments"], b["experiment_id"])
        return _ok({"best": b, "experiment": exp})

    async def near_misses(data):
        inp = _unwrap_input(data)
        tag = inp.get("tag")
        current_best = await kv.get(SCOPES["best"], tag)
        all_nm = await kv.list(SCOPES["near_misses"])
        tag_nm = []
        for n in all_nm:
            if n.get("tag") != tag:
                continue
            if current_best:
                delta_p = n.get("passed", 0) - current_best["passed"]
                delta_s = n.get("aggregate_score", 0) - current_best["aggregate_score"]
                if abs(delta_p) > 1 or abs(delta_s) > NEAR_MISS_THRESHOLD:
                    continue
            tag_nm.append(n)
        filtered = sorted(tag_nm, key=lambda n: n.get("delta_score", 0), reverse=True)
        limit = inp.get("limit", 20)
        return _ok({"near_misses": filtered[:limit], "total": len(filtered)})

    _reg_fn(sdk, "experiment::setup", setup, "Initialize a new experiment run tag.")
    _reg_fn(sdk, "experiment::register", register, "Register a new experiment before benchmarking.")
    _reg_fn(sdk, "experiment::complete", complete, "Record benchmark results. Auto keep/discard.")
    _reg_fn(sdk, "experiment::crash", crash, "Record a crashed experiment.")
    _reg_fn(sdk, "experiment::history", history, "Get experiment history for a tag.")
    _reg_fn(sdk, "experiment::best", best, "Get current best result for a tag.")
    _reg_fn(sdk, "experiment::near_misses", near_misses, "Get near-miss experiments.")


# ---------------------------------------------------------------------------
# Task functions — manage benchmark tasks and execution
# ---------------------------------------------------------------------------

def register_task_functions(sdk, kv):

    async def list_tasks(data):
        _unwrap_input(data)
        tasks_path = Path(TASKS_DIR)
        if not tasks_path.exists():
            return _ok({"tasks": [], "total": 0})
        tasks = []
        for task_dir in sorted(tasks_path.iterdir()):
            if not task_dir.is_dir():
                continue
            task_toml = task_dir / "task.toml"
            instruction = task_dir / "instruction.md"
            tasks.append({
                "name": task_dir.name,
                "has_config": task_toml.exists(),
                "has_instruction": instruction.exists(),
                "has_tests": (task_dir / "tests").exists(),
                "has_environment": (task_dir / "environment").exists(),
            })
        return _ok({"tasks": tasks, "total": len(tasks)})

    async def run_single(data):
        inp = _unwrap_input(data)
        task_name = inp.get("task_name")
        experiment_id = inp.get("experiment_id")
        if not task_name or not experiment_id:
            return _err({"error": "task_name and experiment_id required"})
        if not _validate_safe_name(task_name):
            return _err({"error": "Invalid task_name: must be alphanumeric with .-_ only, no path separators"})

        task_path = (Path(TASKS_DIR) / task_name).resolve()
        tasks_root = Path(TASKS_DIR).resolve()
        if not str(task_path).startswith(str(tasks_root)):
            return _err({"error": "task_name escapes tasks directory"})
        if not task_path.exists():
            return _err({"error": f"Task '{task_name}' not found"}, 404)

        timeout = inp.get("timeout", HARBOR_TIMEOUT)
        start = time.time()
        try:
            result = subprocess.run(
                ["harbor", "run", "-p", str(task_path), "--timeout", str(timeout)],
                capture_output=True, text=True, timeout=timeout + 60,
                cwd=os.path.dirname(HARNESS_PATH),
            )
            duration = time.time() - start

            score = 0.0
            reward_file = task_path / "logs" / "reward.txt"
            if reward_file.exists() and reward_file.stat().st_mtime >= start:
                try:
                    score = float(reward_file.read_text().strip())
                except ValueError:
                    pass

            trajectory = None
            traj_file = task_path / "logs" / "agent" / "trajectory.json"
            if traj_file.exists() and traj_file.stat().st_mtime >= start:
                try:
                    trajectory = json.loads(traj_file.read_text())
                except json.JSONDecodeError:
                    pass

            task_result = {
                "task_name": task_name,
                "experiment_id": experiment_id,
                "score": score,
                "passed": score >= 1.0,
                "duration_seconds": round(duration, 1),
                "exit_code": result.returncode,
                "stdout_tail": result.stdout[-2000:] if result.stdout else "",
                "stderr_tail": result.stderr[-2000:] if result.stderr else "",
                "has_trajectory": trajectory is not None,
                "completed_at": datetime.now(timezone.utc).isoformat(),
            }
            await kv.set(SCOPES["task_results"], f"{experiment_id}:{task_name}", task_result)
            return _ok(task_result)

        except subprocess.TimeoutExpired:
            duration = time.time() - start
            task_result = {
                "task_name": task_name,
                "experiment_id": experiment_id,
                "score": 0.0,
                "passed": False,
                "duration_seconds": round(duration, 1),
                "exit_code": -1,
                "error": "timeout",
                "completed_at": datetime.now(timezone.utc).isoformat(),
            }
            await kv.set(SCOPES["task_results"], f"{experiment_id}:{task_name}", task_result)
            return _ok(task_result)

        except Exception as e:
            return _err({"error": str(e), "task_name": task_name}, 500)

    async def run_batch(data):
        inp = _unwrap_input(data)
        experiment_id = inp.get("experiment_id")
        if not experiment_id:
            return _err({"error": "experiment_id required"})

        concurrency = inp.get("concurrency", HARBOR_CONCURRENCY)
        timeout = inp.get("timeout", HARBOR_TIMEOUT)
        tasks_path = Path(TASKS_DIR)
        task_names = inp.get("tasks") or [
            d.name for d in sorted(tasks_path.iterdir()) if d.is_dir()
        ]

        semaphore = asyncio.Semaphore(concurrency)
        results = []

        async def run_one(name):
            async with semaphore:
                result = await _trigger(sdk, "task::run", {
                    "task_name": name,
                    "experiment_id": experiment_id,
                    "timeout": timeout,
                })
                body = result.get("body", result) if isinstance(result, dict) else result
                return body

        tasks = [run_one(name) for name in task_names]
        results = await asyncio.gather(*tasks, return_exceptions=True)

        passed = 0
        total = len(task_names)
        total_score = 0.0
        total_duration = 0.0
        task_scores = {}

        for r in results:
            if isinstance(r, Exception):
                continue
            if isinstance(r, dict):
                name = r.get("task_name", "unknown")
                score = r.get("score", 0.0)
                task_scores[name] = score
                total_score += score
                total_duration += r.get("duration_seconds", 0)
                if r.get("passed"):
                    passed += 1

        aggregate_score = total_score / total if total > 0 else 0.0

        batch_result = {
            "experiment_id": experiment_id,
            "passed": passed,
            "total_tasks": total,
            "aggregate_score": round(aggregate_score, 4),
            "total_score": round(total_score, 4),
            "task_scores": task_scores,
            "duration_seconds": round(total_duration, 1),
        }
        await kv.set(SCOPES["task_scores"], experiment_id, batch_result)
        return _ok(batch_result)

    async def scores(data):
        inp = _unwrap_input(data)
        eid = inp.get("experiment_id")
        if not eid:
            return _err({"error": "experiment_id required"})
        s = await kv.get(SCOPES["task_scores"], eid)
        if not s:
            return _err({"error": "No scores found"}, 404)
        return _ok(s)

    async def task_failures(data):
        inp = _unwrap_input(data)
        eid = inp.get("experiment_id")
        all_results = await kv.list(SCOPES["task_results"])
        failures = [
            r for r in all_results
            if r.get("experiment_id") == eid and not r.get("passed")
        ]
        return _ok({"failures": failures, "total": len(failures)})

    _reg_fn(sdk, "task::list", list_tasks, "List available benchmark tasks.")
    _reg_fn(sdk, "task::run", run_single, "Run a single benchmark task.")
    _reg_fn(sdk, "task::batch", run_batch, "Run all tasks in a benchmark suite.")
    _reg_fn(sdk, "task::scores", scores, "Get scores for an experiment.")
    _reg_fn(sdk, "task::failures", task_failures, "Get failed tasks for an experiment.")


# ---------------------------------------------------------------------------
# Search functions — adaptive strategy for harness improvement
# ---------------------------------------------------------------------------

def register_search_functions(sdk, kv):

    async def strategy(data):
        inp = _unwrap_input(data)
        s = await kv.get(SCOPES["strategy"], inp.get("tag"))
        return _ok(s or {"mode": "explore", "explore_ratio": 0.7, "temperature": 1.0})

    async def set_strategy(data):
        inp = _unwrap_input(data)
        mode = inp.get("mode")
        tag = inp.get("tag")
        if not mode or not tag:
            return _err({"error": "mode and tag are required"})
        if mode not in VALID_STRATEGIES:
            return _err({"error": f"Invalid mode '{mode}'. Must be one of: {', '.join(sorted(VALID_STRATEGIES))}"})
        s = {
            "mode": mode,
            "explore_ratio": inp.get("explore_ratio", 0.7),
            "temperature": inp.get("temperature", 1.0),
            "updated_at": datetime.now(timezone.utc).isoformat(),
            "reason": inp.get("reason", "manual override"),
        }
        await kv.set(SCOPES["strategy"], tag, s)
        return _ok(s)

    async def adapt(data):
        inp = _unwrap_input(data)
        all_exps = await kv.list(SCOPES["experiments"])
        tag_exps = [e for e in all_exps if e.get("tag") == inp.get("tag") and e.get("status") != "running"]
        tag_exps.sort(key=lambda e: e.get("started_at", ""))

        if len(tag_exps) < 5:
            return _ok({"mode": "explore", "reason": "too few experiments to adapt"})

        recent = tag_exps[-10:]
        keep_rate = sum(1 for e in recent if e.get("status") == "keep") / len(recent)
        crash_rate = sum(1 for e in recent if e.get("status") == "crash") / len(recent)
        all_nm = await kv.list(SCOPES["near_misses"])
        near_misses = [n for n in all_nm if n.get("tag") == inp.get("tag")]

        if crash_rate > 0.5:
            mode, temperature = "exploit", 0.3
            reason = f"high crash rate ({crash_rate*100:.0f}%), conservative tweaks only"
        elif keep_rate == 0 and len(tag_exps) > 15:
            if len(near_misses) >= 2:
                mode, temperature = "combine", 0.5
                reason = f"plateau with {len(near_misses)} near-misses, try combinations"
            else:
                mode, temperature = "ablation", 0.3
                reason = "long plateau, ablation to find essential components"
        elif keep_rate > 0.3:
            mode, temperature = "exploit", 0.5
            reason = f"good keep rate ({keep_rate*100:.0f}%), exploiting current direction"
        else:
            mode, temperature = "explore", 0.8
            reason = "default exploration"

        s = {
            "mode": mode,
            "explore_ratio": 0.8 if mode == "explore" else 0.3,
            "temperature": temperature,
            "updated_at": datetime.now(timezone.utc).isoformat(),
            "reason": reason,
        }
        await kv.set(SCOPES["strategy"], inp.get("tag"), s)
        return _ok(s)

    async def suggest(data):
        inp = _unwrap_input(data)
        all_exps = await kv.list(SCOPES["experiments"])
        tag_exps = sorted(
            [e for e in all_exps if e.get("tag") == inp.get("tag") and e.get("status") != "running"],
            key=lambda e: e.get("started_at", ""),
        )

        strat = await kv.get(SCOPES["strategy"], inp.get("tag"))
        all_nm = await kv.list(SCOPES["near_misses"])
        near_misses = [n for n in all_nm if n.get("tag") == inp.get("tag")]

        category_counts = {}
        for exp in tag_exps:
            cat = exp.get("category", "other")
            if cat not in category_counts:
                category_counts[cat] = {"total": 0, "kept": 0}
            category_counts[cat]["total"] += 1
            if exp.get("status") == "keep":
                category_counts[cat]["kept"] += 1

        underexplored = [c for c in ALL_CATEGORIES[:8] if c not in category_counts or category_counts[c]["total"] < 3]
        high_yield = [c for c, v in category_counts.items() if v["total"] >= 3 and v["kept"] / v["total"] > 0.3]

        kept = [e for e in tag_exps if e.get("status") == "keep"]
        score_trend = [e["aggregate_score"] for e in kept[-5:]]

        recent_failures = []
        for exp in tag_exps[-5:]:
            if exp.get("status") == "discard" and exp.get("task_scores"):
                failed_tasks = [t for t, s in exp["task_scores"].items() if s < 1.0]
                if failed_tasks:
                    recent_failures.append({
                        "experiment": exp["id"],
                        "hypothesis": exp["hypothesis"],
                        "failed_tasks": failed_tasks,
                    })

        mode = strat.get("mode", "explore") if strat else "explore"
        suggestions = _build_suggestions(mode, underexplored, high_yield, near_misses, score_trend, recent_failures)

        return _ok({
            "strategy": mode,
            "total_experiments": len(tag_exps),
            "category_stats": category_counts,
            "underexplored_categories": underexplored,
            "high_yield_categories": high_yield,
            "near_misses_available": len(near_misses),
            "near_miss_categories": list(set(n.get("category") for n in near_misses)),
            "recent_score_trend": score_trend,
            "common_failure_tasks": _common_failures(tag_exps),
            "suggestions": suggestions,
        })

    _reg_fn(sdk, "search::strategy", strategy, "Get current search strategy.")
    _reg_fn(sdk, "search::set_strategy", set_strategy, "Override search strategy.")
    _reg_fn(sdk, "search::adapt", adapt, "Auto-adapt strategy from experiment history.")
    _reg_fn(sdk, "search::suggest_direction", suggest, "Suggest what to try next.")


def _build_suggestions(mode, underexplored, high_yield, near_misses, trend, failures):
    suggestions = []

    if failures:
        task_freq = {}
        for f in failures:
            for t in f["failed_tasks"]:
                task_freq[t] = task_freq.get(t, 0) + 1
        top_failures = sorted(task_freq.items(), key=lambda x: x[1], reverse=True)[:3]
        suggestions.append(f"Recurring failures: {', '.join(f'{t} ({c}x)' for t, c in top_failures)}. Focus on these.")

    if mode == "explore":
        if underexplored:
            suggestions.append(f"Try changes in underexplored categories: {', '.join(underexplored[:3])}")
        suggestions.append("Try adding a new tool or refactoring the system prompt structure")
    elif mode == "exploit":
        if high_yield:
            suggestions.append(f"Double down on high-yield categories: {', '.join(high_yield)}")
        suggestions.append("Make small incremental tweaks to the current best harness")
    elif mode == "combine":
        if len(near_misses) >= 2:
            pair = near_misses[:2]
            suggestions.append(f'Combine near-misses: "{pair[0].get("hypothesis")}" + "{pair[1].get("hypothesis")}"')
    elif mode == "ablation":
        suggestions.append("Remove one tool/capability at a time to identify what actually helps")
        suggestions.append("Simplify: shorter system prompt, fewer tools, less orchestration")

    if len(trend) >= 3 and trend[-1] <= trend[0]:
        suggestions.append("Score trend is flat/worsening. Consider a strategy change.")

    return suggestions


def _common_failures(tag_exps):
    task_fail_counts = {}
    for exp in tag_exps:
        if exp.get("task_scores"):
            for task, score in exp["task_scores"].items():
                if task not in task_fail_counts:
                    task_fail_counts[task] = {"total": 0, "failed": 0}
                task_fail_counts[task]["total"] += 1
                if score < 1.0:
                    task_fail_counts[task]["failed"] += 1
    return {
        t: {"total": v["total"], "failed": v["failed"], "fail_rate": round(v["failed"] / v["total"], 2)}
        for t, v in task_fail_counts.items()
        if v["failed"] > 0
    }


# ---------------------------------------------------------------------------
# Harness functions — manage the agent.py harness file
# ---------------------------------------------------------------------------

def register_harness_functions(sdk, kv):

    async def read_harness(data):
        _unwrap_input(data)
        path = Path(HARNESS_PATH)
        if not path.exists():
            return _err({"error": "agent.py not found"}, 404)
        content = path.read_text()
        lines = content.split("\n")
        editable_end = 0
        for i, line in enumerate(lines):
            if "# --- FIXED ADAPTER BELOW ---" in line or "# FIXED" in line.upper():
                editable_end = i
                break
        return _ok({
            "path": str(path),
            "content": content,
            "lines": len(lines),
            "editable_lines": editable_end if editable_end > 0 else len(lines),
        })

    async def diff_harness(data):
        _unwrap_input(data)
        try:
            result = subprocess.run(
                ["git", "diff", "HEAD~1", "--", HARNESS_PATH],
                capture_output=True, text=True, timeout=10,
                cwd=os.path.dirname(HARNESS_PATH),
            )
            return _ok({"diff": result.stdout, "has_changes": bool(result.stdout.strip())})
        except Exception as e:
            return _err({"error": str(e)})

    async def snapshot(data):
        if not _check_auth(data):
            return _err({"error": "Unauthorized"}, 401)
        inp = _unwrap_input(data)
        name = inp.get("name")
        if not name:
            return _err({"error": "name is required"})
        path = Path(HARNESS_PATH)
        if not path.exists():
            return _err({"error": "agent.py not found"}, 404)

        snap = {
            "name": name,
            "content": path.read_text(),
            "commit_sha": inp.get("commit_sha", "unknown"),
            "experiment_id": inp.get("experiment_id"),
            "created_at": datetime.now(timezone.utc).isoformat(),
        }
        await kv.set(SCOPES["snapshots"], name, snap)
        return _ok({"saved": True, "name": name})

    async def restore_snapshot(data):
        if not _check_auth(data):
            return _err({"error": "Unauthorized"}, 401)
        inp = _unwrap_input(data)
        name = inp.get("name")
        snap = await kv.get(SCOPES["snapshots"], name)
        if not snap:
            return _err({"error": f"Snapshot '{name}' not found"}, 404)
        content = snap.get("content", "")
        if len(content) > 500_000:
            return _err({"error": "Snapshot content exceeds 500KB limit"})
        harness = Path(HARNESS_PATH).resolve()
        if not str(harness).endswith(".py"):
            return _err({"error": "HARNESS_PATH must be a .py file"})
        harness.write_text(content)
        return _ok({"restored": True, "name": name, "from_experiment": snap.get("experiment_id")})

    async def list_snapshots(data):
        _unwrap_input(data)
        snaps = await kv.list(SCOPES["snapshots"])
        return _ok({"snapshots": sorted(snaps, key=lambda s: s.get("created_at", ""), reverse=True)})

    _reg_fn(sdk, "harness::read", read_harness, "Read current agent.py harness.")
    _reg_fn(sdk, "harness::diff", diff_harness, "Diff agent.py against previous commit.")
    _reg_fn(sdk, "harness::snapshot", snapshot, "Save a named snapshot of the harness.")
    _reg_fn(sdk, "harness::restore", restore_snapshot, "Restore a named snapshot.")
    _reg_fn(sdk, "harness::list_snapshots", list_snapshots, "List all saved snapshots.")


# ---------------------------------------------------------------------------
# Report functions — monitoring and export
# ---------------------------------------------------------------------------

def register_report_functions(sdk, kv):

    async def summary(data):
        inp = _unwrap_input(data)
        tag = await kv.get(SCOPES["tags"], inp.get("tag"))
        if not tag:
            return _err({"error": f"Tag not found"}, 404)

        best = await kv.get(SCOPES["best"], inp.get("tag"))
        strat = await kv.get(SCOPES["strategy"], inp.get("tag"))
        all_exps = await kv.list(SCOPES["experiments"])
        tag_exps = sorted(
            [e for e in all_exps if e.get("tag") == inp.get("tag")],
            key=lambda e: e.get("started_at", ""),
        )

        status_counts = {"keep": 0, "discard": 0, "crash": 0, "running": 0}
        category_counts = {}
        for e in tag_exps:
            st = e.get("status", "running")
            status_counts[st] = status_counts.get(st, 0) + 1
            cat = e.get("category", "other")
            category_counts[cat] = category_counts.get(cat, 0) + 1

        kept = [e for e in tag_exps if e.get("status") == "keep"]
        score_history = [
            {
                "id": e["id"],
                "passed": e["passed"],
                "score": e["aggregate_score"],
                "description": e["description"],
                "category": e["category"],
                "at": e.get("finished_at"),
            }
            for e in kept
        ]

        total_duration_min = sum(e.get("duration_seconds", 0) for e in tag_exps) / 60
        total_cost = sum(e.get("estimated_cost", 0) for e in tag_exps)

        return _ok({
            "tag": inp.get("tag"),
            "branch": tag["branch"],
            "best": {
                "passed": best["passed"],
                "score": best["aggregate_score"],
                "commit": best["commit_sha"],
                "experiment_id": best["experiment_id"],
            } if best else None,
            "stats": {
                "total": tag["total_experiments"],
                "kept": status_counts["keep"],
                "discarded": status_counts["discard"],
                "crashed": status_counts["crash"],
                "running": status_counts["running"],
                "keep_rate": status_counts["keep"] / tag["total_experiments"] if tag["total_experiments"] > 0 else 0,
            },
            "categories": category_counts,
            "score_progression": score_history,
            "total_duration_minutes": round(total_duration_min, 1),
            "total_estimated_cost": round(total_cost, 2),
            "strategy": strat.get("mode", "unknown") if strat else "unknown",
            "common_failures": _common_failures(tag_exps),
        })

    async def tsv(data):
        inp = _unwrap_input(data)
        all_exps = await kv.list(SCOPES["experiments"])
        tag_exps = sorted(
            [e for e in all_exps if e.get("tag") == inp.get("tag") and e.get("status") != "running"],
            key=lambda e: e.get("started_at", ""),
        )
        header = "commit\tpassed\ttotal\tscore\tstatus\tdescription"
        rows = []
        for e in tag_exps:
            sha = e["commit_sha"][:7]
            rows.append(f"{sha}\t{e['passed']}\t{e['total_tasks']}\t{e['aggregate_score']:.4f}\t{e['status']}\t{e['description']}")
        return _ok({"tsv": "\n".join([header] + rows), "count": len(rows)})

    async def diff(data):
        inp = _unwrap_input(data)
        a = await kv.get(SCOPES["experiments"], inp["experiment_a"])
        b = await kv.get(SCOPES["experiments"], inp["experiment_b"])
        if not a or not b:
            return _err({"error": "One or both experiments not found"}, 404)

        a_tasks = set(a.get("task_scores", {}).keys())
        b_tasks = set(b.get("task_scores", {}).keys())
        regressions = []
        improvements = []
        for task in a_tasks & b_tasks:
            sa = a["task_scores"][task]
            sb = b["task_scores"][task]
            if sb < sa:
                regressions.append({"task": task, "before": sa, "after": sb})
            elif sb > sa:
                improvements.append({"task": task, "before": sa, "after": sb})

        return _ok({
            "a": {"id": a["id"], "passed": a["passed"], "score": a["aggregate_score"], "description": a["description"]},
            "b": {"id": b["id"], "passed": b["passed"], "score": b["aggregate_score"], "description": b["description"]},
            "delta_passed": b["passed"] - a["passed"],
            "delta_score": round(b["aggregate_score"] - a["aggregate_score"], 4),
            "regressions": regressions,
            "improvements": improvements,
        })

    async def leaderboard(data):
        inp = _unwrap_input(data)
        all_exps = await kv.list(SCOPES["experiments"])
        tag_exps = [e for e in all_exps if e.get("tag") == inp.get("tag") and e.get("status") in ("keep", "discard")]
        ranked = sorted(tag_exps, key=lambda e: (-e.get("passed", 0), -e.get("aggregate_score", 0)))
        limit = inp.get("limit", 10)
        top = [{
            "rank": i + 1,
            "id": e["id"],
            "passed": e["passed"],
            "score": e["aggregate_score"],
            "description": e["description"],
            "category": e["category"],
            "status": e["status"],
        } for i, e in enumerate(ranked[:limit])]
        return _ok({"leaderboard": top, "total": len(tag_exps)})

    async def tags(data):
        _unwrap_input(data)
        all_tags = await kv.list(SCOPES["tags"])
        return _ok({"tags": sorted(all_tags, key=lambda t: t.get("created_at", ""), reverse=True)})

    _reg_fn(sdk, "report::summary", summary, "Generate a full summary report for a tag.")
    _reg_fn(sdk, "report::tsv", tsv, "Export experiment history as TSV.")
    _reg_fn(sdk, "report::diff", diff, "Compare two experiments with task-level diff.")
    _reg_fn(sdk, "report::leaderboard", leaderboard, "Top N experiments by score.")
    _reg_fn(sdk, "report::tags", tags, "List all experiment tags.")


# ---------------------------------------------------------------------------
# HTTP trigger registration
# ---------------------------------------------------------------------------

def register_triggers(sdk):
    http_triggers = [
        ("/api/experiment/setup", "POST", "experiment::setup"),
        ("/api/experiment/register", "POST", "experiment::register"),
        ("/api/experiment/complete", "POST", "experiment::complete"),
        ("/api/experiment/crash", "POST", "experiment::crash"),
        ("/api/experiment/history", "POST", "experiment::history"),
        ("/api/experiment/best", "POST", "experiment::best"),
        ("/api/experiment/near-misses", "POST", "experiment::near_misses"),
        ("/api/task/list", "GET", "task::list"),
        ("/api/task/run", "POST", "task::run"),
        ("/api/task/batch", "POST", "task::batch"),
        ("/api/task/scores", "POST", "task::scores"),
        ("/api/task/failures", "POST", "task::failures"),
        ("/api/search/strategy", "POST", "search::strategy"),
        ("/api/search/set-strategy", "POST", "search::set_strategy"),
        ("/api/search/adapt", "POST", "search::adapt"),
        ("/api/search/suggest", "POST", "search::suggest_direction"),
        ("/api/harness/read", "GET", "harness::read"),
        ("/api/harness/diff", "GET", "harness::diff"),
        ("/api/harness/snapshot", "POST", "harness::snapshot"),
        ("/api/harness/restore", "POST", "harness::restore"),
        ("/api/harness/snapshots", "GET", "harness::list_snapshots"),
        ("/api/report/summary", "POST", "report::summary"),
        ("/api/report/tsv", "POST", "report::tsv"),
        ("/api/report/diff", "POST", "report::diff"),
        ("/api/report/leaderboard", "POST", "report::leaderboard"),
        ("/api/report/tags", "GET", "report::tags"),
    ]
    for path, method, fn in http_triggers:
        _reg_trigger(sdk, "http", fn, {"api_path": path, "http_method": method})



# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

async def main():
    opts = InitOptions(
        worker_name=WORKER_NAME,
        otel=OtelConfig(
            enabled=True,
            service_name="autoharness",
            service_version=VERSION,
            metrics_enabled=True,
        ),
    )
    sdk = register_worker(WS_URL, opts)
    kv = StateKV(sdk)

    register_experiment_functions(sdk, kv)
    register_task_functions(sdk, kv)
    register_search_functions(sdk, kv)
    register_harness_functions(sdk, kv)
    register_report_functions(sdk, kv)
    register_triggers(sdk)

    rest_port = os.environ.get("III_REST_PORT", "3111")
    logger.info("orchestrator started", {
        "version": VERSION,
        "ws_url": WS_URL,
        "rest_url": f"http://localhost:{rest_port}",
        "functions": 26,
        "triggers": 26,
        "auth": "enabled" if AUTH_TOKEN else "disabled",
        "max_experiments": MAX_EXPERIMENTS,
    })

    stop = asyncio.Event()

    def shutdown():
        logger.info("shutting down")
        stop.set()

    loop = asyncio.get_running_loop()
    loop.add_signal_handler(signal.SIGINT, shutdown)
    loop.add_signal_handler(signal.SIGTERM, shutdown)

    await stop.wait()
    await sdk.shutdown()


if __name__ == "__main__":
    asyncio.run(main())
