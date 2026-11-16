/**
 * The `iii` Claude Code plugin, installed wherever this worker runs Claude.
 *
 * One plugin, three sources:
 *   - the committed `iii-plugin/` folder, embedded in the worker bundle
 *     (`assets.generated.ts`): manifest, `/iii:profile`, `/iii:skills`, the
 *     `iii` agent and the CLI. Its standalone `hooks/hooks.json` (a
 *     SessionStart `sync`) is replaced, because the worker generates the
 *     commands itself;
 *   - what only a worker can write (`../plugin.ts`): hooks that report every
 *     turn through this host's `iii` CLI, the `.mcp.json` pointing at the
 *     engine's mcp-server, and the `iii-runtime` skill;
 *   - one `/iii:<profile>` command per visible profile (`./commands.ts`),
 *     generated now, so a session sees the engine's current profiles.
 *
 * Headless turns get it under the OS temp dir (never the caller's repository);
 * terminal sessions get it in `<workspace>/.iii-plugin` on the shell worker's
 * host, through `shell::fs::*`. A `.build-id` hash of the static files makes an
 * unchanged install one small read rather than re-writing every file.
 */

import { createHash, randomBytes } from 'node:crypto';
import { mkdir, readFile, rename, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import type { IIIClient } from 'iii-sdk';
import { fetchIiiContext } from '../iii-context.js';
import { pluginDetail, pluginFiles } from '../plugin.js';
import { readFile as hostRead, writeFile as hostWrite, probe } from '../terminal/host.js';
import { PLUGIN_STATIC_FILES } from './assets.generated.js';
import { type CommandOps, type PluginFile, writeProfileCommands } from './commands.js';
import type { AgentRow } from './directory.js';
import { DEFAULT_MCP_URL, normalizeMcpUrl } from './mcp.js';

export const PLUGIN_NAME = 'iii';
/** Where it lands inside a terminal workspace. */
export const PLUGIN_DIR_NAME = '.iii-plugin';
const LOCAL_ROOT = join(tmpdir(), 'iii-claude-code-plugin');
const MARKER = '.build-id';
/** The standalone hook file; the worker writes its own. */
const STANDALONE_HOOKS = 'hooks/hooks.json';

export type Installed = { dir: string; detail: string };
export type Ops = CommandOps;

export type InstallOptions = {
  /** The `iii` CLI on the host that runs Claude; the activity hooks call it. */
  cli: string;
  context: { text: string; detail: string };
  /** All profiles (hidden included), or null when they could not be read. */
  agents: AgentRow[] | null;
  agentsDetail?: string;
  /** The engine's mcp-server endpoint, and why it may not answer. */
  mcp: { url: string; detail: string };
};

/** Files without which the plugin is not one. */
const REQUIRED = ['.claude-plugin/plugin.json', '.mcp.json'];

function staticFiles(files: PluginFile[] = PLUGIN_STATIC_FILES): PluginFile[] {
  return files.filter((f) => f.path !== STANDALONE_HOOKS);
}

export function buildId(files: PluginFile[] = staticFiles()): string {
  const hash = createHash('sha256');
  for (const f of files) hash.update(f.path).update('\0').update(f.content).update('\0');
  return hash.digest('hex').slice(0, 16);
}

/** Where the worker writes the runtime skill, when it has a context. */
const RUNTIME_SKILL = 'skills/iii-runtime/SKILL.md';

/** Write the whole plugin into `dir` through `ops`. */
export async function installWith(
  dir: string,
  ops: Ops,
  options: InstallOptions,
  embedded: PluginFile[] = PLUGIN_STATIC_FILES,
): Promise<Installed> {
  const fixed = staticFiles(embedded);
  if (!REQUIRED.every((path) => fixed.some((f) => f.path === path))) {
    return {
      dir: '',
      detail:
        'the iii plugin was not embedded in this build (run `pnpm run build:iii-plugin`); the session runs without it',
    };
  }
  const id = buildId(fixed);
  if ((await ops.read(`${dir}/${MARKER}`))?.trim() !== id) {
    for (const f of fixed) await ops.write(`${dir}/${f.path}`, f.content);
    // Last, so an interrupted install is retried rather than trusted.
    await ops.write(`${dir}/${MARKER}`, `${id}\n`);
  }

  const worker = {
    cli: options.cli,
    context: options.context.text,
    contextDetail: options.context.detail,
    mcpUrl: options.mcp.url,
  };
  const generated = pluginFiles(worker);
  for (const f of generated) {
    const path = `${dir}/${f.path}`;
    if ((await ops.read(path)) !== f.content) await ops.write(path, f.content);
  }
  // The runtime skill exists only while there is a context to put in it: a
  // stale one would keep loading old instructions.
  if (!generated.some((f) => f.path === RUNTIME_SKILL)) {
    const path = `${dir}/${RUNTIME_SKILL}`;
    if ((await ops.read(path)) !== null) await ops.remove(path);
  }

  const details = [options.mcp.detail, pluginDetail(worker)];
  if (options.agents) {
    await writeProfileCommands(dir, options.agents, ops);
  } else {
    details.push(
      `the /iii:<profile> commands were not refreshed: ${options.agentsDetail || 'the profiles could not be read'}`,
    );
  }
  return { dir, detail: details.filter(Boolean).join('; ') };
}

const MCP_SETUP =
  "the session's iii tools are unavailable until it runs; add the mcp-server worker behind rbac-proxy (see its README)";

/**
 * Where the engine's mcp-server answers: `III_PLUGIN_MCP_URL` (for a custom
 * `api_path`), else the http worker's address plus `/mcp`. `detail` says why
 * the session's iii tools may not work: no mcp-server on the engine, or no
 * http worker to find it through.
 */
export async function resolveWorkerMcp(
  iii: IIIClient,
  env: NodeJS.ProcessEnv = process.env,
): Promise<{ url: string; detail: string }> {
  const details: string[] = [];
  let url = DEFAULT_MCP_URL;
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
    if (!(res?.functions ?? []).some((f) => f.function_id === 'mcp-server::handler')) {
      details.push(`mcp-server is not running on this engine: ${MCP_SETUP}`);
    }
  } catch (err) {
    details.push(`could not check for mcp-server (${String(err)}); ${MCP_SETUP}`);
  }
  return { url, detail: details.join('; ') };
}

async function listAgents(iii: IIIClient): Promise<{ agents: AgentRow[] | null; detail: string }> {
  try {
    const res = await iii.call<unknown, { agents?: AgentRow[] }>({
      function_id: 'directory::agents::list',
      payload: {},
      timeoutMs: 15_000,
    });
    return { agents: res?.agents ?? [], detail: '' };
  } catch (err) {
    return { agents: null, detail: `iii-directory did not answer (${String(err)})` };
  }
}

/**
 * Write `content` to `path` through a temporary file in the same directory,
 * renamed into place: concurrent installers (sessions run side by side) and
 * a session starting its MCP server meanwhile never see a partial file.
 */
export async function atomicWrite(path: string, content: string): Promise<void> {
  await mkdir(dirname(path), { recursive: true });
  const tmp = `${path}.${process.pid}.${randomBytes(6).toString('hex')}.tmp`;
  try {
    await writeFile(tmp, content, 'utf8');
    await rename(tmp, path);
  } catch (err) {
    await rm(tmp, { force: true });
    throw err;
  }
}

const localOps: Ops = {
  read: (path) => readFile(path, 'utf8').catch(() => null),
  write: atomicWrite,
  remove: (path) => rm(path, { force: true }),
};

/** The plugin on this worker's own disk, for headless turns. Best-effort. */
export async function installLocal(iii: IIIClient): Promise<Installed> {
  try {
    const [context, listed, mcp] = await Promise.all([
      fetchIiiContext(iii),
      listAgents(iii),
      resolveWorkerMcp(iii),
    ]);
    return await installWith(LOCAL_ROOT, localOps, {
      cli: process.env.III_CLI ?? 'iii',
      context,
      agents: listed.agents,
      agentsDetail: listed.detail,
      mcp,
    });
  } catch (err) {
    const detail = `the iii plugin could not be written to ${LOCAL_ROOT}: ${String(err)}`;
    console.warn(`claude-code: ${detail}`);
    return { dir: '', detail };
  }
}

function hostOps(iii: IIIClient): Ops {
  return {
    read: (path) => hostRead(iii, path),
    write: (path, content) => hostWrite(iii, path, content),
    remove: async (path) => {
      try {
        await iii.call({ function_id: 'shell::fs::rm', payload: { path }, timeoutMs: 30_000 });
      } catch (err) {
        if (!String(err).includes('S211')) throw err; // already gone
      }
    },
  };
}

/**
 * The plugin inside a terminal workspace on the shell worker's host. Returns
 * the `iii` CLI it found there (`bridge`), or '' when the activity hooks have
 * no way to reach the bus.
 */
export async function installOnHost(
  iii: IIIClient,
  workspace: string,
  context: { text: string; detail: string },
): Promise<Installed & { bridge: string }> {
  const dir = `${workspace}/${PLUGIN_DIR_NAME}`;
  const bridge = await probe(iii, 'command -v iii');
  const [listed, mcp] = await Promise.all([listAgents(iii), resolveWorkerMcp(iii)]);
  const installed = await installWith(dir, hostOps(iii), {
    cli: bridge || 'iii',
    context,
    agents: listed.agents,
    agentsDetail: listed.detail,
    mcp,
  });
  return { ...installed, bridge };
}
