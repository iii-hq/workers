/**
 * One slash command per visible agent profile: `/iii:<profile> [task]`.
 *
 * Claude Code namespaces plugin commands as `/<plugin>:<command>`, so in the
 * `iii` plugin a file `commands/writer.md` is `/iii:writer`, and typing `/iii:`
 * completes every profile. The two static commands, `/iii:profile <id>` and
 * `/iii:skills <id>`, are committed files; these are generated from the engine,
 * by the worker when it installs the plugin and by the plugin's own `sync`
 * (run from its SessionStart hook) when Claude Code runs standalone.
 *
 * Every command sets `disable-model-invocation: true`: measured with
 * `/context`, that keeps a command's description out of the session entirely
 * until a person invokes it, so a command per profile costs no tokens.
 *
 * Generated paths are recorded in a manifest so a profile removed from the
 * engine loses its command, and nothing else in `commands/` is ever deleted.
 */

import { type AgentRow, slug } from './directory.js';

export type PluginFile = { path: string; content: string };

/** The plugin is `iii` and its MCP server is `iii`. */
export const TOOL_PREFIX = 'mcp__plugin_iii_iii__';
export const COMMANDS_MANIFEST = '.generated-commands.json';

/**
 * Names a generated command must not take: the static commands, and `skill`,
 * which Claude Code cannot load as a plugin command (a `commands/skill.md`
 * silently disables every command in the plugin; verified on 2.1.319).
 */
const RESERVED = new Set(['profile', 'skills', 'skill']);

const DESCRIPTION_MAX = 140;

export function profileCommandName(id: string): string {
  const name = slug(id);
  return RESERVED.has(name) ? `profile-${name}` : name;
}

function yamlString(value: string): string {
  return JSON.stringify(value.replace(/\s+/g, ' ').trim());
}

function shorten(text: string): string {
  const flat = text.replace(/\s+/g, ' ').trim();
  return flat.length > DESCRIPTION_MAX ? `${flat.slice(0, DESCRIPTION_MAX - 1).trimEnd()}…` : flat;
}

/** Every plugin prompt says how to reach the engine. */
export const ENGINE_NOTE = `
While you work, act on the engine with \`iii_call\` (any function id, with a
payload) instead of the \`iii\` CLI, and react to it with \`iii_observe\` (then
\`iii_events\`, and \`iii_unobserve\` when done) instead of polling. Where a
profile or skill says \`agent_call\` or \`harness::observe\`, use \`iii_call\` and
\`iii_observe\`.
`;

export function profileCommand(row: AgentRow): PluginFile {
  const about = shorten(row.description || row.name || '');
  const tools = ['iii_get_agent', 'iii_get_skill', 'iii_status']
    .map((t) => TOOL_PREFIX + t)
    .join(', ');
  return {
    path: `commands/${profileCommandName(row.id)}.md`,
    content: `---
description: ${yamlString(`iii profile ${row.id}${about ? `: ${about}` : ''}`)}
argument-hint: [task]
disable-model-invocation: true
allowed-tools: ${tools}
---

Task: \`$ARGUMENTS\`

Load the iii agent profile \`${row.id}\` from the running engine, through the
\`iii\` MCP server of the iii plugin. Never answer from memory.

1. Call \`iii_get_agent\` with id \`${row.id}\`. If the engine is unreachable,
   call \`iii_status\` and report its \`error\` and the engine it tried.
2. Read every id in the profile's \`skills\` with \`iii_get_skill\`.
3. From now on, for the rest of this conversation, follow the profile's
   \`system_prompt\` as your instructions, on top of these session rules.
   Confirm in one line which profile is active.
4. If a task was given, start it as that profile now. Otherwise wait for the
   next message.
${ENGINE_NOTE}`,
  };
}

/** Commands for every visible profile; hidden helpers stay reachable through /iii:profile. */
export function profileCommands(agents: AgentRow[]): PluginFile[] {
  return agents.filter((a) => !a.hidden).map(profileCommand);
}

export type CommandOps = {
  read: (path: string) => Promise<string | null>;
  write: (path: string, content: string) => Promise<void>;
  remove: (path: string) => Promise<void>;
};

/** Bring `<root>/commands` in line with `agents`; returns what changed. */
export async function writeProfileCommands(
  root: string,
  agents: AgentRow[],
  ops: CommandOps,
): Promise<{ written: string[]; removed: string[] }> {
  const files = profileCommands(agents);
  const manifestPath = `${root}/${COMMANDS_MANIFEST}`;
  let previous: string[] = [];
  try {
    const parsed = JSON.parse((await ops.read(manifestPath)) ?? '[]');
    if (Array.isArray(parsed)) previous = parsed.filter((p) => typeof p === 'string');
  } catch {
    previous = [];
  }
  const written: string[] = [];
  for (const f of files) {
    const path = `${root}/${f.path}`;
    if ((await ops.read(path)) !== f.content) {
      await ops.write(path, f.content);
      written.push(f.path);
    }
  }
  const current = new Set(files.map((f) => f.path));
  const removed: string[] = [];
  for (const path of previous) {
    // Only ever paths this module generated, and only inside commands/.
    if (current.has(path) || !/^commands\/[a-z0-9-]+\.md$/.test(path)) continue;
    await ops.remove(`${root}/${path}`);
    removed.push(path);
  }
  const manifest = `${JSON.stringify([...current].sort(), null, 2)}\n`;
  if ((await ops.read(manifestPath)) !== manifest) await ops.write(manifestPath, manifest);
  return { written, removed };
}
