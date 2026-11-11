#!/usr/bin/env python3
"""A stand-in for Claude Code's usage endpoint (GET /api/oauth/usage), for the
demo container only. It answers per Bearer token with canned numbers that can
be changed while it runs, in the real answer's shape.

  mock_usage.py serve --port 59370         # loopback only
  mock_usage.py set demo-work week=91 five_hour=40 [five_hour_resets=18:10]
  mock_usage.py set demo-side-project status=429   # or 401, or 200 again
  mock_usage.py show

State lives in ~/demo/usage.json and is read again on every request, so
editing the file works too. A reset time is ISO 8601, `HH:MM` (today, or
tomorrow once past), `tomorrow HH:MM`, `<weekday> HH:MM` (the next one) or
`+90m` / `+3h`, in the container's TZ.
"""
import datetime as dt
import json
import os
import sys
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

STATE = os.path.expanduser("~/demo/usage.json")
WEEKDAYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]

DEFAULTS = {
    "demo-work": {"five_hour": 34, "five_hour_resets": "16:40", "week": 62,
                  "week_resets": "sat 14:00", "week_opus": 88, "week_sonnet": None},
    "demo-personal": {"five_hour": 8, "five_hour_resets": "18:20", "week": 12,
                      "week_resets": "thu 09:00", "week_opus": 5, "week_sonnet": None},
    "demo-side-project": {"five_hour": 100, "five_hour_resets": "18:10", "week": 91,
                          "week_resets": "tomorrow 08:00", "week_opus": None,
                          "week_sonnet": None},
    "*": {"five_hour": 3, "five_hour_resets": "+4h", "week": 4, "week_resets": "sun 10:00",
          "week_opus": None, "week_sonnet": None},
}


def load():
    try:
        with open(STATE) as f:
            return json.load(f)
    except (OSError, ValueError):
        return json.loads(json.dumps(DEFAULTS))


def save(state):
    os.makedirs(os.path.dirname(STATE), exist_ok=True)
    tmp = STATE + ".tmp"
    with open(tmp, "w") as f:
        json.dump(state, f, indent=2)
    os.replace(tmp, STATE)


def when(text):
    """A canned reset time as an aware datetime."""
    now = dt.datetime.now().astimezone()
    text = str(text).strip().lower()
    if text.startswith("+"):
        unit = text[-1]
        amount = float(text[1:-1])
        return now + (dt.timedelta(hours=amount) if unit == "h" else dt.timedelta(minutes=amount))
    try:
        parsed = dt.datetime.fromisoformat(text.upper().replace("Z", "+00:00"))
        return parsed if parsed.tzinfo else parsed.astimezone()
    except ValueError:
        pass
    parts = text.split()
    hh, mm = (int(x) for x in parts[-1].split(":"))
    base = now.replace(hour=hh, minute=mm, second=0, microsecond=0)
    if len(parts) == 1:
        return base if base > now else base + dt.timedelta(days=1)
    if parts[0] == "tomorrow":
        return base + dt.timedelta(days=1)
    target = WEEKDAYS.index(parts[0][:3])
    days = (target - now.weekday()) % 7
    out = base + dt.timedelta(days=days)
    return out if out > now else out + dt.timedelta(days=7)


def window(used, resets):
    if used is None:
        return None
    return {"utilization": used, "resets_at": when(resets).astimezone(dt.timezone.utc).isoformat(),
            "limit_dollars": None, "used_dollars": None, "remaining_dollars": None,
            "locked_reason": None}


def answer(entry):
    week_resets = entry.get("week_resets", "sun 10:00")
    five = window(entry.get("five_hour"), entry.get("five_hour_resets", "+4h"))
    week = window(entry.get("week"), week_resets)
    opus = window(entry.get("week_opus"), week_resets)
    sonnet = window(entry.get("week_sonnet"), week_resets)
    limits = []
    if week:
        limits.append({"kind": "weekly_all", "group": "weekly", "percent": week["utilization"],
                       "severity": "normal", "resets_at": week["resets_at"], "scope": None,
                       "is_active": True})
    for name, w in (("Opus", opus), ("Sonnet", sonnet)):
        if w:
            limits.append({"kind": "weekly_scoped", "group": "weekly",
                           "percent": w["utilization"], "severity": "normal",
                           "resets_at": w["resets_at"],
                           "scope": {"model": {"id": None, "display_name": name},
                                     "surface": None}, "is_active": False})
    return {"five_hour": five, "seven_day": week, "seven_day_oauth_apps": None,
            "seven_day_opus": opus, "seven_day_sonnet": sonnet,
            "extra_usage": {"is_enabled": False, "monthly_limit": None, "used_credits": None,
                            "utilization": None, "currency": None},
            "limits": limits}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        sys.stderr.write("%s %s\n" % (self.log_date_time_string(), fmt % args))

    def send(self, status, body):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        if self.path == "/state":
            return self.send(200, load())
        if not self.path.startswith("/api/oauth/usage"):
            return self.send(404, {"error": "not found"})
        auth = self.headers.get("Authorization", "")
        if not auth.startswith("Bearer ") or self.headers.get("anthropic-beta") != "oauth-2025-04-20":
            return self.send(401, {"error": {"type": "authentication_error"}})
        token = auth[len("Bearer "):]
        state = load()
        entry = state.get(token) or (state.get("*") if token.startswith("demo-") else None)
        if entry is None:
            return self.send(401, {"error": {"type": "authentication_error"}})
        status = int(entry.get("status", 200))
        if status != 200:
            return self.send(status, {"error": {"type": "rate_limit_error" if status == 429 else "error"}})
        sys.stderr.write("usage for %s\n" % token)
        self.send(200, answer(entry))

    def do_POST(self):
        if self.path != "/set":
            return self.send(404, {"error": "not found"})
        length = int(self.headers.get("Content-Length", "0"))
        body = json.loads(self.rfile.read(length) or b"{}")
        token = body.pop("token")
        state = load()
        entry = state.setdefault(token, dict(DEFAULTS["*"]))
        entry.update(body)
        save(state)
        self.send(200, {token: entry})


def value(text):
    if text in ("null", "none"):
        return None
    try:
        return float(text) if "." in text else int(text)
    except ValueError:
        return text


def main():
    # Only ever inside the demo container (demo/Dockerfile sets
    # DEMO_CONTAINER=1): it writes ~/demo/usage.json.
    if not (os.path.exists("/.dockerenv") and os.environ.get("DEMO_CONTAINER") == "1"):
        sys.exit("mock_usage.py runs only inside the demo container")
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    command = sys.argv[1]
    if command == "serve":
        port = int(sys.argv[sys.argv.index("--port") + 1])
        if not os.path.exists(STATE):
            save(DEFAULTS)
        ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
    elif command == "set":
        token, pairs = sys.argv[2], sys.argv[3:]
        body = {"token": token}
        for pair in pairs:
            key, _, raw = pair.partition("=")
            body[key] = value(raw)
        port = os.environ.get("MOCK_PORT")
        request = urllib.request.Request(
            "http://127.0.0.1:%s/set" % port, data=json.dumps(body).encode(),
            headers={"Content-Type": "application/json"}, method="POST")
        print(urllib.request.urlopen(request).read().decode())
    elif command == "show":
        print(json.dumps(load(), indent=2))
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
