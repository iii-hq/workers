// LM Studio speaks OpenAI's Chat Completions wire format. Kept separate from
// provider-openai / provider-kimi so any LM Studio-specific extensions (e.g.
// `keep_alive`, vision parts for multimodal GGUFs) can land without coupling
// the providers.

import { logger } from '../runtime/otel.js';
import type { AgentMessage } from '../types/agent-message.js';
import { formatFunctionResultContent } from '../types/wire.js';

/**
 * Some local model templates (notably qwen3's jinja) require at least one
 * `role: 'user'` message and abort with "No user query found in messages"
 * otherwise. That condition is reachable in normal agentic operation:
 * after a tool call cycle whose ancestors got summarised away by async
 * compaction, the flat-state can be [assistant, tool, assistant, tool, …]
 * with the original user message gone. Without this safety net we'd send
 * the request, LM Studio would error mid-stream, and the user would see
 * a cryptic "stream closed mid-response" instead of a clean continuation.
 *
 * The placeholder is intentionally minimal ("(continue)") so the model
 * understands it as a continuation directive — matches what LiteLLM /
 * OpenRouter do for the same template constraint.
 */
export const PLACEHOLDER_USER_MESSAGE = '(continue)';

function hasUserMessage(wire: readonly unknown[]): boolean {
  for (const m of wire) {
    if (m && typeof m === 'object' && (m as { role?: unknown }).role === 'user') {
      return true;
    }
  }
  return false;
}

export function toOpenaiMessages(messages: AgentMessage[], system_prompt: string): unknown[] {
  const out: unknown[] = [];
  // Index of `tool_call_id → out[]` slot for O(1) latest-wins dedup
  // of function_result rows. Pre-fix this was an `out.findIndex` scan
  // per function_result, which gave O(M²) translation cost when the
  // history carried many tool calls — amplified on LM Studio because
  // the auto-load retry can re-translate the same history twice in
  // one user turn.
  const toolResultIndexById = new Map<string, number>();
  if (system_prompt.length > 0) {
    out.push({ role: 'system', content: system_prompt });
  }
  for (const m of messages) {
    if (m.role === 'user') {
      const text = m.content
        .filter(
          (c): c is Extract<(typeof m.content)[number], { type: 'text' }> => c.type === 'text',
        )
        .map((c) => c.text)
        .join('\n');
      out.push({ role: 'user', content: text });
    } else if (m.role === 'assistant') {
      const text = m.content
        .filter(
          (c): c is Extract<(typeof m.content)[number], { type: 'text' }> => c.type === 'text',
        )
        .map((c) => c.text)
        .join('\n');
      // Thinking-mode models served by LM Studio (qwen3, glm, deepseek-r1)
      // require `reasoning_content` echoed back on assistant tool-call
      // messages — same constraint as Kimi K2. sse.ts captures the
      // stream into a ThinkingContent block; project it back here.
      const reasoning = m.content
        .filter(
          (c): c is Extract<(typeof m.content)[number], { type: 'thinking' }> =>
            c.type === 'thinking',
        )
        .map((c) => c.text)
        .join('');
      const tool_calls = m.content
        .filter(
          (c): c is Extract<(typeof m.content)[number], { type: 'function_call' }> =>
            c.type === 'function_call',
        )
        .map((c) => ({
          id: c.id,
          type: 'function',
          function: { name: c.function_id, arguments: JSON.stringify(c.arguments) },
        }));
      const entry: Record<string, unknown> = { role: 'assistant' };
      if (reasoning.length > 0) entry.reasoning_content = reasoning;
      if (text.length > 0) entry.content = text;
      if (tool_calls.length > 0) entry.tool_calls = tool_calls;
      out.push(entry);
    } else if (m.role === 'function_result') {
      const text = formatFunctionResultContent(m);
      const row: Record<string, unknown> = {
        role: 'tool',
        tool_call_id: m.function_call_id,
        content: text,
      };
      if (m.is_error) row.is_error = true;
      // Boundary dedup — see provider-anthropic/wire-messages.ts for why.
      // LM Studio's jinja templates vary by GGUF; some accept duplicates,
      // some don't. Latest-wins replace keeps every template happy.
      // O(1) lookup via the index map (see top-of-function note on
      // why we don't scan `out` linearly here).
      const existingIdx = toolResultIndexById.get(m.function_call_id);
      if (existingIdx !== undefined) {
        out[existingIdx] = row;
      } else {
        toolResultIndexById.set(m.function_call_id, out.length);
        out.push(row);
      }
    }
    // custom messages are skipped
  }
  if (!hasUserMessage(out)) {
    logger.warn(
      'lmstudio: no user-role message in request; injecting placeholder to satisfy strict jinja templates (e.g. qwen3 "No user query found")',
      { messageCount: out.length },
    );
    out.push({ role: 'user', content: PLACEHOLDER_USER_MESSAGE });
  }
  return out;
}
