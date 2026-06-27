"""hermes::* function handlers + the inbound bridge.

The agent loop (`hermes::run`) shells the documented `hermes -z` one-shot, which
returns only the final response text -- so the translated `agent::events` view
carries `turn_end` + `agent_end` with the final message, not per-tool frames
(Hermes one-shot exposes no event stream). `hermes::send` is omnichannel out.
Inbound platform/webhook deliveries land on the HTTP sink and are republished so
iii workers can react.
"""

from __future__ import annotations

import asyncio
import logging
import time
import uuid
from typing import Any

from iii import IIIClient
from iii_helpers.http import HttpRequest, HttpResponse

from . import hermes_cli
from .iii_prompt import III_CONTEXT_PROMPT

SCOPE = "hermes_sessions"
JSON_HEADERS = {"Content-Type": "application/json"}

# Per-process epoch + per-session monotonic counter so item_ids never collide,
# including across resumed runs on the same session_id (a fixed per-call seq
# would make turn 2's frames overwrite turn 1's). Mirrors the harness emitter.
_PROCESS_EPOCH = uuid.uuid4().hex[:8]
_seq_by_session: dict[str, int] = {}


async def _emit(iii: IIIClient, stream_name: str, session_id: str, event: dict[str, Any]) -> None:
    seq = _seq_by_session.get(session_id, 0)
    _seq_by_session[session_id] = seq + 1
    item_id = f"{session_id}-{_PROCESS_EPOCH}-{seq:08d}"
    await iii.trigger_async(
        {
            "function_id": "stream::set",
            "payload": {"stream_name": stream_name, "group_id": session_id, "item_id": item_id, "data": event},
        }
    )


def _extract_prompt(data: dict[str, Any]) -> str:
    if isinstance(data.get("prompt"), str):
        return data["prompt"]
    messages = data.get("messages") or []
    users = [m for m in messages if m.get("role") == "user"]
    if not users:
        raise ValueError("hermes::run requires `prompt` or a user message in `messages`")
    content = users[-1].get("content")
    if isinstance(content, str):
        return content
    blocks = content if isinstance(content, list) else []
    return "\n".join(b.get("text", "") for b in blocks if isinstance(b, dict) and b.get("text"))


def create_handlers(iii: IIIClient, get_cfg, logger: logging.Logger) -> dict[str, Any]:
    live: set[str] = set()

    async def run(data: dict[str, Any]) -> dict[str, Any]:
        cfg = get_cfg()
        session_id = data.get("session_id") or str(uuid.uuid4())
        if session_id in live:
            return {"session_id": session_id, "busy": True, "reason": "a run is already active for this session"}
        live.add(session_id)
        try:
            prompt = _extract_prompt(data)
            cwd = data.get("cwd") or cfg["defaults"]["cwd"]
            model = data.get("model") or cfg["defaults"]["model"]
            tools = data.get("tools")
            tools = cfg["defaults"].get("tools", "") if tools is None else tools
            prior = await iii.trigger_async(
                {"function_id": "state::get", "payload": {"scope": SCOPE, "key": session_id}}
            )
            iii_ctx = data.get("iii_context")
            iii_ctx = cfg["iii_context"] if iii_ctx is None else iii_ctx
            prompt_text = f"{III_CONTEXT_PROMPT}\n\n---\n\n{prompt}" if (iii_ctx and not prior) else prompt

            is_error = False
            try:
                result, _ = await hermes_cli.run_turn(
                    cfg["hermes_executable"] or "hermes",
                    prompt_text,
                    cwd=cwd,
                    model=model,
                    session=session_id,
                    toolsets=tools,
                )
            except Exception as exc:  # noqa: BLE001 - surface the failure in the envelope
                result, is_error = str(exc), True

            usage_info = None if is_error else hermes_cli.read_latest_usage()

            record = {
                "session_id": session_id,
                "cwd": cwd,
                "model": model,
                "status": "error" if is_error else "done",
                "turns": (prior or {}).get("turns", 0) + 1,
                "updated_at_ms": int(time.time() * 1000),
            }
            if usage_info:
                record["usage"] = usage_info["usage"]
                record["total_cost_usd"] = usage_info["total_cost_usd"]
            await iii.trigger_async(
                {"function_id": "state::set", "payload": {"scope": SCOPE, "key": session_id, "value": record}}
            )

            message = {"role": "assistant", "content": [{"type": "text", "text": result}], "provider": "hermes"}
            await _emit(
                iii, cfg["raw_events_stream"], session_id, {"type": "result", "text": result, "is_error": is_error}
            )
            await _emit(
                iii,
                cfg["events_stream"],
                session_id,
                {"type": "turn_end", "message": message, "function_results": []},
            )
            end_event = {"type": "agent_end", "messages": [message]}
            if usage_info:
                end_event["usage"] = usage_info["usage"]
                end_event["total_cost_usd"] = usage_info["total_cost_usd"]
            await _emit(iii, cfg["events_stream"], session_id, end_event)
            envelope = {
                "session_id": session_id,
                "result": result,
                "is_error": is_error,
                "stop_reason": "error" if is_error else "end",
            }
            if usage_info:
                envelope["usage"] = usage_info["usage"]
                envelope["total_cost_usd"] = usage_info["total_cost_usd"]
                envelope["cost_source"] = usage_info["cost_source"]
            return envelope
        finally:
            live.discard(session_id)

    async def start(data: dict[str, Any]) -> dict[str, Any]:
        session_id = data.get("session_id") or str(uuid.uuid4())
        if session_id in live:
            return {
                "session_id": session_id,
                "started": False,
                "busy": True,
                "reason": "a run is already active for this session",
            }
        task = asyncio.create_task(run({**data, "session_id": session_id}))
        task.add_done_callback(
            lambda t: (
                t.exception()
                and logger.error(
                    "hermes::start background run failed: session_id=%s error=%s",
                    session_id,
                    t.exception(),
                )
            )
        )
        return {"session_id": session_id, "started": True}

    async def send(data: dict[str, Any]) -> dict[str, Any]:
        cfg = get_cfg()
        platform = data.get("platform")
        message = data.get("message")
        if not platform or message is None:
            raise ValueError("hermes::send requires `platform` and `message`")
        out = await hermes_cli.send(cfg["hermes_executable"] or "hermes", platform, str(message))
        return {"platform": platform, "sent": True, "detail": out}

    async def sessions_list(_data: dict[str, Any]) -> dict[str, Any]:
        cfg = get_cfg()
        stored = await iii.trigger_async({"function_id": "state::list", "payload": {"scope": SCOPE}}) or []
        raw = await hermes_cli.sessions_list(cfg["hermes_executable"] or "hermes")
        return {"sessions": stored, "hermes_sessions_raw": raw}

    async def status(data: dict[str, Any]) -> dict[str, Any]:
        session_id = data.get("session_id", "")
        record = await iii.trigger_async({"function_id": "state::get", "payload": {"scope": SCOPE, "key": session_id}})
        return {"session_id": session_id, "live": session_id in live, "record": record}

    async def stop(data: dict[str, Any]) -> dict[str, Any]:
        session_id = data.get("session_id", "")
        # `hermes -z` is a short-lived subprocess; there is no live-interrupt
        # handle to cancel mid-turn. Reported for parity; a true interrupt path
        # depends on a long-running gateway run (live-integration item).
        return {"session_id": session_id, "stopped": False, "reason": "hermes one-shot turns are not interruptible"}

    async def inbound(req: HttpRequest[Any], log: logging.Logger) -> HttpResponse[Any]:
        cfg = get_cfg()
        body = req.body or {}
        # Republish the inbound platform/webhook delivery so iii workers can
        # react. TODO(live-integration): the exact Hermes delivery payload shape
        # is confirmed against a running gateway; map it onto a dedicated
        # `hermes::message` trigger type (register_trigger_type) once verified.
        gid = str(body.get("session_id") or body.get("chat_id") or uuid.uuid4())
        await _emit(iii, cfg["raw_events_stream"], gid, {"type": "inbound", "body": body})
        log.info("hermes inbound delivery: group_id=%s", gid)
        return HttpResponse(statusCode=200, body={"ok": True}, headers=JSON_HEADERS)

    return {
        "run": run,
        "start": start,
        "send": send,
        "sessions_list": sessions_list,
        "status": status,
        "stop": stop,
        "inbound": inbound,
    }
