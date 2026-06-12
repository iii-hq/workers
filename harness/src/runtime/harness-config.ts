/**
 * Shape of the single `harness` configuration entry that lives in the
 * built-in `configuration` worker — the agent `permissions` block only.
 * Provider credentials/settings moved to the `llm-router` entry, whose
 * schema the router composes from provider declarations
 * (see `harness/migrate-llm-router-config.ts` for the one-time copy).
 *
 * Value shape:
 *
 *   {
 *     "permissions": { "default_mode": "manual" | "auto" | "full" }
 *   }
 *
 * A stale `providers` block from the pre-router layout may still be present
 * in stored values; it is ignored here and tolerated by the entry schema.
 */

import { configurationGet, type JsonValue } from './configuration.js';
import type { ISdk } from './iii.js';

/** The id of the harness configuration entry. */
export const HARNESS_CONFIG_ID = 'harness';

export const PERMISSION_MODES = ['manual', 'auto', 'full'] as const;
export type PermissionMode = (typeof PERMISSION_MODES)[number];

export type HarnessPermissions = {
  default_mode: PermissionMode;
};

export type HarnessConfigValue = {
  permissions: HarnessPermissions;
};

export const DEFAULT_PERMISSION_MODE: PermissionMode = 'manual';

function isPermissionMode(v: unknown): v is PermissionMode {
  return v === 'manual' || v === 'auto' || v === 'full';
}

/** Coerce an arbitrary JSON value into a well-formed `HarnessConfigValue`. */
export function normalizeHarnessConfig(value: JsonValue | null): HarnessConfigValue {
  const base: HarnessConfigValue = {
    permissions: { default_mode: DEFAULT_PERMISSION_MODE },
  };
  if (!value || typeof value !== 'object' || Array.isArray(value)) return base;
  const obj = value as Record<string, JsonValue>;

  const permissions = obj.permissions;
  if (permissions && typeof permissions === 'object' && !Array.isArray(permissions)) {
    const mode = (permissions as Record<string, JsonValue>).default_mode;
    if (isPermissionMode(mode)) base.permissions.default_mode = mode;
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
