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
 * Headless turns get it in a private directory of their own under the OS temp
 * dir, removed when the turn ends (never the caller's repository);
 * terminal sessions get it in `<workspace>/.iii-plugin` on the shell worker's
 * host, through `shell::fs::*`. A `.build-id` hash of the static files makes an
 * unchanged install one small read rather than re-writing every file.
 */

import { createHash, randomBytes } from 'node:crypto';
import { mkdir, mkdtemp, readFile, rename, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import type { IIIClient } from 'iii-sdk';
import { fetchIiiContext } from '../iii-context.js';
import { MODE_FILE, pluginDetail, pluginFiles } from '../plugin.js';
import {
  readFile as hostRead,
  writeFile as hostWrite,
  listFiles,
  probe,
} from '../terminal/host.js';
import { PLUGIN_STATIC_FILES } from './assets.generated.js';
import { type CommandOps, type PluginFile, writeProfileCommands } from './commands.js';
import type { AgentProfile, AgentRow } from './directory.js';
import { DEFAULT_MCP_URL, normalizeMcpUrl } from './mcp.js';
import { agentFile } from './session.js';

export const PLUGIN_NAME = 'iii';
/** Where it lands inside a terminal workspace. */
export const PLUGIN_DIR_NAME = '.iii-plugin';
/** Prefix of each headless turn's own private plugin directory. */
const LOCAL_PREFIX = join(tmpdir(), 'iii-claude-code-plugin-');
const MARKER = '.build-id';
/** Static files the worker replaces with its own. */
const WORKER_OWNED = ['hooks/hooks.json', '.mcp.json'];

export type Installed = {
  dir: string;
  detail: string;
  /** The default-mcp prompt to append to the system prompt; '' without it. */
  mode: string;
  /** Whether the session's `iii` MCP server is the engine's mcp-server. */
  mcp: boolean;
};
export type Ops = CommandOps;

export type InstallOptions = {
  /** The `iii` CLI on the host that runs Claude; the activity hooks call it. */
  cli: string;
  context: { text: string; detail: string };
  /** All profiles (hidden included), or null when they could not be read. */
  agents: AgentRow[] | null;
  agentsDetail?: string;
  /** The engine's mcp-server endpoint, whether it runs, and what to say. */
  mcp: { url: string; detail: string; available: boolean };
  /** The default-mcp profile, or null when it could not be read. */
  defaultProfile: AgentProfile | null;
  defaultProfileDetail?: string;
};

/** Files without which the plugin is not one. */
const REQUIRED = ['.claude-plugin/plugin.json'];
/** Written only while they apply; removed otherwise, so nothing stale loads. */
const CONDITIONAL = ['.mcp.json', MODE_FILE, 'skills/iii-runtime/SKILL.md'];

function staticFiles(files: PluginFile[] = PLUGIN_STATIC_FILES): PluginFile[] {
  return files.filter((f) => !WORKER_OWNED.includes(f.path));
}

export function buildId(files: PluginFile[] = staticFiles()): string {
  const hash = createHash('sha256');
  for (const f of files) hash.update(f.path).update('\0').update(f.content).update('\0');
  return hash.digest('hex').slice(0, 16);
}

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
      mode: '',
      mcp: false,
    };
  }
  const id = buildId(fixed);
  if ((await ops.read(`${dir}/${MARKER}`))?.trim() !== id) {
    for (const f of fixed) await ops.write(`${dir}/${f.path}`, f.content);
    // Last, so an interrupted install is retried rather than trusted.
    await ops.write(`${dir}/${MARKER}`, `${id}\n`);
  }

  // With mcp-server, default-mcp is both the text appended to Claude Code's
  // prompt and the `iii` agent. Without it there is no `iii` MCP server and
  // nothing to append: the session works through the `iii` CLI, as before.
  const available = options.mcp.available;
  const profile = available ? options.defaultProfile : null;
  const mode = profile?.system_prompt?.trim() ?? '';
  const stubAgent = embedded.find((f) => f.path === 'agents/iii.md')?.content ?? '';
  const worker = {
    cli: options.cli,
    context: options.context.text,
    contextDetail: options.context.detail,
    mcpUrl: available ? options.mcp.url : '',
    mode,
    agent: mode && profile ? agentFile(profile) : '',
  };
  const generated = pluginFiles(worker);
  for (const f of generated) {
    const path = `${dir}/${f.path}`;
    if ((await ops.read(path)) !== f.content) await ops.write(path, f.content);
  }
  // The MCP server, the default-mcp file and the runtime skill exist only while
  // they apply: a stale one would keep loading.
  for (const file of CONDITIONAL) {
    if (generated.some((f) => f.path === file)) continue;
    const path = `${dir}/${file}`;
    if ((await ops.read(path)) !== null) await ops.remove(path);
  }
  // No profile to build the agent from: the plugin's own stub (which loads
  // default-mcp when run) replaces any agent built from an older one.
  if (!worker.agent && stubAgent && (await ops.read(`${dir}/agents/iii.md`)) !== stubAgent) {
    await ops.write(`${dir}/agents/iii.md`, stubAgent);
  }

  const details = [options.mcp.detail, pluginDetail(worker)];
  if (available && !mode) {
    details.push(
      `the default-mcp profile could not be read (${options.defaultProfileDetail || 'it has no prompt'}), so it is neither appended nor the iii agent`,
    );
  }
  if (options.agents) {
    await writeProfileCommands(dir, options.agents, ops);
  } else {
    details.push(
      `the /iii:<profile> commands were not refreshed: ${options.agentsDetail || 'the profiles could not be read'}`,
    );
  }
  return { dir, detail: details.filter(Boolean).join('; '), mode, mcp: available };
}

const MCP_SETUP =
  'sessions work through the `iii` CLI, without the iii tools or default-mcp, until it runs; add the mcp-server worker behind rbac-proxy (see its README)';

/**
 * Where the engine's mcp-server answers: `III_PLUGIN_MCP_URL` (for a custom
 * `api_path`), else the http worker's address plus `/mcp`. `detail` says why
 * the session's iii tools may not work: no mcp-server on the engine, or no
 * http worker to find it through.
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

/** The default-mcp profile, read from iii-directory. */
async function readDefaultProfile(
  iii: IIIClient,
): Promise<{ profile: AgentProfile | null; detail: string }> {
  try {
    const profile = await iii.call<unknown, AgentProfile>({
      function_id: 'directory::agents::get',
      payload: { id: 'default-mcp' },
      timeoutMs: 15_000,
    });
    return { profile: profile ?? null, detail: '' };
  } catch (err) {
    return { profile: null, detail: `iii-directory did not answer (${String(err)})` };
  }
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
/**
 * The plugin for one headless turn, in a directory of its own: created
 * private (0700) by `mkdtemp`, so no other user can plant files in it and no
 * other worker on the host can point it at another engine. `dispose` removes
 * it when the turn ends. Best-effort: a failure runs the turn without it.
 */
export async function installLocal(
  iii: IIIClient,
): Promise<Installed & { dispose: () => Promise<void> }> {
  let dir = '';
  const dispose = async () => {
    if (dir) await rm(dir, { recursive: true, force: true }).catch(() => {});
  };
  try {
    dir = await mkdtemp(LOCAL_PREFIX);
    const [context, listed, mcp, def] = await Promise.all([
      fetchIiiContext(iii),
      listAgents(iii),
      resolveWorkerMcp(iii),
      readDefaultProfile(iii),
    ]);
    const installed = await installWith(dir, localOps, {
      cli: process.env.III_CLI ?? 'iii',
      context,
      agents: listed.agents,
      agentsDetail: listed.detail,
      mcp,
      defaultProfile: def.profile,
      defaultProfileDetail: def.detail,
    });
    return { ...installed, dispose };
  } catch (err) {
    const detail = `the iii plugin could not be written to ${dir || tmpdir()}: ${String(err)}`;
    console.warn(`claude-code: ${detail}`);
    await dispose();
    return { dir: '', detail, mode: '', mcp: false, dispose: async () => {} };
  }
}

/**
 * File operations on the terminal host, for the plugin in `dir`. `known` is the
 * folder's listing (`listFiles`): a file under `dir` that is not in it reads as
 * null without a call, since `coder::read-file` on a missing file fails (C211)
 * and the engine records each miss. Writes and removals keep it current. With
 * no listing (null), every read goes to the host, as before.
 */
function hostOps(iii: IIIClient, dir: string, known: Set<string> | null): Ops {
  const absent = (path: string) => known !== null && path.startsWith(`${dir}/`) && !known.has(path);
  return {
    read: async (path) => (absent(path) ? null : hostRead(iii, path)),
    write: async (path, content) => {
      await hostWrite(iii, path, content);
      known?.add(path);
    },
    remove: async (path) => {
      try {
        await iii.call({ function_id: 'shell::fs::rm', payload: { path }, timeoutMs: 30_000 });
      } catch (err) {
        if (!String(err).includes('S211')) throw err; // already gone
      }
      known?.delete(path);
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
  const [listed, mcp, def, known] = await Promise.all([
    listAgents(iii),
    resolveWorkerMcp(iii),
    readDefaultProfile(iii),
    listFiles(iii, dir),
  ]);
  const installed = await installWith(dir, hostOps(iii, dir, known), {
    cli: bridge || 'iii',
    context,
    agents: listed.agents,
    agentsDetail: listed.detail,
    mcp,
    defaultProfile: def.profile,
    defaultProfileDetail: def.detail,
  });
  return { ...installed, bridge };
}
