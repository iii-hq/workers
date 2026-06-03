/**
 * Shape of the single `harness` configuration entry that lives in the
 * built-in `configuration` worker. It replaces the former `database`-backed
 * `auth-credentials` (api keys) and `provider-config` (runtime overrides)
 * tables, and adds a `permissions` block.
 *
 * Value shape:
 *
 *   {
 *     "permissions": { "default_mode": "manual" | "auto" | "full" },
 *     "providers": {
 *       "anthropic": { "api_key": "...", "api_url": "...", "max_tokens": 8192 },
 *       ...
 *     }
 *   }
 *
 * The `providers` JSON Schema is composed dynamically from each provider's
 * self-declared schema (see `harness/providers/registry.ts`), so adding a
 * provider changes the editable shape automatically.
 */

import { configurationGet, type JsonValue } from './configuration.js';
import type { ISdk } from './iii.js';

/** The id of the harness configuration entry. */
export const HARNESS_CONFIG_ID = 'harness';

export const PERMISSION_MODES = ['manual', 'auto', 'full'] as const;
export type PermissionMode = (typeof PERMISSION_MODES)[number];

/** One provider's stored config. Secret (`api_key`) + non-secret settings. */
export type HarnessProviderConfig = {
  api_key?: string;
  api_url?: string;
  max_tokens?: number;
} & Record<string, unknown>;

export type HarnessPermissions = {
  default_mode: PermissionMode;
};

export type HarnessConfigValue = {
  permissions: HarnessPermissions;
  providers: Record<string, HarnessProviderConfig>;
};

export const DEFAULT_PERMISSION_MODE: PermissionMode = 'manual';

/** The value used to seed the entry the first time it is registered. */
export function baseHarnessConfigValue(): HarnessConfigValue {
  return {
    permissions: { default_mode: DEFAULT_PERMISSION_MODE },
    providers: {},
  };
}

function isPermissionMode(v: unknown): v is PermissionMode {
  return v === 'manual' || v === 'auto' || v === 'full';
}

/** Coerce an arbitrary JSON value into a well-formed `HarnessConfigValue`. */
export function normalizeHarnessConfig(value: JsonValue | null): HarnessConfigValue {
  const base = baseHarnessConfigValue();
  if (!value || typeof value !== 'object' || Array.isArray(value)) return base;
  const obj = value as Record<string, JsonValue>;

  const permissions = obj.permissions;
  if (permissions && typeof permissions === 'object' && !Array.isArray(permissions)) {
    const mode = (permissions as Record<string, JsonValue>).default_mode;
    if (isPermissionMode(mode)) base.permissions.default_mode = mode;
  }

  const providers = obj.providers;
  if (providers && typeof providers === 'object' && !Array.isArray(providers)) {
    for (const [id, cfg] of Object.entries(providers)) {
      if (cfg && typeof cfg === 'object' && !Array.isArray(cfg)) {
        base.providers[id] = cfg as HarnessProviderConfig;
      }
    }
  }
  return base;
}

/**
 * Read the `harness` entry value, env-expanded and normalized. Tolerant:
 * returns the base value when the entry is missing or the worker is down.
 */
export async function readHarnessConfig(iii: ISdk): Promise<HarnessConfigValue> {
  const value = await configurationGet(iii, HARNESS_CONFIG_ID, { raw: false });
  return normalizeHarnessConfig(value);
}

/** Fields that affect upstream model listing for a provider. */
const DISCOVERY_FINGERPRINT_KEYS = ['api_key', 'api_url'] as const;

function normalizeApiKey(v: unknown): string {
  return typeof v === 'string' ? v : '';
}

/**
 * Stable fingerprint of provider settings that should trigger model
 * re-discovery when changed.
 */
export function providerDiscoveryFingerprint(cfg: HarnessProviderConfig): string {
  const parts: string[] = [];
  for (const k of DISCOVERY_FINGERPRINT_KEYS) {
    parts.push(`${k}=${normalizeApiKey(cfg[k])}`);
  }
  return parts.join('\u0001');
}

/**
 * Provider ids whose discovery-relevant config changed between two harness
 * snapshots. Permissions-only edits return [].
 */
export function providersAffectedByConfigChange(
  oldCfg: HarnessConfigValue,
  newCfg: HarnessConfigValue,
): string[] {
  const ids = new Set([...Object.keys(oldCfg.providers), ...Object.keys(newCfg.providers)]);
  const affected: string[] = [];
  for (const id of ids) {
    const oldFp = providerDiscoveryFingerprint(oldCfg.providers[id] ?? {});
    const newFp = providerDiscoveryFingerprint(newCfg.providers[id] ?? {});
    if (oldFp !== newFp) affected.push(id);
  }
  return affected.sort();
}

export type ConfigurationChangeEvent = {
  old_value: JsonValue | null;
  new_value: JsonValue | null;
};

/** Parse a `configuration` trigger payload into old/new values when present. */
export function parseConfigurationChangeEvent(payload: unknown): ConfigurationChangeEvent {
  if (!payload || typeof payload !== 'object') {
    return { old_value: null, new_value: null };
  }
  const o = payload as Record<string, unknown>;
  const body =
    o.new_value !== undefined || o.old_value !== undefined
      ? o
      : o.payload && typeof o.payload === 'object'
        ? (o.payload as Record<string, unknown>)
        : o;
  return {
    old_value: (body.old_value ?? null) as JsonValue | null,
    new_value: (body.new_value ?? null) as JsonValue | null,
  };
}
