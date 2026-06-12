/**
 * Registers the `harness` configuration entry — permissions only. Provider
 * credentials/settings moved to the `llm-router` entry, whose schema the
 * router composes from provider declarations; this entry keeps the agent
 * permissions block (and nothing else) editable in the console.
 *
 * Root `additionalProperties` stays true so a stale `providers` block left
 * behind by the pre-router layout never fails value validation; the migration
 * (`migrate-llm-router-config.ts`) copies it, it just stops being editable.
 */

import { configurationGet, configurationRegister, type JsonValue } from '../runtime/configuration.js';
import { DEFAULT_PERMISSION_MODE, HARNESS_CONFIG_ID, PERMISSION_MODES } from '../runtime/harness-config.js';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';

const ENTRY_DESCRIPTION = 'Agent permissions, managed by the harness.';

export async function registerHarnessConfigEntry(iii: ISdk): Promise<void> {
  const schema = {
    $schema: 'http://json-schema.org/draft-07/schema#',
    type: 'object',
    title: HARNESS_CONFIG_ID,
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
    },
    required: ['permissions'],
    additionalProperties: true,
  };

  try {
    // Read the stored template form so re-registration preserves any
    // operator-set value; only seed on the very first registration.
    const existing = await configurationGet(iii, HARNESS_CONFIG_ID, { raw: true });
    await configurationRegister(iii, {
      id: HARNESS_CONFIG_ID,
      name: HARNESS_CONFIG_ID,
      description: ENTRY_DESCRIPTION,
      schema,
      ...(existing === null && {
        initial_value: {
          permissions: { default_mode: DEFAULT_PERMISSION_MODE },
        } as unknown as JsonValue,
      }),
    });
  } catch (err) {
    logger.warn('harness: configuration::register failed', { err: String(err) });
  }
}
