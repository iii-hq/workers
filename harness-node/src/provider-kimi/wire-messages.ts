// Kept separate from provider-openai so Moonshot-specific extensions
// can land without coupling the two providers.

import type { AgentMessage } from '../types/agent-message.js';
import { formatFunctionResultContent } from '../types/wire.js';

export function toOpenaiMessages(messages: AgentMessage[], system_prompt: string): unknown[] {
  const out: unknown[] = [];
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
      out.push(row);
    }
    // custom messages are skipped
  }
  return out;
}
