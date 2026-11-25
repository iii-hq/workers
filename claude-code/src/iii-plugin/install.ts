/**
 * The `iii` Claude Code plugin, installed where a session runs, with Claude
 * Code's own commands.
 *
 * The plugin is the PUBLISHED one: `iii@iii`, plugin `iii` of the `iii`
 * marketplace (iii-hq/agent-plugins-claude). The worker writes no copy of
 * it, and passes no `--plugin-dir`. For a folder it first adds the
 * marketplace when missing, else refreshes Claude Code's local copy of it
 * (`claude plugin marketplace update iii`), so a new release is seen. Then it
 * asks `claude plugin list --json` (run in that folder) whether `iii@iii`
 * already covers it: when installed, it is updated in place at its own scope
 * (a no-op when current), never reinstalled; when not, it is installed with
 * `claude plugin install iii@iii --scope local` there. The plugin is not
 * configured: it has no options.
 *
 * Where mcp-server answers is not stored anywhere either. The plugin's
 * `.mcp.json` reads `III_MCP_URL` from the environment of the Claude process
 * (`${III_MCP_URL:-http://127.0.0.1:3111/mcp}`), so the worker sets it, read
 * fresh from `http::status`, on EVERY Claude process it starts (`mcpEnv`):
 * two projects on two http ports never share one stored value, and a moved
 * http worker reaches the next session.
 *
 * Both halves use this module: a terminal session runs the commands through
 * `shell::exec` on the terminal host, a headless turn on this worker's own
 * host (`./local-run.ts`).
 */

import type { IIIClient } from 'iii-sdk';
import type { AgentProfile } from './shared/directory.js';
import { DEFAULT_MCP_URL, normalizeMcpUrl } from './shared/mcp.js';
import { profileText } from './shared/session.js';
import { PLUGIN_SOURCE } from './source.generated.js';

export const PLUGIN_ID = 'iii@iii';
export const MARKETPLACE_NAME = 'iii';
export const MARKETPLACE_SOURCE = 'iii-hq/agent-plugins-claude';

export type RunResult = { stdout: string; stderr: string; exit_code: number };
/** Run `program` with `args` in `cwd` (no shell) on the host Claude runs on. */
export type Runner = (program: string, args: string[], cwd: string) => Promise<RunResult>;

export type PluginStatus = {
  /**
   * `present`: already installed and enabled for the folder, left alone.
   * `installed` / `enabled`: done now. `failed`: the session runs without it.
   */
  state: 'present' | 'installed' | 'enabled' | 'failed';
  folder: string;
  detail: string;
};

/** Whether the plugin is loaded in a session started in its folder. */
export function pluginLoaded(status: PluginStatus | null): boolean {
  return status !== null && status.state !== 'failed';
}

type ListEntry = { id?: unknown; scope?: unknown; enabled?: unknown; projectPath?: unknown };

/**
 * What `claude plugin list --json` says about `iii@iii` in `folder` (a real
 * path: Claude Code records the folder resolved). It lists every install of
 * the user, whatever folder it was run in: a user install covers every
 * folder, a local or project one only its own.
 */
export function pluginCoverage(list: unknown, folder: string): 'enabled' | 'disabled' | 'absent' {
  if (!Array.isArray(list)) return 'absent';
  const mine = (list as ListEntry[]).filter((e) => e?.id === PLUGIN_ID);
  const covers = (e: ListEntry) => e.scope === 'user' || e.projectPath === folder;
  if (mine.some((e) => covers(e) && e.enabled === true)) return 'enabled';
  if (mine.some((e) => e.scope === 'local' && e.projectPath === folder)) return 'disabled';
  return 'absent';
}

/**
 * The environment that points one Claude process, and the plugin's hooks,
 * at mcp-server and the bus: `III_MCP_URL` for the plugin's `.mcp.json`,
 * `III_PLUGIN_MCP_URL` for the plugin CLI's hooks of the currently released
 * plugin (it reads that name first), and `III_CLI`, the `iii` CLI the
 * activity hooks report through, when there is one.
 */
export function mcpEnv(mcpUrl: string, cli: string): Record<string, string> {
  return {
    III_MCP_URL: mcpUrl,
    III_PLUGIN_MCP_URL: mcpUrl,
    ...(cli ? { III_CLI: cli } : {}),
  };
}

function said(r: RunResult): string {
  return (r.stderr || r.stdout).trim().split('\n').slice(-3).join(' ') || `exit ${r.exit_code}`;
}

function parseJson(text: string): { ok: true; value: unknown } | { ok: false } {
  try {
    return { ok: true, value: JSON.parse(text) };
  } catch {
    return { ok: false };
  }
}

/** One check at a time per folder: two sessions starting together install once. */
const inFlight = new Map<string, Promise<PluginStatus>>();

/**
 * Make sure `iii@iii` is installed and enabled for `folder` (a real path).
 * Never throws: a failure is a status, and the session starts without it.
 */
export function ensurePlugin(run: Runner, claude: string, folder: string): Promise<PluginStatus> {
  const previous = inFlight.get(folder) ?? Promise.resolve(null);
  const next = previous
    .catch(() => null)
    .then(() => ensureNow(run, claude, folder))
    .finally(() => {
      if (inFlight.get(folder) === next) inFlight.delete(folder);
    });
  inFlight.set(folder, next);
  return next;
}

async function ensureNow(run: Runner, claude: string, folder: string): Promise<PluginStatus> {
  const failed = (detail: string): PluginStatus => ({
    state: 'failed',
    folder,
    detail: `the iii plugin is not loaded in ${folder}: ${detail}`,
  });
  // Problems that leave the plugin loaded, an older release of it at worst.
  const notes: string[] = [];
  try {
    // The marketplace first, refreshed every time: Claude Code installs and
    // updates from its local copy, which does not move on its own, so a
    // stale copy would hold every folder at an old release.
    const markets = await run(claude, ['plugin', 'marketplace', 'list', '--json'], folder);
    const known = markets.exit_code === 0 ? parseJson(markets.stdout) : { ok: false as const };
    const hasMarket =
      known.ok &&
      Array.isArray(known.value) &&
      (known.value as { name?: unknown }[]).some((m) => m?.name === MARKETPLACE_NAME);
    if (hasMarket) {
      const refreshed = await run(
        claude,
        ['plugin', 'marketplace', 'update', MARKETPLACE_NAME],
        folder,
      );
      if (refreshed.exit_code !== 0) {
        notes.push(
          `the ${MARKETPLACE_NAME} marketplace could not be refreshed (${said(refreshed)}), so the plugin may be an older release`,
        );
      }
    } else {
      const added = await run(
        claude,
        ['plugin', 'marketplace', 'add', MARKETPLACE_SOURCE, '--json'],
        folder,
      );
      if (added.exit_code !== 0) {
        return failed(`the ${MARKETPLACE_SOURCE} marketplace could not be added (${said(added)})`);
      }
    }

    const listed = await run(claude, ['plugin', 'list', '--json'], folder);
    if (listed.exit_code !== 0) return failed(`\`claude plugin list\` failed (${said(listed)})`);
    const list = parseJson(listed.stdout);
    if (!list.ok) return failed('`claude plugin list --json` printed something that is not JSON');
    const coverage = pluginCoverage(list.value, folder);
    const detail = () => notes.join('; ');

    if (coverage === 'absent') {
      const installed = await run(
        claude,
        ['plugin', 'install', PLUGIN_ID, '--scope', 'local', '--json'],
        folder,
      );
      if (installed.exit_code !== 0) {
        return failed(`\`claude plugin install ${PLUGIN_ID}\` failed (${said(installed)})`);
      }
      return { state: 'installed', folder, detail: detail() };
    }

    // Installed: bring it to the marketplace's release in place, at the scope
    // it was installed at (a no-op when it is current). Never a reinstall.
    const scope = coverage === 'enabled' ? coveringScope(list.value, folder) : 'local';
    const updated = await run(
      claude,
      ['plugin', 'update', PLUGIN_ID, '--scope', scope, '--json'],
      folder,
    );
    if (updated.exit_code !== 0) {
      notes.push(
        `\`claude plugin update ${PLUGIN_ID}\` failed (${said(updated)}); the installed release is kept`,
      );
    }
    if (coverage === 'enabled') return { state: 'present', folder, detail: detail() };

    const enabled = await run(
      claude,
      ['plugin', 'enable', PLUGIN_ID, '--scope', 'local', '--json'],
      folder,
    );
    if (enabled.exit_code !== 0)
      return failed(`\`claude plugin enable\` failed (${said(enabled)})`);
    return { state: 'enabled', folder, detail: detail() };
  } catch (err) {
    return failed(String(err));
  }
}

/** The scope of the enabled install that covers `folder`: its own, else the user's. */
function coveringScope(list: unknown, folder: string): string {
  const mine = (Array.isArray(list) ? (list as ListEntry[]) : []).filter(
    (e) => e?.id === PLUGIN_ID && e.enabled === true,
  );
  const own = mine.find((e) => e.projectPath === folder && typeof e.scope === 'string');
  if (own) return own.scope as string;
  return 'user';
}

const MCP_SETUP =
  'sessions work through the `iii` CLI, without the iii tools or default-mcp, until it runs; add the mcp-server worker behind rbac-proxy (see its README)';

/**
 * Where the engine's mcp-server answers: `III_PLUGIN_MCP_URL` (for a custom
 * `api_path`), else the http worker's address plus `/mcp`, asked NOW: every
 * session start calls this, so a moved http worker reaches the next session.
 * `detail` says why the session's iii tools may not work.
 */
export async function resolveWorkerMcp(
  iii: IIIClient,
  env: NodeJS.ProcessEnv = process.env,
): Promise<{ url: string; detail: string; available: boolean }> {
  const details: string[] = [];
  let url = DEFAULT_MCP_URL;
  let available = false;
  const override = env.III_PLUGIN_MCP_URL?.trim();
  if (override) {
    url = normalizeMcpUrl(override);
  } else {
    try {
      const status = await iii.call<unknown, { url?: string }>({
        function_id: 'http::status',
        payload: {},
        timeoutMs: 5_000,
      });
      if (status?.url) url = normalizeMcpUrl(status.url);
    } catch (err) {
      details.push(`http::status did not answer (${String(err)}), so the session uses ${url}`);
    }
  }
  try {
    const res = await iii.call<unknown, { functions?: { function_id: string }[] }>({
      function_id: 'engine::functions::list',
      payload: { prefix: 'mcp-server::' },
      timeoutMs: 5_000,
    });
    available = (res?.functions ?? []).some((f) => f.function_id === 'mcp-server::handler');
    if (!available) details.push(`mcp-server is not running on this engine: ${MCP_SETUP}`);
  } catch (err) {
    details.push(`could not check for mcp-server (${String(err)}); ${MCP_SETUP}`);
  }
  return { url, detail: details.join('; '), available };
}

/**
 * The default-mcp profile as the text appended to Claude Code's system
 * prompt (its prompt, then its skills to read), from iii-directory.
 */
export async function readDefaultMcp(iii: IIIClient): Promise<{ text: string; detail: string }> {
  try {
    const profile = await iii.call<unknown, AgentProfile>({
      function_id: 'directory::agents::get',
      payload: { id: 'default-mcp' },
      timeoutMs: 15_000,
    });
    if (!profile?.system_prompt?.trim()) {
      return { text: '', detail: 'the default-mcp profile has no prompt' };
    }
    return { text: profileText(profile), detail: '' };
  } catch (err) {
    return { text: '', detail: `the default-mcp profile could not be read (${String(err)})` };
  }
}

/** Which plugin commit the shared modules (`./shared/`) were copied from, logged once at start. */
export function pluginSourceLine(source: { repo: string; commit: string } = PLUGIN_SOURCE): string {
  return `claude-code: iii plugin modules from ${source.repo} at ${source.commit}`;
}
