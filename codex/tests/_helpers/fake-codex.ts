/** Scripted Codex SDK replacement: a fake Codex implementation whose threads
 *  yield a fixed event list and record the options and prompt they were
 *  driven with. Plain functions returning objects (not ES classes) so they
 *  slot into vi.fn().mockImplementation for both `new Codex()` and calls. */

export type CodexCapture = {
  codexOptions?: Record<string, unknown>;
  input?: unknown;
  threadOptions?: Record<string, unknown>;
  resumedFrom?: string;
  prompt?: string;
  turnOptions?: Record<string, unknown>;
  aborted: boolean;
};

export function fakeCodexClass(events: Array<Record<string, unknown>>, capture: CodexCapture) {
  return function FakeCodex(options: Record<string, unknown>) {
    capture.codexOptions = options;
    return {
      startThread(threadOptions: Record<string, unknown>) {
        capture.threadOptions = threadOptions;
        return makeThread(events, capture);
      },
      resumeThread(id: string, threadOptions: Record<string, unknown>) {
        capture.resumedFrom = id;
        capture.threadOptions = threadOptions;
        return makeThread(events, capture, id);
      },
    };
  };
}

function makeThread(
  events: Array<Record<string, unknown>>,
  capture: CodexCapture,
  id: string | null = null,
) {
  const threadIdFromScript = events.find((e) => e.type === 'thread.started') as
    | { thread_id?: string }
    | undefined;
  return {
    id: id ?? threadIdFromScript?.thread_id ?? null,
    runStreamed: async (prompt: string | unknown[], turnOptions?: Record<string, unknown>) => {
      capture.prompt = typeof prompt === 'string' ? prompt : undefined;
      capture.input = prompt;
      capture.turnOptions = turnOptions;
      const signal = turnOptions?.signal as AbortSignal | undefined;
      return {
        events: (async function* () {
          for (const event of events) {
            if (signal?.aborted) {
              capture.aborted = true;
              throw new Error('aborted');
            }
            yield event;
          }
        })(),
      };
    },
  };
}

export const threadStarted = { type: 'thread.started', thread_id: 'th-1' };
export const turnStarted = { type: 'turn.started' };

export const commandStarted = {
  type: 'item.started',
  item: { id: 'item-1', type: 'command_execution', command: 'ls', status: 'in_progress' },
};

export const commandCompleted = {
  type: 'item.completed',
  item: {
    id: 'item-1',
    type: 'command_execution',
    command: 'ls',
    aggregated_output: 'files',
    exit_code: 0,
    status: 'completed',
  },
};

export const agentMessage = {
  type: 'item.completed',
  item: { id: 'item-2', type: 'agent_message', text: 'done' },
};

export const turnCompleted = {
  type: 'turn.completed',
  usage: {
    input_tokens: 5,
    cached_input_tokens: 100,
    output_tokens: 2,
    reasoning_output_tokens: 7,
  },
};

export const fullTurn = [
  threadStarted,
  turnStarted,
  commandStarted,
  commandCompleted,
  agentMessage,
  turnCompleted,
];
