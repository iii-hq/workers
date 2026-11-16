/**
 * The iii plugin's command line. Claude Code talks to the engine's
 * mcp-server worker itself (the plugin's `.mcp.json`); these commands run
 * outside a session and reach it the same way, over MCP's HTTP transport.
 *
 *   index.mjs sync [--url <mcp-url>] [--plugin-dir <dir>] [--quiet]
 *       Generate one `/iii:<profile>` command per visible agent profile into
 *       the plugin's `commands/` (default: the plugin this file belongs to),
 *       and remove commands of profiles that are gone. The plugin's
 *       SessionStart hook runs it in the background, so a standalone Claude
 *       Code picks up new profiles from the next session on.
 *
 *   index.mjs export [--url ...] [--target ~/.claude] [--skills] [--include-hidden]
 *       Write the engine's agent profiles as native Claude Code subagents
 *       (`<target>/agents/iii-<id>.md`), and with --skills every skill as
 *       `<target>/skills/<id>/SKILL.md`. A snapshot: re-run to refresh.
 *
 *   index.mjs status [--url ...]
 *       Print which mcp-server was used, and what it reports.
 *
 *   index.mjs session-start [--url ...]
 *       A SessionStart hook: print the `default-mcp` profile as the hook's
 *       additionalContext, so every session starts on it.
 *
 *
 * The mcp-server URL: --url, then III_PLUGIN_MCP_URL, then the plugin's
 * `mcp_url` option, then http://127.0.0.1:3111/mcp.
 */

import { mkdir, readFile, rename, rm, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { writeProfileCommands } from './commands.js';
import { agentFiles, renderSkill, renderSubagent, skillDirectories, slug } from './directory.js';
import { McpClient, McpDirectory, resolveMcpUrl } from './mcp.js';
import { agentFile, DEFAULT_PROFILE, sessionStartOutput } from './session.js';

function option(argv: string[], name: string): string {
  const i = argv.indexOf(name);
  return i >= 0 && argv[i + 1] ? argv[i + 1] : '';
}

function expandHome(path: string): string {
  return path.replace(/^~(?=\/|$)/, homedir());
}

/** The plugin root: this bundle lives at `<root>/cli/index.mjs`. */
function pluginRoot(argv: string[], env: NodeJS.ProcessEnv): string {
  const explicit = option(argv, '--plugin-dir');
  if (explicit) return resolve(expandHome(explicit));
  if (env.CLAUDE_PLUGIN_ROOT) return env.CLAUDE_PLUGIN_ROOT;
  return resolve(dirname(fileURLToPath(import.meta.url)), '..');
}

/**
 * Write `path` through a temporary file in the same directory, renamed over
 * it: `sync` runs in the background at session start, and a process cut off
 * mid-write must not leave a truncated agent or command for the next session.
 */
async function replaceFile(path: string, content: string): Promise<void> {
  await mkdir(dirname(path), { recursive: true });
  const tmp = `${path}.${process.pid}.${Date.now().toString(36)}.tmp`;
  try {
    await writeFile(tmp, content, 'utf8');
    await rename(tmp, path);
  } catch (err) {
    await rm(tmp, { force: true });
    throw err;
  }
}

async function sync(dir: McpDirectory, argv: string[], io: CliIo): Promise<void> {
  const root = pluginRoot(argv, io.env);
  const agents = await dir.listAgents(true);
  const result = await writeProfileCommands(root, agents, {
    read: (path) => readFile(path, 'utf8').catch(() => null),
    write: replaceFile,
    remove: (path) => rm(path, { force: true }),
  });
  // The `iii` agent is built from default-mcp; if it cannot be read, the
  // agent the plugin has stays.
  const agent = await dir.getAgent(DEFAULT_PROFILE).catch(() => null);
  if (agent?.system_prompt) {
    await replaceFile(join(root, 'agents', 'iii.md'), agentFile(agent));
  }
  if (!argv.includes('--quiet')) {
    io.stderr(
      `profile commands in ${join(root, 'commands')}: ${result.written.length} written, ${result.removed.length} removed; iii agent ${agent?.system_prompt ? 'built from default-mcp' : 'unchanged (default-mcp could not be read)'}\n`,
    );
  }
}

async function exportNative(dir: McpDirectory, argv: string[], io: CliIo): Promise<void> {
  const target = resolve(expandHome(option(argv, '--target')) || join(homedir(), '.claude'));
  const agents = await dir.listAgents(argv.includes('--include-hidden'));
  // Check every file name before writing anything: two ids that land in one
  // file or directory stop the export with nothing written.
  agentFiles(agents.map((row) => row.id));
  const skills = argv.includes('--skills') ? await dir.listSkills() : [];
  skillDirectories(skills.map((row) => row.id));
  await mkdir(join(target, 'agents'), { recursive: true });
  for (const row of agents) {
    const profile = await dir.getAgent(row.id);
    await writeFile(
      join(target, 'agents', `iii-${slug(row.id)}.md`),
      renderSubagent(profile),
      'utf8',
    );
  }
  io.stderr(`wrote ${agents.length} agent profile(s) to ${join(target, 'agents')}\n`);
  if (!argv.includes('--skills')) return;
  for (const row of skills) {
    const skill = await dir.getSkill(row.id);
    const path = join(target, 'skills', slug(row.id));
    await mkdir(path, { recursive: true });
    await writeFile(join(path, 'SKILL.md'), renderSkill({ ...row, ...skill }), 'utf8');
  }
  io.stderr(`wrote ${skills.length} skill(s) to ${join(target, 'skills')}\n`);
}

export type CliIo = {
  env: NodeJS.ProcessEnv;
  fetchFn: typeof fetch;
  stdout: (text: string) => void;
  stderr: (text: string) => void;
};

function message(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/** Run one command; returns the exit code. */
export async function runCli(argv: string[], io: CliIo): Promise<number> {
  const command = argv[0] && !argv[0].startsWith('-') ? argv[0] : 'status';
  let dir: McpDirectory | null = null;
  try {
    if (command === 'session-start') {
      // Never fail the hook: a session starts either way, told why. A bad
      // address is one more reason, reported the same way.
      const result = await (async () => {
        const mcp = resolveMcpUrl(argv, io.env);
        dir = new McpDirectory(new McpClient(mcp.url, io.fetchFn));
        return { profile: await dir.getAgent(DEFAULT_PROFILE) };
      })().catch((err: unknown) => ({ error: message(err) }));
      io.stdout(`${sessionStartOutput(result)}\n`);
      return 0;
    }
    const mcp = resolveMcpUrl(argv, io.env);
    dir = new McpDirectory(new McpClient(mcp.url, io.fetchFn));
    if (command === 'sync') {
      await sync(dir, argv, io);
    } else if (command === 'export') {
      await exportNative(dir, argv, io);
    } else if (command === 'status') {
      const status = await dir.status();
      const agents = await dir.listAgents(true);
      io.stderr(
        `${JSON.stringify({ mcp_server: mcp, ...status, agents: agents.length }, null, 2)}\n`,
      );
    } else {
      io.stderr(`unknown command: ${command} (expected sync, export, status or session-start)\n`);
      return 2;
    }
    return 0;
  } catch (err) {
    io.stderr(`iii plugin: ${message(err)}\n`);
    return 1;
  } finally {
    await (dir as McpDirectory | null)?.close();
  }
}
