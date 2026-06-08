/**
 * Provider-wire helpers. Port of
 * `harness/crates/harness-types/src/wire.rs`.
 *
 * `formatFunctionResultContent` produces the text body that providers
 * embed into a tool-result block. When the tool's `details.status` is
 * `"denied"`, the body is prefixed with a `[PERMISSION_DENIED]` marker
 * and a single-line JSON envelope so the LLM can parse the structured
 * denial alongside the human-readable reason.
 */

import type { FunctionResultMessage } from './agent-message.js';
import type { ContentBlock } from './content.js';

export function formatFunctionResultContent(msg: FunctionResultMessage): string {
  const body = msg.content
    .filter((c): c is Extract<ContentBlock, { type: 'text' }> => c.type === 'text')
    .map((c) => c.text)
    .join('\n');
  const status =
    typeof msg.details === 'object' && msg.details !== null
      ? (msg.details as Record<string, unknown>).status
      : undefined;
  if (status === 'denied') {
    let envelope: string;
    try {
      envelope = JSON.stringify(msg.details);
    } catch {
      envelope = '{}';
    }
    return `[PERMISSION_DENIED]\n${envelope}\n\n${body}`;
  }
  return body;
}

export type WireResultBlock =
  | { type: 'text'; text: string }
  | { type: 'image'; mime: string; data: string };

/**
 * Block-preserving variant of `formatFunctionResultContent` for providers
 * whose tool-result content accepts structured blocks (Anthropic). The
 * text body is built exactly as the flat-string path (including the
 * `[PERMISSION_DENIED]` envelope), followed by any image blocks in their
 * original order. Text-only providers keep using the flat string.
 */
export function formatFunctionResultBlocks(msg: FunctionResultMessage): WireResultBlock[] {
  const blocks: WireResultBlock[] = [];
  const body = formatFunctionResultContent(msg);
  if (body.length > 0) blocks.push({ type: 'text', text: body });
  for (const c of msg.content) {
    if (c.type === 'image') blocks.push({ type: 'image', mime: c.mime, data: c.data });
  }
  return blocks;
}
