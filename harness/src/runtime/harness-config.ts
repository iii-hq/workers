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
