#!/usr/bin/env python3
"""Send one Harness message through the Console WebSocket proxy."""

import argparse
import asyncio
import json
import sys
import time
import uuid

import websockets


def progress(message: str) -> None:
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
        if (
            frame.get("type") != "invocationresult"
            or frame.get("invocation_id") != invocation_id
        ):
            continue
        if frame.get("error"):
            raise RuntimeError(f"{function_id} failed: {json.dumps(frame['error'])}")
        return frame.get("result")


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
            return messages
        cursor = next_cursor


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


async def run(args) -> None:
    async with websockets.connect(args.url, max_size=None, open_timeout=30) as ws:
        sent = await invoke(
            ws,
            "harness::send",
            {
                "message": args.prompt,
                "model": args.model,
                "provider": args.provider,
                "session": {"title": "Harness quickstart GLM canary"},
                "options": {
                    "max_turns": 1,
                    "max_output_tokens": 1024,
                    "max_total_tokens": 16384,
                },
            },
        )
        if not sent or not sent.get("accepted"):
            raise RuntimeError(f"harness::send was not accepted: {json.dumps(sent)}")

        session_id = sent["session_id"]
        turn_id = sent.get("turn_id")
        progress(f"turn {turn_id} accepted on session {session_id}")

        deadline = time.monotonic() + args.timeout
        while True:
            status = await invoke(ws, "harness::status", {"session_id": session_id})
            state = (status or {}).get("status", "unknown")
            if state in ("failed", "cancelled"):
                error = status.get("result_error") or "no error was reported"
                raise RuntimeError(f"turn ended as {state}: {error}")
            if state == "completed" and not status.get("expects_wake"):
                break
            if time.monotonic() >= deadline:
                raise TimeoutError(
                    f"turn did not complete within {args.timeout}s "
                    f"(last status: {state})"
                )
            await asyncio.sleep(2)

        reply = last_assistant_text(await transcript(ws, session_id))
        if not reply:
            raise RuntimeError("session transcript has no non-empty assistant reply")

        progress(f"assistant replied ({len(reply)} chars)")
        json.dump(
            {"session_id": session_id, "turn_id": turn_id, "reply": reply},
            sys.stdout,
        )
        print(flush=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", required=True)
    parser.add_argument("--model", required=True)
    parser.add_argument("--provider", required=True)
    parser.add_argument("--prompt", required=True)
    parser.add_argument("--timeout", type=int, default=240)
    try:
        asyncio.run(run(parser.parse_args()))
    except Exception as error:
        progress(f"FAIL: {error}")
        raise SystemExit(1) from error


if __name__ == "__main__":
    main()
