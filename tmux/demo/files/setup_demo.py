#!/usr/bin/env python3
"""Fake Claude Code accounts and Codex records for the demo container, as the
artboards show them. Every token is a fake string (`demo-<account>`) that only
the mock usage endpoint knows.

  setup_demo.py --accounts all   # Work (default), Personal, Side project
  setup_demo.py --accounts two   # Work, Personal (Add account makes Side project)
  setup_demo.py --accounts one   # Work only, no accounts.json

As the worker keeps them (agent-accounts G1, G2): the global account (Work)
is signed in to ~/.claude; every account, `default` included, has a folder
whose .claude.json holds its profile (`oauthAccount`), and each sign-in is in
one place only: ~/.claude for the global one, its folder for the others.
"""
import argparse
import datetime as dt
import json
import os
import sys
import time
import uuid


def require_container(what, *paths):
    """Only ever inside the demo container (demo/Dockerfile sets
    DEMO_CONTAINER=1): on a host this would write to the real home folder.
    The fake claude's sign-in also runs in a test's sandbox
    (FAKE_CLAUDE_SANDBOX), for `paths` inside it only."""
    if os.path.exists("/.dockerenv") and os.environ.get("DEMO_CONTAINER") == "1":
        return
    sandbox = os.environ.get("FAKE_CLAUDE_SANDBOX")
    if sandbox and paths:
        root = os.path.realpath(sandbox)
        if all(os.path.realpath(p) == root or os.path.realpath(p).startswith(root + os.sep)
               for p in paths):
            return
    sys.stderr.write("%s runs only inside the demo container\n" % what)
    sys.exit(2)


HOME = os.path.expanduser("~")
ROOT = os.path.join(HOME, ".local/share/iii/claude-accounts")
SHARED = {
    "settings": ["settings.json", "CLAUDE.md", "keybindings.json"],
    "extensions": ["skills", "agents", "commands", "output-styles"],
    "plugins": ["plugins"],
    "conversations": ["projects", "history.jsonl", "todos", "plans"],
}
# Who each fake account is (the fake `claude auth login` uses the same table).
PEOPLE = {
    "default": ("you@company.com", "Company", "claude_team", "team", "default_claude_team"),
    "personal": ("you@home.dev", "Personal", "claude_max", "max", "default_claude_max_20x"),
    "side-project": ("you@client.io", "Client", "claude_pro", "pro", "default_claude_pro"),
}


def write_json(path, value, mode=0o600):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        json.dump(value, f, indent=2)
    os.chmod(path, mode)


def token_name(account_id):
    """The fake token's name: `demo-work` for default, `demo-<id>` otherwise."""
    return "work" if account_id == "default" else account_id


def profile(account_id):
    """The `oauthAccount` Claude Code keeps for a signed-in account."""
    email, org, org_type, _plan, tier = PEOPLE.get(
        account_id, ("%s@example.dev" % account_id, "Example", "claude_pro", "pro",
                     "default_claude_pro"))
    return {"accountUuid": str(uuid.uuid5(uuid.NAMESPACE_URL, "demo-account:" + account_id)),
            "emailAddress": email, "organizationUuid": str(
                uuid.uuid5(uuid.NAMESPACE_URL, "demo-org:" + org)),
            "organizationName": org, "organizationType": org_type,
            "organizationRateLimitTier": tier,
            "displayName": "You" if account_id in PEOPLE else account_id.capitalize()}


def write_record(account_json, account_id):
    current = {}
    if os.path.exists(account_json):
        with open(account_json) as f:
            current = json.load(f)
    current.update({"hasCompletedOnboarding": True, "theme": "dark",
                    "oauthAccount": profile(account_id)})
    write_json(account_json, current)


def sign_in(config_dir, account_json, account_id):
    """What `claude auth login` leaves: a new chain in config_dir (its other
    keys kept) and the profile in account_json."""
    credentials = os.path.join(config_dir, ".credentials.json")
    require_container("setup_demo.sign_in", credentials, account_json)
    _email, _org, _org_type, plan, tier = PEOPLE.get(
        account_id, ("", "", "", "pro", "default_claude_pro"))
    current = {}
    if os.path.exists(credentials):
        with open(credentials) as f:
            current = json.load(f)
    current["claudeAiOauth"] = {
        "accessToken": "demo-%s" % token_name(account_id),
        "refreshToken": "demo-refresh-%s-%s" % (token_name(account_id), uuid.uuid4().hex[:8]),
        "expiresAt": int((time.time() + 30 * 86400) * 1000),
        "scopes": ["user:inference", "user:profile", "user:sessions:claude_code"],
        "subscriptionType": plan, "rateLimitTier": tier}
    write_json(credentials, current)
    write_record(account_json, account_id)


def make_account(account_id, signed_in=True):
    """An account's folder: links into ~/.claude, its profile, and its sign-in
    unless it is the global one (whose sign-in is in ~/.claude)."""
    folder = os.path.join(ROOT, account_id)
    os.makedirs(folder, mode=0o700, exist_ok=True)
    for items in SHARED.values():
        for item in items:
            target = os.path.join(HOME, ".claude", item)
            link = os.path.join(folder, item)
            if os.path.exists(target) and not os.path.lexists(link):
                os.symlink(target, link)
    if signed_in:
        sign_in(folder, os.path.join(folder, ".claude.json"), account_id)
    else:
        write_record(os.path.join(folder, ".claude.json"), account_id)


def codex_rollout():
    now = dt.datetime.now().astimezone()
    days = (1 - now.weekday()) % 7 or 7  # the next Tuesday
    resets = (now + dt.timedelta(days=days)).replace(hour=9, minute=48, second=0, microsecond=0)
    day = os.path.join(HOME, ".codex/sessions", now.strftime("%Y/%m/%d"))
    os.makedirs(day, exist_ok=True)
    thread = str(uuid.uuid4())
    stamp = now.astimezone(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.%f")[:-3] + "Z"
    lines = [
        {"timestamp": stamp, "type": "session_meta", "payload": {
            "id": thread, "timestamp": stamp, "cwd": HOME + "/projects/api", "source": "cli",
            "originator": "codex_cli_rs", "cli_version": "0.130.0"}},
        {"timestamp": stamp, "type": "event_msg", "payload": {
            "type": "token_count",
            "info": {"total_token_usage": {"input_tokens": 1200, "output_tokens": 80,
                                           "total_tokens": 1280},
                     "model_context_window": 475000},
            "rate_limits": {"limit_id": "codex", "limit_name": None,
                            "primary": {"used_percent": 16.0, "window_minutes": 10080,
                                        "resets_at": int(resets.timestamp())},
                            "secondary": None,
                            "credits": {"has_credits": True, "unlimited": False,
                                        "balance": "62500.0000000000"},
                            "plan_type": "pro"}}},
    ]
    name = "rollout-%s-%s.jsonl" % (now.strftime("%Y-%m-%dT%H-%M-%S"), thread)
    with open(os.path.join(day, name), "w") as f:
        for line in lines:
            f.write(json.dumps(line) + "\n")


def main():
    require_container("setup_demo.py")
    parser = argparse.ArgumentParser()
    parser.add_argument("--accounts", choices=["all", "two", "one"], default="all")
    args = parser.parse_args()
    claude_home = os.path.join(HOME, ".claude")
    for folder in ("projects", "skills", "agents", "commands", "todos", "plans"):
        os.makedirs(os.path.join(claude_home, folder), exist_ok=True)
    write_json(os.path.join(claude_home, "settings.json"), {"theme": "dark"}, 0o644)
    open(os.path.join(claude_home, "history.jsonl"), "a").close()
    sign_in(claude_home, os.path.join(HOME, ".claude.json"), "default")
    accounts = [{"id": "default", "name": "Work", "color": "blue", "shared": list(SHARED)}]
    ids = {"all": ["personal", "side-project"], "two": ["personal"], "one": []}[args.accounts]
    names = {"personal": ("Personal", "green"), "side-project": ("Side project", "violet")}
    if ids:
        # Work is global: its folder holds its profile, its sign-in is in ~/.claude.
        make_account("default", signed_in=False)
    for account_id in ids:
        make_account(account_id)
        name, color = names[account_id]
        accounts.append({"id": account_id, "name": name, "color": color,
                         "shared": list(SHARED), "created_at_ms": int(time.time() * 1000)})
    if ids:
        write_json(os.path.join(ROOT, "accounts.json"), {
            "version": 1, "global": "default", "accounts": accounts,
            "rules": {"mode": "suggest", "at": "week_90", "five_hour": False,
                      "back_after_reset": False, "order": ["personal"]},
            "dismissed": [], "last_switch": None, "slot_version": 2})
        os.chmod(ROOT, 0o700)
    codex_rollout()


if __name__ == "__main__":
    main()
