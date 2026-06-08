/**
 * AgentMessage → Anthropic wire shape. Mirrors
 * `provider-anthropic/src/lib.rs::{to_wire_messages, content_block_to_wire,
 * encode_tool_name, decode_tool_name}`.
 */

import { logger } from '../runtime/otel.js';
import type { AgentMessage } from '../types/agent-message.js';
import type { ContentBlock } from '../types/content.js';
import { formatFunctionResultBlocks, formatFunctionResultContent } from '../types/wire.js';

/**
 * Content shipped in the synthetic `tool_result` placeholder we inject
 * when an assistant turn has a `tool_use` block with no matching
 * `function_result` AgentMessage anywhere in the conversation. Anthropic
 * rejects orphan tool_uses with: "tool_use IDs were found without
 * tool_result blocks immediately after". This message lets the model
 * understand the call was interrupted and move on.
 */
const ORPHAN_TOOL_PLACEHOLDER =
  'Tool call was interrupted before completing. Continue without its output.';

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
  if (b.type === 'thinking') {
    // During tool use Anthropic requires signed thinking blocks passed back
    // unmodified (400 otherwise); unsigned blocks (aborted/partial stream)
    // would fail signature verification and are dropped instead.
    if (b.signature) return { type: 'thinking', thinking: b.text, signature: b.signature };
    return null;
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

  // Boundary sanitization for orphan tool_uses. Pre-pass: collect every
  // function_call_id that has a matching function_result AgentMessage.
  // After emitting an assistant turn, any tool_use block whose id is NOT
  // in this set gets a synthetic tool_result placeholder injected into
  // the next user message — preventing Anthropic's
  //   "tool_use IDs were found without tool_result blocks immediately after"
  // error. Investigation traced three orchestrator paths that can leave an
  // orphan: abort during awaiting_approval, sync /compact mid-execution,
  // and prepared-call desync. Defending at the wire layer covers all of
  // them without coupling the fix to a specific state-machine edge.
  const resolvedIds = new Set<string>();
  for (const m of messages) {
    if (m.role === 'function_result') resolvedIds.add(m.function_call_id);
  }

  for (const m of messages) {
    if (m.role === 'user') {
      // Merge any pending tool_results (from prior function_result
      // AgentMessages OR synthetic placeholders for orphan tool_uses)
      // INTO this user message's content. Anthropic allows tool_result
      // blocks and regular content in the same user message — and it
      // forbids consecutive user messages, which a separate `flush()`
      // would create.
      const userContent = m.content.map(contentBlockToWire).filter((v): v is unknown => v !== null);
      const content = [...pending, ...userContent];
      pending = [];
      out.push({ role: 'user', content });
    } else if (m.role === 'assistant') {
      flush();
      const content = m.content.map(contentBlockToWire).filter((v): v is unknown => v !== null);
      out.push({ role: 'assistant', content });
      // Inject placeholders for any tool_use in this assistant that has
      // no matching function_result. They land in `pending` and get
      // flushed into the NEXT user message — exactly where Anthropic
      // expects to find them.
      for (const block of m.content) {
        if (block.type !== 'function_call') continue;
        if (resolvedIds.has(block.id)) continue;
        logger.warn(
          'provider-anthropic: tool_use lacks matching function_result; injecting synthetic placeholder',
          { tool_use_id: block.id, function_id: block.function_id },
        );
        pending.push({
          type: 'tool_result',
          tool_use_id: block.id,
          content: ORPHAN_TOOL_PLACEHOLDER,
          is_error: true,
        });
        // Mark resolved so a stray duplicate of the same orphan in a
        // pathological message list doesn't get a second placeholder.
        resolvedIds.add(block.id);
      }
    } else if (m.role === 'function_result') {
      // Boundary dedup: even with idempotency guards upstream, never let a
      // duplicate tool_result block reach Anthropic. The API rejects with
      //   "each tool_use must have a single result. Found multiple
      //    tool_result blocks with id: toolu_..."
      // and the whole turn fails. Latest-wins: replace any existing block
      // with the same tool_use_id in the current pending batch so the
      // most recent function_result is what the model sees.
      // Anthropic tool_result content accepts either a flat string or an
      // array of text/image blocks. Keep the flat string whenever there
      // are no images — that's the long-standing wire shape (and what
      // prompt caching has seen) — and only switch to the array form when
      // an image block must reach the model (e.g. web::fetch image mode).
      const resultBlocks = formatFunctionResultBlocks(m);
      const hasImages = resultBlocks.some((b) => b.type === 'image');
      const block = {
        type: 'tool_result',
        tool_use_id: m.function_call_id,
        content: hasImages
          ? resultBlocks.map((b) =>
              b.type === 'image'
                ? {
                    type: 'image',
                    source: { type: 'base64', media_type: b.mime, data: b.data },
                  }
                : { type: 'text', text: b.text },
            )
          : formatFunctionResultContent(m),
        is_error: m.is_error,
      };
      const existingIdx = pending.findIndex(
        (b) => (b as { tool_use_id?: string } | null)?.tool_use_id === m.function_call_id,
      );
      if (existingIdx >= 0) {
        pending[existingIdx] = block;
      } else {
        pending.push(block);
      }
    }
    // custom messages are skipped
  }
  flush();
  return out;
}
