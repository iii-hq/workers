const DEFAULT_RESERVED_TOKENS = 20_000;
const DEFAULT_TAIL_TURNS = 2;
const DEFAULT_MIN_PRESERVE_RECENT_TOKENS = 2_000;
const DEFAULT_MAX_PRESERVE_RECENT_TOKENS = 8_000;
const DEFAULT_PRUNE_PROTECT = 40_000;
const DEFAULT_PRUNE_MIN_FREE = 20_000;
const DEFAULT_TOOL_OUTPUT_MAX_CHARS = 2_000;
// Sized to cover a summariser stream (10-30s); shorter values surface
// `busy` to users when async compaction is mid-flight.
const DEFAULT_BUSY_TIMEOUT_MS = 30_000;

function intEnv(name: string, def: number): number {
  const v = process.env[name];
  if (!v) return def;
  const n = Number.parseInt(v, 10);
  return Number.isFinite(n) && n > 0 ? n : def;
}

export function reservedTokens(): number {
  return intEnv('COMPACT_RESERVED_TOKENS', DEFAULT_RESERVED_TOKENS);
}

export function tailTurns(): number {
  return intEnv('COMPACT_TAIL_TURNS', DEFAULT_TAIL_TURNS);
}

export function preserveRecentTokensOverride(): number | undefined {
  const v = process.env.COMPACT_PRESERVE_RECENT_TOKENS;
  if (!v) return undefined;
  const n = Number.parseInt(v, 10);
  return Number.isFinite(n) && n > 0 ? n : undefined;
}

export const MIN_PRESERVE_RECENT_TOKENS = DEFAULT_MIN_PRESERVE_RECENT_TOKENS;
export const MAX_PRESERVE_RECENT_TOKENS = DEFAULT_MAX_PRESERVE_RECENT_TOKENS;

export function pruneProtect(): number {
  return intEnv('COMPACT_PRUNE_PROTECT', DEFAULT_PRUNE_PROTECT);
}

export function pruneMinFree(): number {
  return intEnv('COMPACT_PRUNE_MIN_FREE', DEFAULT_PRUNE_MIN_FREE);
}

export function toolOutputMaxChars(): number {
  return intEnv('COMPACT_TOOL_OUTPUT_MAX_CHARS', DEFAULT_TOOL_OUTPUT_MAX_CHARS);
}

export function busyTimeoutMs(): number {
  return intEnv('COMPACT_BUSY_TIMEOUT_MS', DEFAULT_BUSY_TIMEOUT_MS);
}

export function pruneProtectedTools(): string[] {
  const v = process.env.COMPACT_PRUNE_PROTECTED_TOOLS;
  if (!v) return [];
  return v
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);
}
