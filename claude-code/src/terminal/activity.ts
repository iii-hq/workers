/**
 * Claude Code hooks in, AgentEvent frames out.
 *
 * The workspace hooks post every lifecycle event here, and this is where a
 * terminal turn becomes the same wire shape a headless agent worker emits:
 * a user message, an assistant message per tool call, a
 * `function_execution_start`/`end` pair with a real duration, and a
 * `turn_end`/`agent_end` when Claude stops. The console then renders this
 * terminal's turns like any other agent's, instead of the operator reading a
 * transport event and guessing.
 *
 * The function itself is plumbing — one call per hook — so it is
 * `trace_hidden`: the signal is the stream, not the delivery.
 */

import { randomUUID } from 'node:crypto';
import type { IIIClient } from 'iii-sdk';
import type { Emit } from '../events.js';
import { runTurnSpan } from '../trace.js';
import type {
  AgentMessage,
  AssistantMessage,
  ContentBlock,
  FunctionResultMessage,
  HookEvent,
} from './types.js';

const IDLE_MS = 60 * 60_000;

type ToolCall = { function_id: string; started_at: number };

/**
 * A readable name for one terminal session, in the shape pi's extension already
 * uses (`pi-<pid>-<base36 time>`): a bare UUID in a trace group tells a reader
 * nothing, while `claude-1c51db64-mtaevw6b` says which agent it is at a glance
 * and still carries Claude Code's own session id in the middle, so a transcript
 * on disk can be found from it.
 *
 * Minted once per session and reused, because it IS the group id: every event
 * of a session has to land in the same group.
 */
function groupName(claudeSessionId: string): string {
  const short = claudeSessionId.replace(/[^a-zA-Z0-9]/g, '').slice(0, 8) || 'session';
  return `claude-${short}-${Date.now().toString(36)}`;
}

type SessionState = {
  transcript: AgentMessage[];
  calls: Map<string, ToolCall>;
  results: FunctionResultMessage[];
  touched: number;
  /**
   * The turn every hook of this prompt belongs to, and what the prompt said.
   * A terminal turn arrives as several separate calls — one per hook — so the
   * turn identity cannot come from the call; it is opened at
   * `UserPromptSubmit` and reused until the next one.
   */
  turnId: string;
  prompt: string;
  /** What this session is called on the events stream and in a trace. */
  group: string;
};

/** `mcp__server__tool` is that server's function; everything else is ours. */
export function toolFunctionId(name: string): string {
  if (!name) return 'claude::tool';
  return name.startsWith('mcp__')
    ? name.replace(/^mcp__/, '').replace(/__/g, '::')
    : `claude::${name}`;
}

/**
 * A stable id for one tool call. Claude Code sends `tool_use_id` on recent
 * versions; without it, the tool plus its input identifies the call well
 * enough to pair a Pre with its Post.
 */
export function callId(event: HookEvent): string {
  if (event.tool_use_id) return event.tool_use_id;
  const input = JSON.stringify(event.tool_input ?? {});
  return `${event.tool_name ?? 'tool'}:${input.slice(0, 200)}`;
}

function assistant(content: ContentBlock[], stop_reason = 'tool_use'): AssistantMessage {
  return {
    role: 'assistant',
    content,
    stop_reason,
    error_message: null,
    usage: null,
    model: 'claude-code',
    provider: 'claude-code',
    timestamp: Date.now(),
  };
}

function resultContent(response: unknown): ContentBlock[] {
  if (response == null) return [];
  if (typeof response === 'string') return [{ type: 'text', text: response }];
  return [{ type: 'text', text: JSON.stringify(response) }];
}

function isError(response: unknown): boolean {
  if (!response || typeof response !== 'object') return false;
  const record = response as Record<string, unknown>;
  if (record.is_error === true || record.success === false) return true;
  return typeof record.exit_code === 'number' && record.exit_code !== 0;
}

export class ActivityTracker {
  private sessions = new Map<string, SessionState>();

  constructor(private readonly emit: Emit) {}

  private state(sessionId: string): SessionState {
    let state = this.sessions.get(sessionId);
    if (!state) {
      state = {
        transcript: [],
        calls: new Map(),
        results: [],
        touched: Date.now(),
        turnId: randomUUID(),
        prompt: '',
        group: groupName(sessionId),
      };
      this.sessions.set(sessionId, state);
    }
    state.touched = Date.now();
    this.sweep();
    return state;
  }

  /** A terminal left open for a day should not hold a day of transcripts. */
  private sweep(): void {
    const now = Date.now();
    for (const [id, state] of this.sessions) {
      if (now - state.touched > IDLE_MS) this.sessions.delete(id);
    }
  }

  /**
   * One hook, traced as part of its turn. The identity is the same set of
   * keys a harness turn stamps, so a terminal session's spans group and label
   * themselves in the console's trace views without the console knowing what
   * a Claude Code hook is.
   */
  async handle(event: HookEvent): Promise<{ ok: true; event: string }> {
    const name = event.hook_event_name ?? 'unknown';
    const sessionId = event.session_id || 'claude-code';
    const state = this.state(sessionId);
    if (name === 'UserPromptSubmit') {
      state.turnId = randomUUID();
      state.prompt = event.prompt ?? '';
    }
    return runTurnSpan(
      `claude terminal ${name}`,
      {
        sessionId: state.group,
        turnId: state.turnId,
        kind: 'claude.terminal.turn',
        message: state.prompt,
        displayName: state.prompt ? `Claude terminal · ${state.prompt}` : 'Claude terminal',
      },
      () => this.apply(name, event, sessionId, state),
    );
  }

  private async apply(
    name: string,
    event: HookEvent,
    sessionId: string,
    state: SessionState,
  ): Promise<{ ok: true; event: string }> {
    // Events go out under the readable name; `sessionId` stays Claude Code's
    // own id because it is the key `SessionEnd` forgets.
    const group = state.group;
    switch (name) {
      case 'UserPromptSubmit': {
        const message: AgentMessage = {
          role: 'user',
          content: [{ type: 'text', text: event.prompt ?? '' }],
          timestamp: Date.now(),
        };
        state.transcript.push(message);
        state.results = [];
        await this.emit(group, { type: 'message_complete', message });
        break;
      }

      case 'PreToolUse': {
        const id = callId(event);
        const function_id = toolFunctionId(event.tool_name ?? '');
        state.calls.set(id, { function_id, started_at: Date.now() });
        const message = assistant([
          { type: 'function_call', id, function_id, arguments: event.tool_input ?? {} },
        ]);
        state.transcript.push(message);
        await this.emit(group, { type: 'message_complete', message });
        await this.emit(group, {
          type: 'function_execution_start',
          function_call_id: id,
          function_id,
          args: event.tool_input ?? {},
        });
        break;
      }

      case 'PostToolUse': {
        const id = callId(event);
        const call = state.calls.get(id);
        state.calls.delete(id);
        const function_id = call?.function_id ?? toolFunctionId(event.tool_name ?? '');
        const content = resultContent(event.tool_response);
        const failed = isError(event.tool_response);
        const result: FunctionResultMessage = {
          role: 'function_result',
          function_call_id: id,
          function_id,
          content,
          details: null,
          is_error: failed,
          timestamp: Date.now(),
        };
        state.transcript.push(result);
        state.results.push(result);
        await this.emit(group, {
          type: 'function_execution_end',
          function_call_id: id,
          function_id,
          result: { content, details: null },
          is_error: failed,
          duration_ms: call ? Date.now() - call.started_at : 0,
        });
        break;
      }

      case 'Stop': {
        const message = assistant([], 'end');
        state.transcript.push(message);
        await this.emit(group, {
          type: 'turn_end',
          message,
          function_results: state.results,
        });
        await this.emit(group, { type: 'agent_end', messages: state.transcript });
        state.results = [];
        break;
      }

      case 'SessionEnd':
        this.sessions.delete(sessionId);
        break;

      default:
        // SessionStart and anything a newer CLI adds: the session bucket now
        // exists, which is all an unmapped event has to do.
        break;
    }

    return { ok: true, event: name };
  }
}

export function registerActivity(iii: IIIClient, emit: Emit): ActivityTracker {
  const tracker = new ActivityTracker(emit);
  iii.registerFunction(
    'claude::terminal::activity',
    (input: HookEvent) => tracker.handle(input ?? {}),
    {
      description:
        'A Claude Code lifecycle event from a terminal session (SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, Stop, SessionEnd), posted by the workspace hooks. Translated into AgentEvent frames on the events stream.',
      request_format: {
        type: 'object',
        properties: {
          hook_event_name: { type: 'string' },
          session_id: { type: 'string' },
          cwd: { type: 'string' },
          prompt: { type: 'string' },
          tool_name: { type: 'string' },
          tool_use_id: { type: 'string' },
          tool_input: { type: 'object' },
          tool_response: {},
        },
      },
      response_format: {
        type: 'object',
        required: ['ok', 'event'],
        properties: { ok: { type: 'boolean' }, event: { type: 'string' } },
      },
      metadata: { internal: true, trace_hidden: true },
    },
  );
  return tracker;
}
