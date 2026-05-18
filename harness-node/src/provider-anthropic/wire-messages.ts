/**
 * AgentMessage → Anthropic wire shape. Mirrors
 * `provider-anthropic/src/lib.rs::{to_wire_messages, content_block_to_wire,
 * encode_tool_name, decode_tool_name}`.
 */

import type { AgentMessage } from '../types/agent-message.js';
import type { ContentBlock } from '../types/content.js';
import { formatFunctionResultContent } from '../types/wire.js';

/**
 * Anthropic's tool-name regex is `^[a-zA-Z0-9_-]{1,128}$`; bus ids use
 * `::` separators. We replace `::` with `__` on the way out and reverse
 * on the way back. (Tool names that already contain `__` are not in use
 * today — see the Rust port for the same caveat.)
 */
export function encodeToolName(name: string): string {
  return name.replaceAll('::', '__');
}

export function decodeToolName(name: string): string {
  return name.replaceAll('__', '::');
}

export function contentBlockToWire(b: ContentBlock): unknown | null {
  if (b.type === 'text') return { type: 'text', text: b.text };
  if (b.type === 'function_call') {
    return {
      type: 'tool_use',
      id: b.id,
      name: encodeToolName(b.function_id),
      input: b.arguments,
    };
  }
  return null;
}

export function toWireMessages(messages: AgentMessage[]): unknown[] {
  const out: unknown[] = [];
  let pending: unknown[] = [];
  const flush = () => {
    if (pending.length > 0) {
      out.push({ role: 'user', content: pending });
      pending = [];
    }
  };
  for (const m of messages) {
    if (m.role === 'user') {
      flush();
      const content = m.content.map(contentBlockToWire).filter((v): v is unknown => v !== null);
      out.push({ role: 'user', content });
    } else if (m.role === 'assistant') {
      flush();
      const content = m.content.map(contentBlockToWire).filter((v): v is unknown => v !== null);
      out.push({ role: 'assistant', content });
    } else if (m.role === 'function_result') {
      pending.push({
        type: 'tool_result',
        tool_use_id: m.function_call_id,
        content: formatFunctionResultContent(m),
        is_error: m.is_error,
      });
    }
    // custom messages are skipped
  }
  flush();
  return out;
}
