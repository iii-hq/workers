/**
 * Anthropic prompt-cache markers. Mirrors
 * `provider-anthropic/src/lib.rs::{build_system_field,
 * apply_tools_cache_control, apply_messages_cache_anchor}`.
 */

const CACHE_MIN_CHARS = 4096;
const CACHE_FLAG_ENV = 'HARNESS_ANTHROPIC_CACHE';

let cacheEnabledCached: boolean | null = null;
function cacheEnabled(): boolean {
  if (cacheEnabledCached !== null) return cacheEnabledCached;
  const v = process.env[CACHE_FLAG_ENV];
  cacheEnabledCached = v === undefined || !['0', 'false', 'FALSE', 'False'].includes(v);
  return cacheEnabledCached;
}

const ephemeral = () => ({ type: 'ephemeral' });

export function buildSystemField(prompt: string): unknown {
  if (cacheEnabled() && prompt.length >= CACHE_MIN_CHARS) {
    return [{ type: 'text', text: prompt, cache_control: ephemeral() }];
  }
  return prompt;
}

export function applyToolsCacheControl(tools: Record<string, unknown>[]): void {
  if (!cacheEnabled() || tools.length === 0) return;
  const size = tools.reduce((acc, t) => acc + JSON.stringify(t).length, 0);
  if (size < CACHE_MIN_CHARS) return;
  const last = tools[tools.length - 1];
  if (last) last.cache_control = ephemeral();
}

export function applyMessagesCacheAnchor(wire: Record<string, unknown>[]): void {
  if (!cacheEnabled() || wire.length === 0) return;
  let lastStable = -1;
  for (let i = wire.length - 1; i >= 0; i--) {
    if (isStableAssistant(wire, i)) {
      lastStable = i;
      break;
    }
  }
  if (lastStable < 0) return;
  const msg = wire[lastStable];
  if (!msg) return;
  const content = msg.content;
  if (!Array.isArray(content) || content.length === 0) return;
  const lastBlock = content[content.length - 1] as Record<string, unknown> | undefined;
  if (lastBlock) lastBlock.cache_control = ephemeral();
}

function isStableAssistant(wire: Record<string, unknown>[], idx: number): boolean {
  const msg = wire[idx];
  if (!msg) return false;
  if (msg.role !== 'assistant') return false;
  const content = msg.content;
  if (!Array.isArray(content)) return true;
  const toolUseIds = content
    .filter(
      (b): b is { id: string } =>
        Boolean(b) &&
        typeof b === 'object' &&
        (b as Record<string, unknown>).type === 'tool_use' &&
        typeof (b as Record<string, unknown>).id === 'string',
    )
    .map((b) => b.id);
  if (toolUseIds.length === 0) return true;
  return toolUseIds.every((id) => hasDownstreamToolResult(wire.slice(idx + 1), id));
}

function hasDownstreamToolResult(later: Record<string, unknown>[], id: string): boolean {
  return later.some((m) => {
    if (m.role !== 'user') return false;
    const content = m.content;
    if (!Array.isArray(content)) return false;
    return content.some(
      (b) =>
        b &&
        typeof b === 'object' &&
        (b as Record<string, unknown>).type === 'tool_result' &&
        (b as Record<string, unknown>).tool_use_id === id,
    );
  });
}
