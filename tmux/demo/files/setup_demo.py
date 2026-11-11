#!/usr/bin/env python3
"""Fake Claude Code accounts and Codex records for the demo container, as the
artboards show them. Every token is a fake string (`demo-<account>`) that only
the mock usage endpoint knows.

  setup_demo.py --accounts all   # Work (default), Personal, Side project
  setup_demo.py --accounts two   # Work, Personal (Add account makes Side project)
  setup_demo.py --accounts one   # Work only, no accounts.json
"""
import argparse
import datetime as dt
import json
import os
import time
import uuid

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


def sign_in(config_dir, account_json, account_id):
    email, org, org_type, plan, tier = PEOPLE.get(
        account_id, ("%s@example.dev" % account_id, "Example", "claude_pro", "pro",
                     "default_claude_pro"))
    write_json(os.path.join(config_dir, ".credentials.json"), {"claudeAiOauth": {
        "accessToken": "demo-%s" % ("work" if account_id == "default" else account_id),
        "refreshToken": "demo-refresh", "expiresAt": int((time.time() + 30 * 86400) * 1000),
        "scopes": ["user:inference", "user:profile", "user:sessions:claude_code"],
        "subscriptionType": plan, "rateLimitTier": tier}})
    current = {}
    if os.path.exists(account_json):
        with open(account_json) as f:
            current = json.load(f)
    current.update({"hasCompletedOnboarding": True, "theme": "dark", "oauthAccount": {
        "emailAddress": email, "organizationName": org, "organizationType": org_type,
        "organizationRateLimitTier": tier, "displayName": "You"}})
    write_json(account_json, current)


def make_account(account_id):
    folder = os.path.join(ROOT, account_id)
    os.makedirs(folder, mode=0o700, exist_ok=True)
    for items in SHARED.values():
        for item in items:
            target = os.path.join(HOME, ".claude", item)
            link = os.path.join(folder, item)
            if os.path.exists(target) and not os.path.lexists(link):
                os.symlink(target, link)
    sign_in(folder, os.path.join(folder, ".claude.json"), account_id)


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
    parser = argparse.ArgumentParser()
    parser.add_argument("--accounts", choices=["all", "two", "one"], default="all")
    args = parser.parse_args()
    claude_home = os.path.join(HOME, ".claude")
    for folder in ("projects", "skills", "agents", "commands", "todos", "plans"):
        os.makedirs(os.path.join(claude_home, folder), exist_ok=True)
    write_json(os.path.join(claude_home, "settings.json"), {"theme": "dark"}, 0o644)
    open(os.path.join(claude_home, "history.jsonl"), "a").close()
    sign_in(claude_home, os.path.join(HOME, ".claude.json"), "default")
    accounts = [{"id": "default", "name": "Work", "color": "blue"}]
    ids = {"all": ["personal", "side-project"], "two": ["personal"], "one": []}[args.accounts]
    names = {"personal": ("Personal", "green"), "side-project": ("Side project", "violet")}
    for account_id in ids:
        make_account(account_id)
        name, color = names[account_id]
        accounts.append({"id": account_id, "name": name, "color": color,
                         "shared": list(SHARED), "created_at_ms": int(time.time() * 1000)})
    if ids:
        write_json(os.path.join(ROOT, "accounts.json"), {
            "version": 1, "global": "default", "accounts": accounts,
            "rules": {"mode": "suggest", "at": "week_90", "five_hour": False,
                      "move_idle": True, "back_after_reset": False, "order": ["personal"]},
            "dismissed": [], "last_switch": None})
        os.chmod(ROOT, 0o700)
    codex_rollout()


if __name__ == "__main__":
    main()
