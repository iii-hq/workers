/**
 * Compaction execution: load active path, split keep-recent / older-prefix,
 * summarise via the in-process channel pump, append a `Compaction` entry
 * to the session-tree. Mirrors `context-compaction/src/summarize.rs`.
 */

import { readFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { AgentMessage } from '../types/agent-message.js';
import { keepRecentTurns, summarizerModel, summarizerProvider } from './config.js';
import { stampLastCompaction } from './lease.js';
import { streamAndCollect } from './stream-collect.js';

const __dirname = dirname(fileURLToPath(import.meta.url));

let cachedPrompt: string | null = null;
async function compactionPrompt(): Promise<string> {
  if (cachedPrompt !== null) return cachedPrompt;
  cachedPrompt = await readFile(join(__dirname, 'prompts', 'compaction.txt'), 'utf8');
  return cachedPrompt;
}

export function splitMessages(
  messages: AgentMessage[],
  keep_recent: number,
): { older: AgentMessage[]; recent: AgentMessage[] } {
  if (messages.length <= keep_recent) {
    return { older: [], recent: messages.slice() };
  }
  const split = messages.length - keep_recent;
  return { older: messages.slice(0, split), recent: messages.slice(split) };
}

export function estimateTokenCount(messages: AgentMessage[]): number {
  let chars = 0;
  for (const m of messages) chars += JSON.stringify(m).length;
  return Math.floor(chars / 4);
}

export function renderUserPrompt(older: AgentMessage[]): string {
  let out =
    'Summarise the conversation below following the system-prompt structure exactly. Keep identifiers verbatim.\n\n';
  out += '<conversation>\n';
  for (const msg of older) {
    out += `\n[${msg.role}]\n`;
    if (Array.isArray(msg.content)) {
      for (const block of msg.content) {
        if (block.type === 'text') out += `${block.text}\n`;
        else if (block.type === 'function_call')
          out += `[tool_call] ${block.function_id} ${JSON.stringify(block.arguments)}\n`;
      }
    }
  }
  out += '</conversation>\n';
  return out;
}

async function loadActiveMessages(iii: ISdk, session_id: string): Promise<AgentMessage[]> {
  const resp = await iii.trigger<unknown, { messages?: Array<{ message?: AgentMessage }> }>({
    function_id: 'session-tree::messages',
    payload: { session_id },
    timeoutMs: 10_000,
  });
  const arr = resp?.messages;
  if (!Array.isArray(arr)) throw new Error('session-tree::messages returned non-array');
  return arr.map((entry) => entry?.message).filter((m): m is AgentMessage => Boolean(m));
}

async function appendCompaction(
  iii: ISdk,
  session_id: string,
  summary: string,
  tokens_before: number,
): Promise<void> {
  await iii.trigger<unknown, unknown>({
    function_id: 'session-tree::compact',
    payload: {
      session_id,
      summary,
      tokens_before,
      details: { read_files: [], modified_files: [] },
    },
    timeoutMs: 10_000,
  });
  await stampLastCompaction(iii, session_id);
}

export async function summarizeAndAppend(iii: ISdk, session_id: string): Promise<void> {
  const messages = await loadActiveMessages(iii, session_id);
  const keep = keepRecentTurns();
  const { older } = splitMessages(messages, keep);
  if (older.length === 0) return;
  const tokens_before = estimateTokenCount(older);

  const provider = summarizerProvider() ?? 'anthropic';
  const model = summarizerModel() ?? 'claude-haiku-4-5';
  const prompt = await compactionPrompt();
  const userPrompt = renderUserPrompt(older);

  const final = await streamAndCollect(
    iii,
    {
      system_prompt: prompt,
      model,
      messages: [
        {
          role: 'user',
          content: [{ type: 'text', text: userPrompt }],
          timestamp: Date.now(),
        },
      ],
      tools: [],
    },
    provider === 'openai' ? 'provider::openai::stream' : 'provider::anthropic::stream',
  );

  let summary = '';
  for (const block of final.content) {
    if (block.type === 'text') summary += summary.length > 0 ? `\n${block.text}` : block.text;
  }
  if (summary.length === 0) {
    logger.warn('summariser produced empty summary; skipping append', { session_id });
    return;
  }
  await appendCompaction(iii, session_id, summary, tokens_before);
}
