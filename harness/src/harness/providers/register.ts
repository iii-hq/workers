/**
 * Bus surface for the harness provider registry:
 *
 *   - `harness::provider::register` — a provider self-declares its config
 *     schema + defaults; the registry recomposes and re-registers the
 *     `harness` configuration entry.
 *   - `harness::provider::resolve` — a provider fetches its credential +
 *     settings at request time (secret stays server-side; agents are denied
 *     this function in `iii-permissions.yaml`).
 *   - `harness::provider::list` — enumerate declared providers (id,
 *     display_name, supports_model_listing) for the console.
 */

import { z } from 'zod';
import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import { ProviderRegistry } from './registry.js';

const DeclarationSchema = z.object({
  id: z.string().min(1),
  display_name: z.string().optional(),
  credential_env_var: z.string().optional(),
  config_schema: z.record(z.unknown()).optional(),
  defaults: z.record(z.unknown()).optional(),
  supports_model_listing: z.boolean().optional(),
});

const ResolveSchema = z.object({ provider: z.string().min(1) });

/**
 * Construct the registry, seed the base `harness` entry, and register the
 * three bus functions. Returns the registry so callers can reuse it.
 */
export async function registerProviderRegistry(iii: ISdk): Promise<ProviderRegistry> {
  const registry = new ProviderRegistry(iii);
  await registry.init();

  iii.registerFunction(
    'harness::provider::register',
    async (payload: unknown) => {
      const decl = DeclarationSchema.parse(payload);
      await registry.declare(decl);
      return { ok: true };
    },
    {
      description:
        'Self-declare an LLM provider (id, config schema, defaults) into the dynamic harness configuration schema.',
    },
  );

  iii.registerFunction(
    'harness::provider::resolve',
    async (payload: unknown) => {
      const { provider } = ResolveSchema.parse(payload);
      return registry.resolve(provider);
    },
    {
      description:
        'Resolve a provider credential + settings (api_url, max_tokens) from the harness configuration. Server-side only.',
    },
  );

  iii.registerFunction(
    'harness::provider::list',
    async () => ({ providers: registry.list() }),
    { description: 'List providers declared to the harness.' },
  );

  logger.info('provider-registry: ready', {});
  return registry;
}
