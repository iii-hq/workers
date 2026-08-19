#!/usr/bin/env python3
"""Hermetic public-wrapper differential checks for the HTTP compatibility tier."""

from __future__ import annotations

import argparse
import asyncio
import gzip
import html
import json
import subprocess
import sys
import threading
import zlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parent.parent
STANDALONE = ROOT.parent / "scrapling"


class Server(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self) -> None:
        super().__init__(("127.0.0.1", 0), Handler)
        self.attempts: dict[str, int] = {}


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_: object) -> None:
        pass

    def do_GET(self) -> None:
        self.respond()

    def do_POST(self) -> None:
        self.respond()

    def do_PUT(self) -> None:
        self.respond()

    def do_DELETE(self) -> None:
        self.respond()

    def respond(self) -> None:
        parsed = urlsplit(self.path)
        if parsed.path == "/chain":
            remaining = int(parsed.query or "0")
            if remaining:
                return self.send(302, b"chain", [("Location", f"/chain?{remaining - 1}")])
        if parsed.path == "/redirect":
            return self.send(302, b"redirect", [("Location", "/echo?redirected=1")])
        if parsed.path == "/loop":
            return self.send(302, b"loop", [("Location", "/loop")])
        if parsed.path == "/flaky":
            attempts = self.server.attempts  # type: ignore[attr-defined]
            attempts[parsed.query] = attempts.get(parsed.query, 0) + 1
            if attempts[parsed.query] == 1:
                self.connection.shutdown(2)
                self.connection.close()
                return
        if parsed.path == "/latin":
            return self.send(
                200,
                b"<p>caf\xe9</p>",
                [("Content-Type", "text/html; charset=iso-8859-1")],
            )
        if parsed.path == "/gzip":
            return self.send(
                200,
                gzip.compress(b"<p>compressed</p>", mtime=0),
                [("Content-Type", "text/html"), ("Content-Encoding", "gzip")],
            )
        if parsed.path == "/deflate":
            return self.send(
                200,
                zlib.compress(b"<p>compressed</p>"),
                [("Content-Type", "text/html"), ("Content-Encoding", "deflate")],
            )
        if parsed.path == "/duplicates":
            return self.send(
                201,
                b"<p>duplicates</p>",
                [
                    ("Content-Type", "text/html"),
                    ("X-Test", "one"),
                    ("X-Test", "two"),
                    ("Set-Cookie", "first=1; Path=/"),
                    ("Set-Cookie", "second=2; Path=/"),
                ],
            )
        if parsed.path == "/set-cookie":
            return self.send(
                200,
                b"<p>stored</p>",
                [("Content-Type", "text/html"), ("Set-Cookie", "sid=abc; Path=/")],
            )
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length).decode("utf-8", "replace")
        selected = [
            [name.lower(), value]
            for name, value in self.headers.items()
            if name.lower()
            in {"authorization", "content-type", "cookie", "proxy-authorization", "x-first", "x-second"}
        ]
        echoed = json.dumps(
            {"method": self.command, "target": self.path, "body": body, "headers": selected},
            ensure_ascii=False,
            separators=(",", ":"),
        )
        self.send(200, f"<pre>{html.escape(echoed)}</pre>".encode())

    def send(self, status: int, body: bytes, headers: list[tuple[str, str]] | None = None) -> None:
        self.send_response_only(status)
        for name, value in headers or [("Content-Type", "text/html; charset=utf-8")]:
            self.send_header(name, value)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)


class Oracle:
    def __init__(self) -> None:
        sys.path.insert(0, str(STANDALONE))
        from src import sessions
        from src.handlers import create_handlers

        self.sessions = sessions.setup(max_sessions=8, idle_timeout=900)
        self.handlers = create_handlers(lambda: {})
        self.loop = asyncio.new_event_loop()

    def query(self, function: str, payload: dict[str, Any]) -> dict[str, Any]:
        name = function.removeprefix("browser::").replace("-", "_")
        try:
            return {"ok": self.loop.run_until_complete(self.handlers[name](payload))}
        except Exception as error:  # noqa: BLE001 - exact error text is contract data
            return {"err": str(error)}

    def close(self) -> None:
        self.sessions.close_all()
        self.loop.close()


class Driver:
    def __init__(self) -> None:
        subprocess.run(
            ["cargo", "build", "--quiet", "--example", "scrapling_http_differential", "--features", "scrapling-compat"],
            cwd=ROOT,
            check=True,
        )
        metadata = json.loads(
            subprocess.check_output(["cargo", "metadata", "--format-version=1", "--no-deps"], cwd=ROOT)
        )
        executable = Path(metadata["target_directory"]) / "debug/examples/scrapling_http_differential"
        self.process = subprocess.Popen([executable], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)

    def query(self, function: str, payload: dict[str, Any]) -> dict[str, Any]:
        assert self.process.stdin and self.process.stdout
        json.dump({"function": function, "payload": payload}, self.process.stdin, separators=(",", ":"))
        self.process.stdin.write("\n")
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        if not line:
            raise RuntimeError("Rust HTTP differential driver stopped")
        return json.loads(line)

    def close(self) -> None:
        self.process.terminate()
        self.process.wait()


def normalized(value: Any, origin: str) -> Any:
    if isinstance(value, str):
        return value.replace(origin, "{origin}")
    if isinstance(value, list):
        return [normalized(item, origin) for item in value]
    if isinstance(value, dict):
        return {key: normalized(item, origin) for key, item in value.items()}
    return value


def cases(origin: str) -> list[tuple[str, dict[str, Any]]]:
    def request(path: str, **values: Any) -> dict[str, Any]:
        return {
            "url": origin + path,
            "impersonate": "",
            "stealthy_headers": False,
            "retries": 1,
            "include_html": True,
            **values,
        }

    return [
        ("get", request("/echo")),
        ("default-impersonation", {"url": origin + "/echo", "retries": 1, "include_html": True}),
        ("params", request("/echo?old=1", params={"old": "2", "many": [1, 2], "flag": True, "none": None})),
        ("post-form", request("/echo", method="post", data={"a": 1, "flag": True, "none": None})),
        ("post-json", request("/echo", method="post", json={"a": [1, True, None]})),
        ("put", request("/echo", method="put", data={"a": "b"})),
        ("delete", request("/echo", method="delete", json={"delete": True})),
        ("headers", request("/echo", headers={"x-second": "2", "x-first": "1", "x-empty": ""})),
        ("cookies", request("/echo", cookies={"second": "2", "first": "1"})),
        ("auth", request("/echo", auth=["user", "pass"])),
        ("redirect", request("/redirect")),
        ("redirect-all", request("/redirect", follow_redirects=True)),
        ("no-redirect", request("/redirect", follow_redirects=False)),
        ("duplicates", request("/duplicates")),
        ("latin", request("/latin")),
        ("gzip", request("/gzip")),
        ("deflate", request("/deflate")),
        ("retry", request("/flaky?one", retries=2, retry_delay=0)),
        ("bulk", {**request("/unused"), "url": None, "urls": [origin + "/echo?a", origin + "/latin"]}),
        ("missing-url", {"retries": 1}),
        ("bad-method", request("/echo", method="patch")),
        ("bad-impersonation", request("/echo", impersonate="bogus")),
        ("proxy-conflict", request("/echo", proxy="http://one", proxies={"all": "http://two"})),
        ("bad-auth", request("/echo", auth=["one"])),
        ("negative-timeout", request("/echo", timeout=-1)),
        ("negative-redirects", request("/echo", max_redirects=-2)),
        ("negative-retries", request("/echo", retries=-1)),
        ("negative-delay", request("/flaky?delay", retries=2, retry_delay=-1)),
        ("redirect-limit", request("/loop", max_redirects=1)),
        ("unlimited-redirects", request("/chain?35", follow_redirects=True, max_redirects=-1)),
        (
            "proxy",
            {**request("/unused"), "url": "http://example.invalid/echo", "proxy": origin},
        ),
        (
            "proxies-scheme",
            {**request("/unused"), "url": "http://example.invalid/echo", "proxies": {"http": origin}},
        ),
        (
            "proxies-host",
            {
                **request("/unused"),
                "url": "http://example.invalid/echo",
                "proxies": {"http://example.invalid": origin},
            },
        ),
        (
            "proxy-auth",
            {
                **request("/unused"),
                "url": "http://example.invalid/echo",
                "proxy": origin,
                "proxy_auth": ["proxy-user", "proxy-pass"],
            },
        ),
    ]


def session_checks(oracle: Oracle, driver: Driver, origin: str) -> list[tuple[str, Any, Any, Any]]:
    mismatches = []
    constructor = {"type": "http", "impersonate": "", "headers": {"x-first": "session"}}
    expected_open = oracle.query("browser::session-open", constructor)
    actual_open = driver.query("browser::session-open", constructor)
    expected_id = expected_open.get("ok", {}).get("session_id")
    actual_id = actual_open.get("ok", {}).get("session_id")
    for label, session_id in [("oracle", expected_id), ("rust", actual_id)]:
        valid = (
            isinstance(session_id, str)
            and len(session_id) == 32
            and all(character in "0123456789abcdef" for character in session_id)
            and session_id[12] == "4"
            and session_id[16] in "89ab"
        )
        if not valid:
            mismatches.append((f"session-open-{label}-uuid", constructor, "UUID4 hex", session_id))
    if not isinstance(expected_id, str) or not isinstance(actual_id, str):
        return mismatches
    expected_open["ok"]["session_id"] = "{session}"
    actual_open["ok"]["session_id"] = "{session}"
    if expected_open != actual_open:
        mismatches.append(("session-open", constructor, expected_open, actual_open))

    for name, path in [("session-set-cookie", "/set-cookie"), ("session-cookie-state", "/echo")]:
        common = {"url": origin + path, "include_html": True}
        expected = normalized(
            oracle.query("browser::session-fetch", {"session_id": expected_id, **common}), origin
        )
        actual = normalized(
            driver.query("browser::session-fetch", {"session_id": actual_id, **common}), origin
        )
        if expected != actual:
            mismatches.append((name, common, expected, actual))

    expected_list = oracle.query("browser::session-list", {})
    actual_list = driver.query("browser::session-list", {})
    for value, session_id in [(expected_list, expected_id), (actual_list, actual_id)]:
        for item in value.get("ok", {}).get("sessions", []):
            if item.get("session_id") == session_id:
                item["session_id"] = "{session}"
            created_at = item.get("created_at")
            last_used = item.get("last_used")
            if isinstance(created_at, int | float) and isinstance(last_used, int | float):
                if last_used < created_at:
                    mismatches.append(
                        ("session-list-time-order", {}, "last_used >= created_at", item.copy())
                    )
            for key in ("created_at", "last_used", "idle_s"):
                number = item.get(key)
                if not isinstance(number, int | float) or number < 0:
                    mismatches.append((f"session-list-{key}", {}, "non-negative number", number))
                item[key] = "{number}"
    if expected_list != actual_list:
        mismatches.append(("session-list", {}, expected_list, actual_list))

    for name in ("session-close", "session-close-again"):
        expected = oracle.query("browser::session-close", {"session_id": expected_id})
        actual = driver.query("browser::session-close", {"session_id": actual_id})
        if expected != actual:
            mismatches.append((name, {}, expected, actual))
    return mismatches


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--oracle-check", choices=["full", "parser-runtime", "none"], default="full")
    args = parser.parse_args()
    if sys.version_info[:3] != (3, 12, 13):
        parser.error(f"requires frozen CPython 3.12.13, got {sys.version.split()[0]}")
    if args.oracle_check != "none":
        command = [sys.executable, ROOT / "scripts/verify_oracle.py"]
        if args.oracle_check == "parser-runtime":
            command.append("--parser-runtime")
        subprocess.run(command, check=True)

    server = Server()
    origin = f"http://127.0.0.1:{server.server_port}"
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    oracle, driver = Oracle(), Driver()
    mismatches = []
    try:
        for name, payload in cases(origin):
            server.attempts.clear()
            expected = normalized(oracle.query("browser::fetch", payload), origin)
            server.attempts.clear()
            actual = normalized(driver.query("browser::fetch", payload), origin)
            if expected != actual:
                mismatches.append((name, payload, expected, actual))
        mismatches.extend(session_checks(oracle, driver, origin))
    finally:
        oracle.close()
        driver.close()
        server.shutdown()
        server.server_close()

    for name, payload, expected, actual in mismatches:
        print(f"{name}: payload={json.dumps(normalized(payload, origin), ensure_ascii=False)}")
        print(f"  expected={json.dumps(expected, ensure_ascii=False)}")
        print(f"  actual={json.dumps(actual, ensure_ascii=False)}")
    if mismatches:
        print(f"FAILED: {len(mismatches)} HTTP/session mismatches", file=sys.stderr)
        return 1
    print(f"PASS HTTP: {len(cases(origin))} request cases plus persistent session state")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
