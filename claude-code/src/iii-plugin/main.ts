#!/usr/bin/env node
/**
 * iii plugin entry point.
 *
 *   index.mjs [serve] [--url ws://host:port]
 *       MCP server on stdio (what the plugin's `.mcp.json` runs).
 *
 *   index.mjs sync [--url ...] [--plugin-dir <dir>] [--quiet]
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
 *       Print which engine was resolved and whether it answers.
 */

import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { writeProfileCommands } from './commands.js';
import { Directory, renderSkill, renderSubagent, slug } from './directory.js';
import { serve } from './server.js';

function option(argv: string[], name: string): string {
  const i = argv.indexOf(name);
  return i >= 0 && argv[i + 1] ? argv[i + 1] : '';
}

function expandHome(path: string): string {
  return path.replace(/^~(?=\/|$)/, homedir());
}

/** The plugin root: this bundle lives at `<root>/server/index.mjs`. */
function pluginRoot(argv: string[]): string {
  const explicit = option(argv, '--plugin-dir');
  if (explicit) return resolve(expandHome(explicit));
  if (process.env.CLAUDE_PLUGIN_ROOT) return process.env.CLAUDE_PLUGIN_ROOT;
  return resolve(dirname(fileURLToPath(import.meta.url)), '..');
}

async function sync(dir: Directory, argv: string[]): Promise<void> {
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

async function exportNative(dir: Directory, argv: string[]): Promise<void> {
  const target = resolve(expandHome(option(argv, '--target')) || join(homedir(), '.claude'));
  const agents = await dir.listAgents(argv.includes('--include-hidden'));
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
  const skills = await dir.listSkills();
  for (const row of skills) {
    const skill = await dir.getSkill(row.id);
    const path = join(target, 'skills', slug(row.id));
    await mkdir(path, { recursive: true });
    await writeFile(join(path, 'SKILL.md'), renderSkill({ ...row, ...skill }), 'utf8');
  }
  console.error(`wrote ${skills.length} skill(s) to ${join(target, 'skills')}`);
}

const argv = process.argv.slice(2);
const command = argv[0] && !argv[0].startsWith('-') ? argv[0] : 'serve';
const dir = new Directory();

try {
  if (command === 'serve') {
    await serve(dir);
  } else if (command === 'sync') {
    await sync(dir, argv);
  } else if (command === 'export') {
    await exportNative(dir, argv);
  } else if (command === 'status') {
    const agents = await dir.listAgents(true);
    console.error(JSON.stringify({ ...dir.status(), agents: agents.length }, null, 2));
  } else {
    console.error(`unknown command: ${command} (expected serve, sync, export or status)`);
    process.exitCode = 2;
  }
} catch (err) {
  console.error(`iii plugin: ${err instanceof Error ? err.message : String(err)}`);
  process.exitCode = 1;
} finally {
  await dir.close();
  if (command !== 'serve') process.exit();
}
