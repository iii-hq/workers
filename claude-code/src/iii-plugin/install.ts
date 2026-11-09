/**
 * The `iii` Claude Code plugin, installed wherever this worker runs Claude.
 *
 * One plugin, three sources:
 *   - the committed `iii-plugin/` folder, embedded in the worker bundle
 *     (`assets.generated.ts`): manifest, MCP server, `/iii:profile`,
 *     `/iii:skills`. Its standalone `hooks/hooks.json` (a SessionStart `sync`)
 *     is replaced, because the worker generates the commands itself;
 *   - what only a worker can write (`../plugin.ts`): hooks that report every
 *     turn through this host's `iii` CLI, and the `iii-runtime` skill;
 *   - one `/iii:<profile>` command per visible profile (`./commands.ts`),
 *     generated now, so a session sees the engine's current profiles.
 *
 * Headless turns get it under the OS temp dir (never the caller's repository);
 * terminal sessions get it in `<workspace>/.iii-plugin` on the shell worker's
 * host, through `shell::fs::*`. A `.build-id` hash of the static files makes an
 * unchanged install one small read rather than re-writing the 500 KB server.
 */

import { createHash } from 'node:crypto';
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import type { IIIClient } from 'iii-sdk';
import { fetchIiiContext } from '../iii-context.js';
import { pluginDetail, pluginFiles } from '../plugin.js';
import { readFile as hostRead, writeFile as hostWrite, probe } from '../terminal/host.js';
import { PLUGIN_STATIC_FILES } from './assets.generated.js';
import { type CommandOps, type PluginFile, writeProfileCommands } from './commands.js';
import type { AgentRow } from './directory.js';

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
};

function staticFiles(files: PluginFile[] = PLUGIN_STATIC_FILES): PluginFile[] {
  return files.filter((f) => f.path !== STANDALONE_HOOKS);
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
  if (!fixed.some((f) => f.path === 'server/index.mjs')) {
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
  };
  for (const f of pluginFiles(worker)) {
    const path = `${dir}/${f.path}`;
    if ((await ops.read(path)) !== f.content) await ops.write(path, f.content);
  }

  const details = [pluginDetail(worker)];
  if (options.agents) {
    await writeProfileCommands(dir, options.agents, ops);
  } else {
    details.push(
      `the /iii:<profile> commands were not refreshed: ${options.agentsDetail || 'the profiles could not be read'}`,
    );
  }
  return { dir, detail: details.filter(Boolean).join('; ') };
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

const localOps: Ops = {
  read: (path) => readFile(path, 'utf8').catch(() => null),
  write: async (path, content) => {
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, content, 'utf8');
  },
  remove: (path) => rm(path, { force: true }),
};

/** The plugin on this worker's own disk, for headless turns. Best-effort. */
export async function installLocal(iii: IIIClient): Promise<Installed> {
  try {
    const [context, listed] = await Promise.all([fetchIiiContext(iii), listAgents(iii)]);
    return await installWith(LOCAL_ROOT, localOps, {
      cli: process.env.III_CLI ?? 'iii',
      context,
      agents: listed.agents,
      agentsDetail: listed.detail,
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
  const listed = await listAgents(iii);
  const installed = await installWith(dir, hostOps(iii), {
    cli: bridge || 'iii',
    context,
    agents: listed.agents,
    agentsDetail: listed.detail,
  });
  return { ...installed, bridge };
}
