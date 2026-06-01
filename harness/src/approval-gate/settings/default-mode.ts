/**
 * Default approval mode, sourced from the `harness` configuration entry's
 * `permissions.default_mode`. New sessions (no stored approval settings)
 * inherit this mode; sessions with explicit settings keep their own.
 *
 * Seeded at startup and kept live via a `configuration` trigger so an
 * operator editing the harness entry (manual | auto | full) takes effect
 * without a restart. Tolerant of a missing configuration worker — the
 * built-in 'manual' default stands.
 */

import {
  HARNESS_CONFIG_ID,
  type PermissionMode,
  readHarnessConfig,
} from '../../runtime/harness-config.js';
import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';

let defaultMode: PermissionMode = 'manual';

export function getDefaultMode(): PermissionMode {
  return defaultMode;
}

export async function initDefaultMode(iii: ISdk): Promise<void> {
  await refresh(iii);
  try {
    iii.registerFunction('approval::on_harness_config', async () => {
      await refresh(iii);
      return null;
    });
    iii.registerTrigger({
      type: 'configuration',
      function_id: 'approval::on_harness_config',
      config: { configuration_id: HARNESS_CONFIG_ID },
    });
  } catch (err) {
    logger.warn('approval-gate: could not bind harness-config trigger', { err: String(err) });
  }
}

async function refresh(iii: ISdk): Promise<void> {
  try {
    const cfg = await readHarnessConfig(iii);
    defaultMode = cfg.permissions.default_mode;
  } catch (err) {
    logger.warn('approval-gate: default-mode refresh failed', { err: String(err) });
  }
}
