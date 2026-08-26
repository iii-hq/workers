/**
 * Everything the terminal needs before it opens: the CLI installed, the
 * workspace equipped with the engine notes (whose iii half comes from
 * `iii-directory`) and the hooks that report what Claude does. All of it runs on the terminal host through
 * the `shell` worker, so this worker never touches a local path.
 */

import type { IIIClient } from 'iii-sdk';
import type { TerminalConfig } from '../config.js';
import { exec, hostRoot, mkdir, probe, quote, readFile, writeFile } from './host.js';
import { fetchIiiContext } from '../iii-context.js';
import { NOTES_BEGIN, NOTES_END, engineNotes } from './notes.js';

const INSTALL_CMD = 'curl -fsSL https://claude.ai/install.sh | bash';
/** Where every hook posts, and therefore how a hook entry is recognised. */
const ACTIVITY_TARGET = 'claude::terminal::activity';

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

export async function prepareWorkspace(iii: IIIClient, config: TerminalConfig): Promise<Prepared> {
  const workspace = config.workspace_dir || `${await hostRoot(iii)}/claude-code`;
  const env = await sessionEnv(iii);
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
    // Equipping the workspace is not what makes a terminal usable: a read-only
    // `CLAUDE.md` or an unreadable `.claude/` still leaves a working CLI in a
    // working directory. A failure here is reported, and the terminal opens.
    try {
      await writeNotes(iii, workspace);
      bridge = await writeHooks(iii, workspace);
      if (!bridge) {
        const mute =
          'the `iii` CLI is not on the terminal host, so the activity hooks cannot reach the bus: the terminal works, but no turn will reach agent::events';
        console.warn(`claude-code: ${mute}`);
        detail = detail ? `${detail}; ${mute}` : mute;
      }
    } catch (err) {
      const failed = `the workspace could not be equipped (${String(err)}); the terminal opens without the iii notes or the activity hooks`;
      console.warn(`claude-code: ${failed}`);
      detail = detail ? `${detail}; ${failed}` : failed;
    }
  }

  return { ...base, executable, detail, bridge };
}

/**
 * What a session runs with, beyond what the terminal host already exports.
 *
 * `III_URL` is the reason a worker Claude writes registers against the engine
 * this worker is talking to.
 *
 * `USER` is the reason it can log in at all. Claude Code keeps its
 * subscription credentials in the OS keychain and looks them up by the current
 * user — and a worker's environment is NOT the operator's shell: compose
 * clears it and re-seeds an allowlist from the daemon's own environment, so a
 * daemon started without `USER` hands every child a blank one. The symptom is
 * a CLI that reports `loggedIn: false` beside a keychain that plainly holds
 * the login. Asking the terminal host who it is costs one call and removes the
 * dependency on how the daemon was started.
 *
 * `COLORFGBG` says light-on-dark. The page paints a dark terminal, an agent
 * TUI picks its palette from this, and without it half the interface can
 * arrive in dark ink on a dark background.
 */
async function sessionEnv(iii: IIIClient): Promise<Record<string, string>> {
  const env: Record<string, string> = {
    III_URL: process.env.III_URL ?? 'ws://127.0.0.1:49134',
    COLORFGBG: '15;0',
  };
  const user = await probe(iii, 'id -un');
  if (user) {
    env.USER = user;
    env.LOGNAME = user;
  } else {
    console.warn(
      'claude-code terminal: the terminal host did not name its user; a keychain login may read as signed out',
    );
  }
  return env;
}

async function resolveExecutable(iii: IIIClient, config: TerminalConfig): Promise<string> {
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

/** The worker's block inside CLAUDE.md; text outside the markers is the operator's. */
async function writeNotes(iii: IIIClient, workspace: string): Promise<void> {
  const engineUrl = process.env.III_URL ?? 'ws://127.0.0.1:49134';
  // The iii half of this block belongs to `iii-directory`; this worker only
  // says where the workspace and the engine are.
  const context = await fetchIiiContext(iii);
  const notes = engineNotes({
    workspace,
    engineUrl,
    context: context.text,
    detail: context.detail,
  });
  const block = `${NOTES_BEGIN}\n${notes.trim()}\n${NOTES_END}`;
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
 * posts the hook JSON to `claude::terminal::activity` with the `iii` CLI — the bus
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
/**
 * True for an entry this worker wrote. The trigger target is the mark — an
 * operator hook never posts to `claude::terminal::activity` — so a stale entry
 * from an earlier boot is recognised whatever `iii` path it was baked with.
 */
function isWorkerHook(entry: unknown): boolean {
  const commands = (entry as { hooks?: { command?: unknown }[] } | null)?.hooks;
  if (!Array.isArray(commands)) return false;
  return commands.some(
    (hook) => typeof hook?.command === 'string' && hook.command.includes(ACTIVITY_TARGET),
  );
}

export async function writeHooks(iii: IIIClient, workspace: string): Promise<string> {
  const found = await probe(iii, 'command -v iii');
  const cli = found || 'iii';
  const command = `${quote(cli)} trigger ${ACTIVITY_TARGET} --json "$(cat)" --timeout-ms 3000 >/dev/null 2>&1 || true`;
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
    const mine = shape === 'matcher' ? { matcher: '*', ...entry } : entry;
    // Only this worker's own entry is rewritten. An operator who hangs a
    // formatter on PostToolUse keeps it: the entries here are appended to what
    // is already registered for the event, and the one dropped is the previous
    // version of this same entry (the `iii` path can move between boots).
    const existing = Array.isArray(hooks[event])
      ? (hooks[event] as unknown[]).filter((item) => !isWorkerHook(item))
      : [];
    hooks[event] = [...existing, mine];
  }
  const next = `${JSON.stringify({ ...settings, hooks }, null, 2)}\n`;
  if (next !== current) await writeFile(iii, path, next);
  return found;
}
