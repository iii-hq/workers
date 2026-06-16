/** Scripted Agent SDK `query()` replacement: yields a fixed message list and
 *  records the prompt/options it was called with plus interrupt() calls. */

export type QueryCapture = {
  prompt?: string;
  options?: Record<string, unknown>;
  interrupted: boolean;
};

export function scriptedQuery(messages: Array<Record<string, unknown>>, capture: QueryCapture) {
  return (args: { prompt: string; options: Record<string, unknown> }) => {
    capture.prompt = args.prompt;
    capture.options = args.options;
    return {
      async *[Symbol.asyncIterator]() {
        yield* messages;
      },
      interrupt: async () => {
        capture.interrupted = true;
      },
    };
  };
}

export const initMsg = { type: 'system', subtype: 'init', session_id: 'cs-1' };

export const assistantMsg = {
  type: 'assistant',
  message: {
    model: 'claude-opus-4-8',
    usage: { input_tokens: 5, output_tokens: 2 },
    content: [
      { type: 'text', text: 'running ls' },
      { type: 'tool_use', id: 'toolu_1', name: 'Bash', input: { command: 'ls' } },
    ],
  },
};

export const toolResultMsg = {
  type: 'user',
  message: {
    content: [{ type: 'tool_result', tool_use_id: 'toolu_1', content: 'files', is_error: false }],
  },
};

export const resultMsg = {
  type: 'result',
  subtype: 'success',
  session_id: 'cs-1',
  num_turns: 2,
  total_cost_usd: 0.01,
  usage: { input_tokens: 5, output_tokens: 2 },
  result: 'done',
  is_error: false,
};

export const fullTurn = [initMsg, assistantMsg, toolResultMsg, resultMsg];
