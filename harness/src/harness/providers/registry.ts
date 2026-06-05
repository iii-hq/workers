/**
 * In-memory provider registry that owns the `harness` configuration entry.
 *
 * Each provider worker self-declares (`harness::provider::register`) its
 * config schema + defaults at startup. The registry composes the aggregate
 * JSON Schema and (re-)registers the `harness` entry in the `configuration`
 * worker, so the editable shape grows/shrinks with the set of live providers.
 *
 * Providers fetch their secret + settings at request time via
 * `harness::provider::resolve`, which reads the stored value and falls back
 * to the provider's env var when no `api_key` is configured.
 *
 * Declarations are serialized through an in-process queue so concurrent
 * startup declarations from sibling provider workers can't clobber each
 * other's schema/value writes — the harness composite is the single owner.
 */

import {
  configurationGet,
  configurationRegister,
  type JsonSchema,
  type JsonValue,
} from '../../runtime/configuration.js';
import {
  baseHarnessConfigValue,
  DEFAULT_PERMISSION_MODE,
  HARNESS_CONFIG_ID,
  type HarnessConfigValue,
  type HarnessProviderConfig,
  normalizeHarnessConfig,
  PERMISSION_MODES,
} from '../../runtime/harness-config.js';
import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import type { Credential } from '../../runtime/provider-resolve.js';

const ENTRY_NAME = 'harness';
const ENTRY_DESCRIPTION =
  'LLM provider credentials/settings and agent permissions, managed by the harness.';

export type ProviderDefaults = {
  api_url?: string;
  max_tokens?: number;
} & Record<string, unknown>;

/** Payload a provider sends to `harness::provider::register`. */
export type ProviderDeclaration = {
  id: string;
  display_name?: string;
  /** Env var consulted as a credential fallback when no `api_key` is configured. */
  credential_env_var?: string;
  /** JSON Schema for this provider's config object (api_key, api_url, ...). */
  config_schema?: JsonSchema;
  /** Default settings, used to seed the value and as a resolve() fallback. */
  defaults?: ProviderDefaults;
  /** True when the provider exposes `provider::<id>::refresh_models`. */
  supports_model_listing?: boolean;
};

/** Result of `harness::provider::resolve`. */
export type ProviderResolveResult = {
  configured: boolean;
  source: 'stored' | 'environment' | null;
  credential: Credential | null;
  api_url: string | null;
  max_tokens: number | null;
};

export type ProviderListEntry = {
  id: string;
  display_name: string;
  supports_model_listing: boolean;
};

/** Generic per-provider schema used when a provider declares without one. */
function defaultProviderSchema(decl: ProviderDeclaration): JsonSchema {
  return {
    type: 'object',
    title: decl.display_name ?? decl.id,
    properties: {
      api_key: { type: 'string', title: 'api key', format: 'password', writeOnly: true },
      api_url: {
        type: 'string',
        title: 'api url',
        ...(decl.defaults?.api_url !== undefined && { default: decl.defaults.api_url }),
      },
      max_tokens: {
        type: 'integer',
        title: 'max tokens',
        minimum: 1,
        maximum: 1_048_576,
        ...(decl.defaults?.max_tokens !== undefined && { default: decl.defaults.max_tokens }),
      },
    },
    additionalProperties: false,
  };
}

export class ProviderRegistry {
  private readonly iii: ISdk;
  private readonly providers = new Map<string, ProviderDeclaration>();
  private writeChain: Promise<void> = Promise.resolve();

  constructor(iii: ISdk) {
    this.iii = iii;
  }

  /**
   * Ensure the `harness` entry exists with at least the permissions block,
   * even before any provider declares. Tolerant of a missing configuration
   * worker — logs and moves on.
   */
  async init(): Promise<void> {
    await this.enqueue(() => this.ensureRegistered());
  }

  /** Record a provider declaration and re-register the composite schema. */
  async declare(decl: ProviderDeclaration): Promise<void> {
    if (!decl.id) throw new Error('provider declaration requires an id');
    await this.enqueue(async () => {
      this.providers.set(decl.id, decl);
      await this.ensureRegistered();
    });
  }

  list(): ProviderListEntry[] {
    return [...this.providers.values()].map((d) => ({
      id: d.id,
      display_name: d.display_name ?? d.id,
      supports_model_listing: d.supports_model_listing ?? false,
    }));
  }

  /** Resolve a provider's credential + settings for a stream/complete call. */
  async resolve(providerId: string): Promise<ProviderResolveResult> {
    const value = await this.readValue({ raw: false });
    const cfg: HarnessProviderConfig = value.providers[providerId] ?? {};
    const decl = this.providers.get(providerId);

    let credential: Credential | null = null;
    let source: 'stored' | 'environment' | null = null;
    const storedKey =
      typeof cfg.api_key === 'string' && cfg.api_key.length > 0 ? cfg.api_key : null;
    if (storedKey) {
      credential = { type: 'api_key', key: storedKey };
      source = 'stored';
    } else {
      const envVar = decl?.credential_env_var;
      const envKey = envVar ? process.env[envVar] : undefined;
      if (envKey) {
        credential = { type: 'api_key', key: envKey };
        source = 'environment';
      }
    }

    const api_url =
      typeof cfg.api_url === 'string' && cfg.api_url.length > 0
        ? cfg.api_url
        : (decl?.defaults?.api_url ?? null);
    // Only a user-configured max_tokens counts. The declared default is NOT
    // seeded here — the per-model clamp (runtime/output-tokens.ts) would
    // treat it as a deliberate override and pin every request to 8192;
    // providers apply their own fallback after the clamp.
    const max_tokens =
      typeof cfg.max_tokens === 'number' && cfg.max_tokens > 0 ? cfg.max_tokens : null;

    return { configured: credential !== null, source, credential, api_url, max_tokens };
  }

  // -------------------------------------------------------------------------
  // Internals
  // -------------------------------------------------------------------------

  private composeSchema(): JsonSchema {
    const providerProps: Record<string, JsonSchema> = {};
    for (const [id, decl] of this.providers) {
      providerProps[id] = decl.config_schema ?? defaultProviderSchema(decl);
    }
    return {
      $schema: 'http://json-schema.org/draft-07/schema#',
      type: 'object',
      title: ENTRY_NAME,
      properties: {
        permissions: {
          type: 'object',
          title: 'permissions',
          properties: {
            default_mode: {
              type: 'string',
              title: 'default mode',
              description: 'Default approval mode applied to new agent sessions.',
              enum: [...PERMISSION_MODES],
              default: DEFAULT_PERMISSION_MODE,
            },
          },
          required: ['default_mode'],
          additionalProperties: false,
        },
        providers: {
          type: 'object',
          title: 'providers',
          description: 'Per-provider credentials and settings.',
          properties: providerProps,
          additionalProperties: false,
        },
      },
      required: ['permissions', 'providers'],
      additionalProperties: false,
    };
  }

  private async readValue(opts: { raw: boolean }): Promise<HarnessConfigValue> {
    const raw = await configurationGet(this.iii, HARNESS_CONFIG_ID, { raw: opts.raw });
    return normalizeHarnessConfig(raw);
  }

  private async ensureRegistered(): Promise<void> {
    const schema = this.composeSchema();
    try {
      // Read the stored template form so re-registration preserves any
      // operator-set value; only seed on the very first registration.
      const existing = await configurationGet(this.iii, HARNESS_CONFIG_ID, { raw: true });
      await configurationRegister(this.iii, {
        id: HARNESS_CONFIG_ID,
        name: ENTRY_NAME,
        description: ENTRY_DESCRIPTION,
        schema,
        ...(existing === null && {
          initial_value: baseHarnessConfigValue() as unknown as JsonValue,
        }),
      });
    } catch (err) {
      logger.warn('provider-registry: configuration::register failed', { err: String(err) });
    }
  }

  private enqueue<R>(op: () => Promise<R>): Promise<R> {
    const next = this.writeChain.then(() => op());
    this.writeChain = next.then(
      () => undefined,
      () => undefined,
    );
    return next;
  }
}
