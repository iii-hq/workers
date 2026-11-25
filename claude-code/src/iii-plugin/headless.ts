/**
 * The `iii` plugin for one headless `claude::run` turn, on this worker's own
 * host: the published plugin checked (and installed, scope local) in the
 * folder the turn runs in, and the environment of the turn's Claude process
 * read now: `III_MCP_URL` (the http worker's address plus `/mcp`) and
 * `III_CLI`. No plugin copy, no `--plugin-dir`: the turn loads the plugin
 * from the folder's local settings (`settingSources: ['local']`).
 */

import { accessSync, constants } from 'node:fs';
import { delimiter, join } from 'node:path';
import type { IIIClient } from 'iii-sdk';
import type { Config } from '../config.js';
import {
  ensurePlugin,
  mcpEnv,
  type PluginStatus,
  readDefaultMcp,
  resolveWorkerMcp,
} from './install.js';
import { localRun } from './local-run.js';

export type HeadlessPlugin = {
  /** The plugin check; null when the iii context is off (nothing touched). */
  status: PluginStatus | null;
  mcp: { url: string; detail: string; available: boolean };
  /** default-mcp as text to append; '' without mcp-server or the profile. */
  mode: string;
  /** The whole environment of the turn's Claude process. */
  env: Record<string, string>;
  detail: string;
};

/** `III_CLI` when the worker has one, else `iii` found on PATH, else ''. */
export function iiiCli(env: NodeJS.ProcessEnv = process.env): string {
  if (env.III_CLI?.trim()) return env.III_CLI.trim();
  for (const dir of (env.PATH ?? '').split(delimiter)) {
    if (!dir) continue;
    const candidate = join(dir, 'iii');
    try {
      accessSync(candidate, constants.X_OK);
      return candidate;
    } catch {
      // not here
    }
  }
  return '';
}

function processEnv(): Record<string, string> {
  const env: Record<string, string> = {};
  for (const [k, v] of Object.entries(process.env)) if (typeof v === 'string') env[k] = v;
  return env;
}

export async function headlessPlugin(
  iii: IIIClient,
  cfg: Config,
  cwd: string,
  withPlugin: boolean,
): Promise<HeadlessPlugin> {
  const mcp = await resolveWorkerMcp(iii);
  const env = { ...processEnv(), ...mcpEnv(mcp.url, iiiCli()) };
  if (!withPlugin) return { status: null, mcp, mode: '', env, detail: '' };

  const asked = cwd || process.cwd();
  const real = await localRun('pwd', ['-P'], asked);
  const folder = real.exit_code === 0 ? real.stdout.trim() : '';
  const status: PluginStatus = folder
    ? await ensurePlugin(localRun, cfg.claude_executable || 'claude', folder)
    : {
        state: 'failed',
        folder: asked,
        detail: `the iii plugin is not loaded: ${asked} is not reachable (${real.stderr.trim()})`,
      };
  const details = [mcp.detail, status.detail];
  let mode = '';
  if (mcp.available) {
    const profile = await readDefaultMcp(iii);
    mode = profile.text;
    if (!mode) details.push(`default-mcp is not appended: ${profile.detail}`);
  }
  return { status, mcp, mode, env, detail: details.filter(Boolean).join('; ') };
}
