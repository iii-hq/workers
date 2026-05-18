/**
 * Env-driven config knobs. Mirrors
 * `context-compaction/src/config.rs`.
 */

const DEFAULT_TRIGGER_TOKENS = 60_000;
const DEFAULT_KEEP_RECENT_TURNS = 3;

export function triggerTokens(): number {
  const v = process.env.COMPACT_TRIGGER_TOKENS;
  if (!v) return DEFAULT_TRIGGER_TOKENS;
  const n = Number.parseInt(v, 10);
  return Number.isFinite(n) && n > 0 ? n : DEFAULT_TRIGGER_TOKENS;
}

export function keepRecentTurns(): number {
  const v = process.env.COMPACT_KEEP_RECENT_TURNS;
  if (!v) return DEFAULT_KEEP_RECENT_TURNS;
  const n = Number.parseInt(v, 10);
  return Number.isFinite(n) && n > 0 ? n : DEFAULT_KEEP_RECENT_TURNS;
}

export function summarizerProvider(): string | undefined {
  return process.env.COMPACT_SUMMARIZER_PROVIDER || undefined;
}

export function summarizerModel(): string | undefined {
  return process.env.COMPACT_SUMMARIZER_MODEL || undefined;
}
