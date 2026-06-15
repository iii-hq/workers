/**
 * One-time boot migration: copy the `harness` entry's `providers` block into
 * the `llm-router` configuration entry (now the single home for provider
 * credentials + settings), and seed routing parity with the legacy local
 * `decide()` — anthropic as the default provider plus the gpt-/o<digit>- and
 * kimi-/moonshot-v1- prefix heuristics.
 *
 * Idempotent: a marker in iii-state (scope `harness-migrations`) skips the
 * work on later boots, and an `llm-router` entry that already has providers
 * is treated as migrated (the operator got there first). The copy reads the
 * `harness` entry raw so `${VAR:default}` templates survive verbatim. The set
 * is retried briefly because the router composes the entry schema only after
 * the first provider registers.
 *
 * The stale `providers` block in the `harness` entry is left in place; the
 * console edits the `llm-router` entry from now on.
 */

import { configurationGet, configurationSet, type JsonValue } from '../runtime/configuration.js';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { stateGet, stateSet } from '../runtime/state.js';

export const LLM_ROUTER_CONFIG_ID = 'llm-router';

const MIGRATION_SCOPE = 'harness-migrations';
const MIGRATION_KEY = 'llm-router-providers';

const SET_ATTEMPTS = 10;
const SET_RETRY_MS = 3_000;

/**
 * Routing parity with the legacy `decide()` (turn-orchestrator
 * provider-router): unmatched models fell back to anthropic, gpt-/o<digit>-
 * routed to openai, kimi-/moonshot-v1- to kimi. Rust regex: inline `(?i)`,
 * no `/i` flag.
 */
export const ROUTING_PARITY_SEED = {
  default_provider: 'anthropic',
  routing_heuristics: [
    { pattern: '(?i)^(gpt-|o[0-9]-)', provider: 'openai' },
    { pattern: '(?i)^(kimi-|moonshot-v1-)', provider: 'kimi' },
  ],
} as const;

function asObject(v: JsonValue | null): Record<string, JsonValue> {
  return v && typeof v === 'object' && !Array.isArray(v) ? { ...v } : {};
}

function providersOf(v: JsonValue | null): Record<string, JsonValue> {
  const providers = asObject(v).providers;
  return asObject(providers ?? null);
}

/** Compose the migrated `llm-router` value without clobbering operator edits. */
export function composeMigratedValue(
  routerValue: JsonValue | null,
  harnessProviders: Record<string, JsonValue>,
): Record<string, JsonValue> {
  const out = asObject(routerValue);
  out.providers = { ...harnessProviders, ...providersOf(routerValue) };
  if (typeof out.default_provider !== 'string') {
    out.default_provider = ROUTING_PARITY_SEED.default_provider;
  }
  if (!Array.isArray(out.routing_heuristics)) {
    out.routing_heuristics = ROUTING_PARITY_SEED.routing_heuristics.map((h) => ({ ...h }));
  }
  return out;
}

export async function migrateLlmRouterConfig(iii: ISdk): Promise<void> {
  const marker = await stateGet<{ migrated_at?: string }>(iii, MIGRATION_SCOPE, MIGRATION_KEY);
  if (marker) return;

  const routerValue = await configurationGet(iii, LLM_ROUTER_CONFIG_ID, { raw: true });
  if (Object.keys(providersOf(routerValue)).length > 0) {
    // Operator already populated the entry — record that and never touch it again.
    await stateSet(iii, MIGRATION_SCOPE, MIGRATION_KEY, {
      migrated_at: new Date().toISOString(),
      note: 'llm-router entry already populated',
    });
    return;
  }

  // Raw read preserves `${VAR:default}` credential templates verbatim.
  const harnessValue = await configurationGet(iii, 'harness', { raw: true });
  const migrated = composeMigratedValue(routerValue, providersOf(harnessValue));

  for (let attempt = 1; attempt <= SET_ATTEMPTS; attempt++) {
    try {
      await configurationSet(iii, LLM_ROUTER_CONFIG_ID, migrated);
      await stateSet(iii, MIGRATION_SCOPE, MIGRATION_KEY, {
        migrated_at: new Date().toISOString(),
        providers: Object.keys(providersOf(migrated)),
      });
      logger.info('migrated provider config to the llm-router entry', {
        providers: Object.keys(providersOf(migrated)),
      });
      return;
    } catch (err) {
      // The router registers the entry (and composes its schema) only after
      // the first provider registration lands — early boots race that.
      logger.warn('llm-router config migration set failed; retrying', {
        attempt,
        err: String(err),
      });
      await new Promise((resolve) => setTimeout(resolve, SET_RETRY_MS));
    }
  }
  // No marker written: the next boot retries the whole migration.
  logger.error('llm-router config migration did not land; will retry next boot', {});
}
