#!/usr/bin/env node
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
 *       The SessionStart hook: print the `default-mcp` profile as the hook's
 *       additionalContext, so every session starts on it.
 *
 * The mcp-server URL: --url, then III_PLUGIN_MCP_URL, then the plugin's
 * `mcp_url` option, then http://127.0.0.1:3111/mcp.
 */

import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { writeProfileCommands } from './commands.js';
import { renderSkill, renderSubagent, skillDirectories, slug } from './directory.js';
import { McpClient, McpDirectory, resolveMcpUrl } from './mcp.js';
import { DEFAULT_PROFILE, sessionStartOutput } from './session.js';

function option(argv: string[], name: string): string {
  const i = argv.indexOf(name);
  return i >= 0 && argv[i + 1] ? argv[i + 1] : '';
}

function expandHome(path: string): string {
  return path.replace(/^~(?=\/|$)/, homedir());
}

/** The plugin root: this bundle lives at `<root>/cli/index.mjs`. */
function pluginRoot(argv: string[]): string {
  const explicit = option(argv, '--plugin-dir');
  if (explicit) return resolve(expandHome(explicit));
  if (process.env.CLAUDE_PLUGIN_ROOT) return process.env.CLAUDE_PLUGIN_ROOT;
  return resolve(dirname(fileURLToPath(import.meta.url)), '..');
}

async function sync(dir: McpDirectory, argv: string[]): Promise<void> {
  const root = pluginRoot(argv);
  const agents = await dir.listAgents(true);
  const result = await writeProfileCommands(root, agents, {
    read: (path) => readFile(path, 'utf8').catch(() => null),
    write: async (path, content) => {
      await mkdir(dirname(path), { recursive: true });
      await writeFile(path, content, 'utf8');
    },
    remove: (path) => rm(path, { force: true }),
  });
  if (!argv.includes('--quiet')) {
    console.error(
      `profile commands in ${join(root, 'commands')}: ${result.written.length} written, ${result.removed.length} removed`,
    );
  }
}

async function exportNative(dir: McpDirectory, argv: string[]): Promise<void> {
  const target = resolve(expandHome(option(argv, '--target')) || join(homedir(), '.claude'));
  const agents = await dir.listAgents(argv.includes('--include-hidden'));
  // Check the skills before writing anything: two ids that land in one
  // directory stop the export with nothing written.
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
  console.error(`wrote ${agents.length} agent profile(s) to ${join(target, 'agents')}`);
  if (!argv.includes('--skills')) return;
  for (const row of skills) {
    const skill = await dir.getSkill(row.id);
    const path = join(target, 'skills', slug(row.id));
    await mkdir(path, { recursive: true });
    await writeFile(join(path, 'SKILL.md'), renderSkill({ ...row, ...skill }), 'utf8');
  }
  console.error(`wrote ${skills.length} skill(s) to ${join(target, 'skills')}`);
}

const argv = process.argv.slice(2);
const command = argv[0] && !argv[0].startsWith('-') ? argv[0] : 'status';
let dir: McpDirectory | null = null;

try {
  const mcp = resolveMcpUrl(argv);
  dir = new McpDirectory(new McpClient(mcp.url));
  if (command === 'sync') {
    await sync(dir, argv);
  } else if (command === 'export') {
    await exportNative(dir, argv);
  } else if (command === 'session-start') {
    // Never fail the hook: a session starts either way, told why.
    const result = await dir
      .getAgent(DEFAULT_PROFILE)
      .then((profile) => ({ profile }))
      .catch((err: unknown) => ({ error: err instanceof Error ? err.message : String(err) }));
    process.stdout.write(`${sessionStartOutput(result)}\n`);
  } else if (command === 'status') {
    const status = await dir.status();
    const agents = await dir.listAgents(true);
    console.error(JSON.stringify({ mcp_server: mcp, ...status, agents: agents.length }, null, 2));
  } else {
    console.error(`unknown command: ${command} (expected sync, export, status or session-start)`);
    process.exitCode = 2;
  }
} catch (err) {
  console.error(`iii plugin: ${err instanceof Error ? err.message : String(err)}`);
  process.exitCode = 1;
} finally {
  await dir?.close();
}
