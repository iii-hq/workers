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
