import { unwrapBody } from '../runtime/handler.js';
import type { ISdk } from '../runtime/iii.js';
import { EXPECTED_WORKERS } from './expected-workers.js';

const NAME = 'harness';
// Picked up from package.json at build time; falls back to a stable string.
let cachedVersion = '0.1.0';
try {
  // Best-effort runtime read. Vitest + tsx both support import.meta resolution
  // but we want a static fallback for portability.
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  const pkg = await import('../../package.json', { with: { type: 'json' } });
  if (typeof (pkg as { default?: { version?: string } }).default?.version === 'string') {
    cachedVersion = (pkg as { default: { version: string } }).default.version;
  }
} catch {
  // ignore; keep the fallback
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'harness::status',
    async (input: unknown) => {
      // Body is unwrapped to mirror the Rust handler, but we don't read
      // anything from it.
      void unwrapBody(input);
      return {
        ok: true,
        name: NAME,
        version: cachedVersion,
        expected_workers: EXPECTED_WORKERS,
      };
    },
    {
      description:
        'Returns the harness bundle name, version, and the list of expected runtime workers.',
    },
  );
}
