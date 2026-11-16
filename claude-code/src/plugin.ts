/**
 * The iii plugin: one directory that teaches Claude Code what this engine is
 * and reports back what it did.
 *
 * Claude Code loads a local plugin with `--plugin-dir`, and the Agent SDK's
 * `plugins: [{ type: 'local', path }]` becomes exactly that flag — so the same
 * directory serves BOTH halves of this worker. A terminal session a person
 * types into and a headless `claude::run` turn get the same hooks and the same
 * skill, from one description of what the plugin contains.
 *
 * That is why the hooks moved here from `.claude/settings.json`: settings are
 * the operator's file, and rewriting six of its keys on every boot meant
 * merging around whatever else lived there. A plugin is this worker's own
 * directory — it can be written whole, and it carries the skill with it.
 *
 * The skill's text is not written here either. It comes from `iii-directory`
 * (see `iii-context.ts`), so the plugin is a delivery mechanism, never a second
 * copy.
 */

import { approveHookEntry } from './iii-plugin/mode.js';

/** Where the plugin is materialised, relative to a workspace. */
export const PLUGIN_DIR_NAME = '.iii-plugin';

/** The default-mcp prompt, appended to Claude Code's system prompt (an agent
 * would replace it). */
export const MODE_FILE = 'default-mcp.md';

/** Where every hook posts, and therefore how a hook entry is recognised. */
export const ACTIVITY_TARGET = 'claude::terminal::activity';

/** Claude Code lifecycle events worth reporting, and the shape each takes. */
const HOOK_EVENTS: [event: string, shape: 'plain' | 'matcher'][] = [
  ['SessionStart', 'plain'],
  ['SessionEnd', 'plain'],
  ['UserPromptSubmit', 'plain'],
  ['Stop', 'plain'],
  ['PreToolUse', 'matcher'],
  ['PostToolUse', 'matcher'],
];

export type PluginFile = { path: string; content: string };

export type PluginOptions = {
  /** The `iii` CLI on the host that will RUN the agent; the hooks call it. */
  cli: string;
  /** The iii context from `iii-directory`. No skill is written without it. */
  context: string;
  /** Why the context is missing, when it is. */
  contextDetail?: string;
  /**
   * The mcp-server endpoint the session's `iii` MCP server is; empty when the
   * engine runs none, and the session works through the `iii` CLI instead.
   */
  mcpUrl?: string;
  /** The default-mcp prompt, written for `--append-system-prompt-file`. */
  mode?: string;
  /** The `iii` agent built from default-mcp (`agents/iii.md`). */
  agent?: string;
};

/**
 * A shell-safe single-quoted word. The CLI path is discovered per host and can
 * carry a space; a hook command is a shell string, so it has to survive one.
 */
function quote(value: string): string {
  return `'${value.replace(/'/g, `'\\''`)}'`;
}

/**
 * Every file of the plugin, as paths relative to its root. The caller decides
 * how they land — through `shell::*` (the `ide` worker) on the terminal host, or on this
 * worker's own disk for a headless turn.
 */
export function pluginFiles(options: PluginOptions): PluginFile[] {
  // `"$(cat)"` expands the hook payload exactly once, so a prompt containing
  // shell syntax stays data and never becomes a command.
  const command = `${quote(options.cli)} call ${ACTIVITY_TARGET} --json "$(cat)" --timeout-ms 3000 >/dev/null 2>&1 || true`;
  const hooks: Record<string, unknown[]> = {};
  for (const [event, shape] of HOOK_EVENTS) {
    const entry = { hooks: [{ type: 'command', command }] };
    hooks[event] = [shape === 'matcher' ? { matcher: '*', ...entry } : entry];
  }
  // The read-only approvals for the iii tools, after the activity report.
  hooks.PreToolUse.push(approveHookEntry());

  // The manifest and commands are the committed `iii-plugin/` folder (see
  // `iii-plugin/install.ts`); these are the parts only a worker can write:
  // hooks that call this host's `iii` CLI, the mcp-server address, and the
  // runtime skill.
  const files: PluginFile[] = [
    { path: 'hooks/hooks.json', content: `${JSON.stringify({ hooks }, null, 2)}\n` },
  ];
  if (options.mcpUrl) {
    // The committed `.mcp.json` reads the user's `mcp_url` option; a worker
    // knows where mcp-server is, so it writes the address itself.
    const mcp = { mcpServers: { iii: { type: 'http', url: options.mcpUrl } } };
    files.push({ path: '.mcp.json', content: `${JSON.stringify(mcp, null, 2)}\n` });
  }
  if (options.mode?.trim()) {
    files.push({ path: MODE_FILE, content: `${options.mode.trim()}\n` });
  }
  if (options.agent) {
    files.push({ path: 'agents/iii.md', content: options.agent });
  }

  const context = options.context.trim();
  if (context) {
    files.push({
      path: 'skills/iii-runtime/SKILL.md',
      content: `---
name: iii-runtime
description: How to work against the live iii engine this session runs on — discovery through the \`iii\` CLI, the calling rules, and which capabilities are already installed. Read it before calling anything on the bus.
---

${context}
`,
    });
  }
  return files;
}

/**
 * What the plugin cannot deliver, said out loud. A missing skill is a fact the
 * operator should see, not a silent gap in what the agent knows.
 */
export function pluginDetail(options: PluginOptions): string {
  // With mcp-server, the skill is left out on purpose (default-mcp guides).
  if (options.context.trim() || options.mcpUrl) return '';
  return `the iii runtime skill is not in the plugin: ${
    options.contextDetail || 'iii-directory served nothing'
  }`;
}
