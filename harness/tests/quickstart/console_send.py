#!/usr/bin/env python3
"""Send a message through the Console's /ws proxy and wait for the reply.

Speaks the engine WebSocket protocol (invokefunction/invocationresult)
through the Console proxy — the same path the browser SPA uses — so a pass
proves the Console-to-engine-to-provider chain end to end:

  1. harness::send {message, model, provider}
  2. poll harness::status until the turn completes (fail on failed/cancelled)
  3. session::messages -> last non-empty assistant text

Prints {"session_id", "turn_id", "reply"} as JSON on stdout; progress and
errors go to stderr. Requires the `websockets` package.
"""

import argparse
import asyncio
import json
import sys
import time
import uuid

import websockets


def eprint(message: str) -> None:
    print(f"[console-send] {message}", file=sys.stderr, flush=True)


async def invoke(ws, function_id: str, data: dict, timeout: float = 30.0):
    invocation_id = str(uuid.uuid4())
    await ws.send(
        json.dumps(
            {
                "type": "invokefunction",
                "invocation_id": invocation_id,
                "function_id": function_id,
                "data": data,
            }
        )
    )
    deadline = time.monotonic() + timeout
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError(f"{function_id}: no invocationresult within {timeout}s")
        frame = json.loads(await asyncio.wait_for(ws.recv(), remaining))
        # The proxy is transparent, so unrelated engine frames may interleave.
        if (
            frame.get("type") != "invocationresult"
            or frame.get("invocation_id") != invocation_id
        ):
            continue
        if frame.get("error"):
            raise RuntimeError(f"{function_id} failed: {json.dumps(frame['error'])}")
        return frame.get("result")


def last_assistant_text(messages: list) -> str:
    texts = []
    for item in messages:
        message = item.get("message") or {}
        if message.get("role") != "assistant":
            continue
        for part in message.get("content") or []:
            if part.get("type") == "text" and (part.get("text") or "").strip():
                texts.append(part["text"].strip())
    return texts[-1] if texts else ""


async def transcript(ws, session_id: str) -> list:
    messages, cursor = [], None
    while True:
        page = await invoke(
            ws,
            "session::messages",
            {
                "session_id": session_id,
                "limit": 500,
                "cursor": cursor,
                "include_custom": True,
            },
        )
        messages.extend(page.get("messages") or [])
        next_cursor = page.get("next_cursor")
        if not next_cursor or next_cursor == cursor:
            break
        cursor = next_cursor
    return messages


async def run(args) -> None:
    async with websockets.connect(args.url, max_size=None) as ws:
        send = await invoke(
            ws,
            "harness::send",
            {
                "message": args.prompt,
                "model": args.model,
                "provider": args.provider,
                "session": {"title": "Harness quickstart validation"},
                "options": {
                    "max_turns": 2,
                    "max_output_tokens": 1024,
                    "max_total_tokens": 16384,
                },
            },
        )
        if not send or not send.get("accepted"):
            raise RuntimeError(f"harness::send was not accepted: {json.dumps(send)}")
        session_id = send["session_id"]
        turn_id = send.get("turn_id")
        eprint(f"turn {turn_id} accepted on session {session_id}")

        deadline = time.monotonic() + args.timeout
        last_status = "unknown"
        while True:
            status = await invoke(ws, "harness::status", {"session_id": session_id})
            last_status = status.get("status")
            if last_status in ("failed", "cancelled"):
                error = status.get("result_error") or "no error was reported"
                raise RuntimeError(f"turn ended as {last_status}: {error}")
            if last_status == "completed" and not status.get("expects_wake"):
                eprint("turn completed")
                break
            if time.monotonic() > deadline:
                raise TimeoutError(
                    f"turn did not complete within {args.timeout}s "
                    f"(last status: {last_status})"
                )
            await asyncio.sleep(2)

        reply = last_assistant_text(await transcript(ws, session_id))
        if not reply:
            raise RuntimeError("no non-empty assistant reply in the session transcript")
        eprint(f"assistant replied ({len(reply)} chars)")
        json.dump(
            {"session_id": session_id, "turn_id": turn_id, "reply": reply},
            sys.stdout,
        )
        print(flush=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", required=True, help="Console proxy, e.g. ws://127.0.0.1:3113/ws")
    parser.add_argument("--model", required=True)
    parser.add_argument("--provider", required=True)
    parser.add_argument("--prompt", required=True)
    parser.add_argument("--timeout", type=int, default=240, help="turn completion timeout in seconds")
    try:
        asyncio.run(run(parser.parse_args()))
    except Exception as error:  # surface a single clean line for the validator
        eprint(f"FAIL: {error}")
        sys.exit(1)


if __name__ == "__main__":
    main()
