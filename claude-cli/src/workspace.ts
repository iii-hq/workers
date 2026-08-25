/**
 * Everything the terminal needs before it opens: the CLI installed, the
 * workspace equipped with the iii skills and the engine notes, and the hooks
 * that report what Claude does. All of it runs on the terminal host through
 * the `shell` worker, so this worker never touches a local path.
 */

import type { IIIClient } from 'iii-sdk';
import type { Config } from './config.js';
import { exec, hostRoot, mkdir, probe, readFile, writeFile } from './host.js';
import { NOTES_BEGIN, NOTES_END, engineNotes } from './notes.js';

const INSTALL_CMD = 'curl -fsSL https://claude.ai/install.sh | bash';
const SKILLS_CMD = 'npx -y skills add iii-hq/iii --all -y';
const SKILLS_MARKER = '.iii/skills-installed';

/** Claude Code lifecycle events worth reporting, and the shape each takes. */
const HOOK_EVENTS: [event: string, shape: 'plain' | 'matcher'][] = [
  ['SessionStart', 'plain'],
  ['SessionEnd', 'plain'],
  ['UserPromptSubmit', 'plain'],
  ['Stop', 'plain'],
  ['PreToolUse', 'matcher'],
  ['PostToolUse', 'matcher'],
];

export type Prepared = {
  workspace: string;
  executable: string;
  args: string[];
  env: Record<string, string>;
  /** Empty when the terminal is ready; otherwise why it is not. */
  detail: string;
  /**
   * The `iii` CLI on the terminal host, which is how the hooks reach the bus.
   * Empty means the hooks are installed but mute: the terminal works and
   * nothing reaches `agent::events`. Worth knowing, because it is what breaks
   * first if the worker that owns the terminal moves into a guest of its own.
   */
  bridge: string;
};

export async function prepareWorkspace(iii: IIIClient, config: Config): Promise<Prepared> {
  const workspace = config.workspace_dir || `${await hostRoot(iii)}/claude-cli`;
  // Sessions inherit the engine address, so a worker Claude writes registers
  // against the same engine it is talking to.
  const env = { III_URL: process.env.III_URL ?? 'ws://127.0.0.1:49134' };
  const base = { workspace, args: config.args, env, bridge: '' };
  let detail = '';

  try {
    await mkdir(iii, workspace);
  } catch (err) {
    return { ...base, executable: '', detail: `workspace ${workspace} is unreachable: ${err}` };
  }

  const executable = await resolveExecutable(iii, config);
  if (!executable) {
    detail = config.auto_install
      ? 'claude could not be installed on the terminal host'
      : 'claude is not on the terminal host and auto_install is off';
  }

  let bridge = '';
  if (config.setup_workspace) {
    await installSkills(iii, workspace);
    await writeNotes(iii, workspace);
    bridge = await writeHooks(iii, workspace);
    if (!bridge) {
      const mute =
        'the `iii` CLI is not on the terminal host, so the activity hooks cannot reach the bus: the terminal works, but no turn will reach agent::events';
      console.warn(`claude-cli: ${mute}`);
      detail = detail ? `${detail}; ${mute}` : mute;
    }
  }

  return { ...base, executable, detail, bridge };
}

async function resolveExecutable(iii: IIIClient, config: Config): Promise<string> {
  if (config.executable) return config.executable;
  const found = await probe(iii, 'command -v claude');
  if (found) return found;
  if (!config.auto_install) return '';
  console.log(`claude not found on the terminal host — installing: ${INSTALL_CMD}`);
  try {
    await exec(iii, INSTALL_CMD, { timeoutMs: 10 * 60_000 });
  } catch (err) {
    console.warn(`claude install failed: ${String(err)}`);
    return '';
  }
  // The installer lands in ~/.local/bin, which a non-login shell may not have
  // on PATH yet.
  return (await probe(iii, 'command -v claude')) || (await probe(iii, 'ls ~/.local/bin/claude'));
}

/**
 * The iii skills, from the monorepo that publishes them. Best-effort and once
 * per workspace: the terminal must open even when the skills CLI is
 * unreachable. The workspace gets its own `package.json` first, because the
 * skills CLI installs at "project level" — the nearest manifest — and without
 * one the skills land above the workspace, where Claude does not look.
 */
async function installSkills(iii: IIIClient, workspace: string): Promise<void> {
  if ((await readFile(iii, `${workspace}/${SKILLS_MARKER}`)) !== null) return;
  if ((await readFile(iii, `${workspace}/package.json`)) === null) {
    await writeFile(
      iii,
      `${workspace}/package.json`,
      `${JSON.stringify({ name: 'claude-cli-workspace', private: true, version: '0.0.0' }, null, 2)}\n`,
    );
  }
  try {
    const result = await exec(iii, SKILLS_CMD, { cwd: workspace, timeoutMs: 5 * 60_000 });
    if (result.exit_code !== 0) {
      console.warn(`iii skills install exited ${result.exit_code}: ${result.stderr.slice(0, 400)}`);
      return;
    }
    await writeFile(iii, `${workspace}/${SKILLS_MARKER}`, `${new Date().toISOString()}\n`);
  } catch (err) {
    console.warn(`iii skills install failed: ${String(err)}`);
  }
}

/** The worker's block inside CLAUDE.md; text outside the markers is the operator's. */
async function writeNotes(iii: IIIClient, workspace: string): Promise<void> {
  const engineUrl = process.env.III_URL ?? 'ws://127.0.0.1:49134';
  const block = `${NOTES_BEGIN}\n${engineNotes({ workspace, engineUrl }).trim()}\n${NOTES_END}`;
  const path = `${workspace}/CLAUDE.md`;
  const current = (await readFile(iii, path)) ?? '';
  let next: string;
  if (current.includes(NOTES_BEGIN) && current.includes(NOTES_END)) {
    const start = current.indexOf(NOTES_BEGIN);
    const end = current.indexOf(NOTES_END) + NOTES_END.length;
    next = current.slice(0, start) + block + current.slice(end);
  } else {
    next = current.trim() ? `${block}\n\n${current.trimStart()}` : `${block}\n`;
  }
  if (next !== current) await writeFile(iii, path, next);
}

/**
 * The hooks that turn a terminal turn into `agent::events` frames. Each event
 * posts the hook JSON to `claude-cli::activity` with the `iii` CLI — the bus
 * is the only transport that works whether or not the terminal host is this
 * worker's host, and `"$(cat)"` expands the payload exactly once, so a prompt
 * containing shell syntax is data and not a command.
 *
 * Only this worker's own keys in `.claude/settings.json` are rewritten;
 * everything else in the file is the operator's.
 *
 * Returns the CLI it found, or '' — the hooks are written either way (a CLI
 * installed later then works), but an empty answer means they are mute and the
 * caller says so out loud.
 */
export async function writeHooks(iii: IIIClient, workspace: string): Promise<string> {
  const found = await probe(iii, 'command -v iii');
  const cli = found || 'iii';
  const command = `${cli} trigger claude-cli::activity --json "$(cat)" --timeout-ms 3000 >/dev/null 2>&1 || true`;
  const path = `${workspace}/.claude/settings.json`;

  let settings: Record<string, unknown> = {};
  const current = await readFile(iii, path);
  if (current) {
    try {
      settings = JSON.parse(current) as Record<string, unknown>;
    } catch {
      console.warn(`${path} is not valid JSON — rewriting it`);
    }
  }
  const hooks: Record<string, unknown> = {
    ...((settings.hooks as Record<string, unknown> | undefined) ?? {}),
  };
  for (const [event, shape] of HOOK_EVENTS) {
    const entry = { hooks: [{ type: 'command', command }] };
    hooks[event] = shape === 'matcher' ? [{ matcher: '*', ...entry }] : [entry];
  }
  const next = `${JSON.stringify({ ...settings, hooks }, null, 2)}\n`;
  if (next !== current) await writeFile(iii, path, next);
  return found;
}
