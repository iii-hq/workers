import { readFileSync } from 'node:fs';
import { join } from 'node:path';
/** Scripted Agent SDK `query()` replacement: yields a fixed message list and
 *  records the prompt/options it was called with plus interrupt() calls. */

export type QueryCapture = {
  prompt?: string;
  options?: Record<string, unknown>;
  interrupted: boolean;
  /** The local plugin's files as they were when the turn started. */
  pluginFiles?: Record<string, string | null>;
};

const PLUGIN_PATHS = [
  '.claude-plugin/plugin.json',
  '.mcp.json',
  'hooks/hooks.json',
  'default-mcp.md',
  'agents/iii.md',
  'skills/iii-runtime/SKILL.md',
];

function snapshot(dir: string): Record<string, string | null> {
  const files: Record<string, string | null> = {};
  for (const path of PLUGIN_PATHS) {
    try {
      files[path] = readFileSync(join(dir, path), 'utf8');
    } catch {
      files[path] = null;
    }
  }
  return files;
}

export function scriptedQuery(messages: Array<Record<string, unknown>>, capture: QueryCapture) {
  return (args: { prompt: string; options: Record<string, unknown> }) => {
    capture.prompt = args.prompt;
    capture.options = args.options;
    const plugins = args.options.plugins as Array<{ path: string }> | undefined;
    if (plugins?.[0]?.path) capture.pluginFiles = snapshot(plugins[0].path);
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
