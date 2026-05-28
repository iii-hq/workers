/**
 * Bootstrap: register the `database` configuration entry before the database
 * worker binary starts. Invoked by run-tests.sh via `npm run seed-config`.
 */

import { readFileSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { registerWorker } from 'iii-sdk';
import {
  DATABASE_CONFIG_ID,
  DATABASE_CONFIG_VALUE,
} from './database-config.ts';

const URL = process.env.III_URL ?? 'ws://127.0.0.1:49134';
const CONFIG_RETRIES = 3;
const CONFIG_TIMEOUT_MS = 5_000;

const __dirname = dirname(fileURLToPath(import.meta.url));
const SCHEMA_PATH = resolve(__dirname, '../fixtures/database.schema.json');

function loadSchema(): unknown {
  return JSON.parse(readFileSync(SCHEMA_PATH, 'utf8'));
}

async function triggerWithRetry(
  iii: ReturnType<typeof registerWorker>,
  functionId: string,
  payload: unknown,
): Promise<unknown> {
  let lastErr: unknown;
  for (let attempt = 1; attempt <= CONFIG_RETRIES; attempt++) {
    try {
      return await iii.trigger({
        function_id: functionId,
        payload,
        timeout_ms: CONFIG_TIMEOUT_MS,
      });
    } catch (e) {
      lastErr = e;
      if (attempt < CONFIG_RETRIES) {
        console.warn(
          `[seed-config] ${functionId} failed (attempt ${attempt}/${CONFIG_RETRIES}):`,
          e,
        );
        await new Promise((r) => setTimeout(r, 250 * attempt));
      }
    }
  }
  throw lastErr;
}

async function main(): Promise<void> {
  const iii = registerWorker(URL);
  const schema = loadSchema();

  console.log('[seed-config] registering configuration entry', {
    id: DATABASE_CONFIG_ID,
    url: URL,
  });

  await triggerWithRetry(iii, 'configuration::register', {
    id: DATABASE_CONFIG_ID,
    name: 'Database',
    description: 'Connection pools for PostgreSQL, MySQL, and SQLite.',
    schema,
    initial_value: DATABASE_CONFIG_VALUE,
  });

  const got = (await triggerWithRetry(iii, 'configuration::get', {
    id: DATABASE_CONFIG_ID,
  })) as { id?: string; value?: unknown };

  if (got?.id !== DATABASE_CONFIG_ID || got.value == null) {
    throw new Error(
      `configuration::get verification failed: ${JSON.stringify(got)}`,
    );
  }

  console.log('[seed-config] configuration entry ready');
  await new Promise((r) => setTimeout(r, 200));
  await iii.shutdown();
}

main().catch((e) => {
  console.error('[seed-config] fatal:', e?.stack ?? e);
  process.exit(1);
});
