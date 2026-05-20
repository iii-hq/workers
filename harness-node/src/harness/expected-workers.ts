/**
 * Workers the harness composes. Mirrors the dependency block of
 * `harness/iii.worker.yaml`. Adjust this list when adding/removing
 * workers from the bundle.
 */

export const EXPECTED_WORKERS: readonly string[] = [
  'iii-state',
  'iii-queue',
  'iii-stream',
  'iii-bridge',
  'iii-http',
  'iii-sandbox',
  'iii-directory',
  'turn-orchestrator',
  'models-catalog',
  'shell',
  'provider-anthropic',
  'provider-openai',
  'approval-gate',
  'session',
  'hook-fanout',
  'auth-credentials',
  'llm-budget',
] as const;
