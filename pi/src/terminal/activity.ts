/**
 * pi's extension events in, AgentEvent frames out.
 *
 * The extension this worker installs (`src/extension.ts`) posts one flat
 * event per pi lifecycle callback, and this is where a terminal turn becomes
 * the same wire shape a headless agent worker emits: a user message, an
 * assistant message per tool call, a `function_execution_start`/`end` pair
 * with a real duration, and `turn_end`/`agent_end` when the run finishes. The
 * console then renders this terminal's turns like any other agent's.
 *
 * The function itself is plumbing — one call per event — so it is
 * `trace_hidden`: the signal is the stream, not the delivery.
 */

import { randomUUID } from 'node:crypto';
import type { IIIClient } from 'iii-sdk';
import type { Emit } from '../events.js';
import { toolFunctionId } from '../map.js';
import { runTurnSpan } from '../trace.js';
import type {
  AgentMessage,
  AssistantMessage,
  ContentBlock,
  FunctionResultMessage,
  PiEvent,
} from '../types.js';

const IDLE_MS = 60 * 60_000;

type ToolCall = { function_id: string; started_at: number };

type SessionState = {
  transcript: AgentMessage[];
  calls: Map<string, ToolCall>;
  results: FunctionResultMessage[];
  touched: number;
  /**
   * The run every event of this prompt belongs to, and what the prompt said.
   * One pi run arrives as several separate calls — one per extension event —
   * so the turn identity cannot come from the call; it is opened at
   * `agent_start` and reused until the next one.
   */
  turnId: string;
  prompt: string;
  /**
   * Ids this worker invented, by tool name. pi's `call_id` is optional, and
   * both halves of a call have to carry the SAME id — otherwise the console
   * shows a call that never ends, and its duration reads as 0. An id derived
   * from the pending-call count cannot do that, because the count moves
   * between the two events, so the id is generated once at `tool_start` and
   * kept here until `tool_end` claims it.
   */
  generated: Map<string, string>;
  /** Only ever grows, so two calls to one tool never share an id. */
  nextId: number;
};

/** The id for a `tool_start` that named none, remembered for its `tool_end`. */
function startGeneratedId(state: SessionState, tool: string): string {
  const id = `${tool}-${state.nextId}`;
  state.nextId += 1;
  state.generated.set(tool, id);
  return id;
}

/**
 * The id its `tool_start` invented. A `tool_end` with no start behind it (a
 * restart mid-call, an extension that dropped a frame) still gets an id of its
 * own rather than one belonging to another call.
 */
function endGeneratedId(state: SessionState, tool: string): string {
  const id = state.generated.get(tool);
  if (id === undefined) return startGeneratedId(state, tool);
  state.generated.delete(tool);
  return id;
}

function assistant(content: ContentBlock[], stop_reason = 'tool_use'): AssistantMessage {
  return {
    role: 'assistant',
    content,
    stop_reason,
    error_message: null,
    usage: null,
    model: 'pi',
    provider: 'pi',
    timestamp: Date.now(),
  };
}

function resultContent(result: unknown): ContentBlock[] {
  if (result == null) return [];
  if (typeof result === 'string') return [{ type: 'text', text: result }];
  if (typeof result === 'object') {
    const blocks = (result as { content?: unknown }).content;
    if (Array.isArray(blocks)) {
      const text = blocks
        .map((block) =>
          typeof block === 'object' &&
          block !== null &&
          typeof (block as { text?: unknown }).text === 'string'
            ? (block as { text: string }).text
            : '',
        )
        .filter(Boolean)
        .join('\n');
      if (text) return [{ type: 'text', text }];
    }
  }
  return [{ type: 'text', text: JSON.stringify(result) }];
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
        generated: new Map(),
        nextId: 0,
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
   * One extension event, traced as part of its run. The identity is the same
   * set of keys a harness turn stamps, so a pi session's spans group and label
   * themselves in the console's trace views without the console knowing what a
   * pi extension event is.
   */
  async handle(event: PiEvent): Promise<{ ok: true; event: string }> {
    const name = event.event ?? 'unknown';
    const sessionId = event.session_id || 'pi';
    const state = this.state(sessionId);
    if (name === 'agent_start') {
      state.turnId = randomUUID();
      state.prompt = event.prompt ?? '';
    }
    return runTurnSpan(
      `pi ${name}`,
      {
        sessionId,
        turnId: state.turnId,
        kind: 'pi.terminal.turn',
        message: state.prompt,
        displayName: state.prompt ? `pi terminal · ${state.prompt}` : 'pi terminal',
      },
      () => this.apply(name, event, sessionId, state),
    );
  }

  private async apply(
    name: string,
    event: PiEvent,
    sessionId: string,
    state: SessionState,
  ): Promise<{ ok: true; event: string }> {
    switch (name) {
      case 'agent_start': {
        const message: AgentMessage = {
          role: 'user',
          content: [{ type: 'text', text: event.prompt ?? '' }],
          timestamp: Date.now(),
        };
        state.transcript.push(message);
        state.results = [];
        await this.emit(sessionId, { type: 'message_complete', message });
        break;
      }

      case 'tool_start': {
        const id = event.call_id || startGeneratedId(state, event.tool ?? 'tool');
        const function_id = toolFunctionId(event.tool ?? '');
        state.calls.set(id, { function_id, started_at: Date.now() });
        const message = assistant([
          { type: 'function_call', id, function_id, arguments: event.args ?? {} },
        ]);
        state.transcript.push(message);
        await this.emit(sessionId, { type: 'message_complete', message });
        await this.emit(sessionId, {
          type: 'function_execution_start',
          function_call_id: id,
          function_id,
          args: event.args ?? {},
        });
        break;
      }

      case 'tool_end': {
        const id = event.call_id || endGeneratedId(state, event.tool ?? 'tool');
        const call = state.calls.get(id);
        state.calls.delete(id);
        const function_id = call?.function_id ?? toolFunctionId(event.tool ?? '');
        const content = resultContent(event.result);
        const failed = event.is_error === true;
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
        await this.emit(sessionId, {
          type: 'function_execution_end',
          function_call_id: id,
          function_id,
          result: { content, details: null },
          is_error: failed,
          duration_ms: call ? Date.now() - call.started_at : 0,
        });
        break;
      }

      case 'agent_end': {
        const message = assistant([], 'end');
        state.transcript.push(message);
        await this.emit(sessionId, { type: 'turn_end', message, function_results: state.results });
        await this.emit(sessionId, { type: 'agent_end', messages: state.transcript });
        state.results = [];
        break;
      }

      case 'session_end':
        this.sessions.delete(sessionId);
        break;

      default:
        // session_start, and anything a newer pi adds: the session bucket now
        // exists, which is all an unmapped event has to do.
        break;
    }

    return { ok: true, event: name };
  }
}

export function registerActivity(iii: IIIClient, emit: Emit): ActivityTracker {
  const tracker = new ActivityTracker(emit);
  iii.registerFunction('pi::terminal::activity', (input: PiEvent) => tracker.handle(input ?? {}), {
    description:
      'A pi lifecycle event from a terminal session (session_start, agent_start, tool_start, tool_end, agent_end, session_end), posted by the workspace extension. Translated into AgentEvent frames on the events stream.',
    request_format: {
      type: 'object',
      properties: {
        event: { type: 'string' },
        session_id: { type: 'string' },
        cwd: { type: 'string' },
        prompt: { type: 'string' },
        tool: { type: 'string' },
        call_id: { type: 'string' },
        args: { type: 'object' },
        result: {},
        is_error: { type: 'boolean' },
      },
    },
    response_format: {
      type: 'object',
      required: ['ok', 'event'],
      properties: { ok: { type: 'boolean' }, event: { type: 'string' } },
    },
    metadata: { internal: true, trace_hidden: true },
  });
  return tracker;
}
