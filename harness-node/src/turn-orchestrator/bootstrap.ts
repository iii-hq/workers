/**
 * Best-effort fetch of default skills at boot. Mirrors
 * `turn-orchestrator/src/bootstrap.rs`. Failures are logged and never
 * abort startup.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { TurnOrchestratorConfig } from './config.js';

export async function run(iii: ISdk, cfg: TurnOrchestratorConfig): Promise<void> {
  const namespaces = new Set<string>();
  for (const uri of cfg.system_default_skills) {
    const id = uri.startsWith('iii://') ? uri.slice('iii://'.length) : uri;
    const ns = id.split('/')[0];
    if (ns) namespaces.add(ns);
  }
  for (const ns of namespaces) {
    try {
      await iii.trigger<unknown, unknown>({
        function_id: 'directory::skills::download',
        payload: { id: ns },
        timeoutMs: 60_000,
      });
    } catch (err) {
      logger.debug('directory::skills::download failed (best-effort)', {
        id: ns,
        err: String(err),
      });
    }
  }
}
