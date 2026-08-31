/**
 * Everything the terminal needs before it opens: the CLI installed, the
 * workspace equipped with the engine notes (whose iii half comes from
 * `iii-directory`) and the hooks that report what Claude does. All of it runs on the terminal host through
 * the `shell` worker, so this worker never touches a local path.
 */

import type { IIIClient } from 'iii-sdk';
import type { TerminalConfig } from '../config.js';
import { PLUGIN_DIR_NAME, pluginDetail, pluginFiles } from '../plugin.js';
import { exec, hostRoot, mkdir, probe, readFile, writeFile } from './host.js';
import { fetchIiiContext } from '../iii-context.js';
import { NOTES_BEGIN, NOTES_END, engineNotes } from './notes.js';

const INSTALL_CMD = 'curl -fsSL https://claude.ai/install.sh | bash';

export type Prepared = {
  workspace: string;
  executable: string;
  args: string[];
  env: Record<string, string>;
  /** Empty when the terminal is ready; otherwise why it is not. */
  detail: string;
  /** The plugin directory a session loads with `--plugin-dir`. */
  plugin: string;
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
  const base = { workspace, args: config.args, env, bridge: '', plugin: '' };
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
  let plugin = '';
  if (config.setup_workspace) {
    // Equipping the workspace is not what makes a terminal usable: a read-only
    // `CLAUDE.md` or an unreadable `.claude/` still leaves a working CLI in a
    // working directory. A failure here is reported, and the terminal opens.
    try {
      const context = await fetchIiiContext(iii);
      await writeNotes(iii, workspace, context);
      const written = await writePlugin(iii, workspace, context);
      bridge = written.bridge;
      plugin = written.dir;
      if (written.detail) {
        console.warn(`claude-code: ${written.detail}`);
        detail = detail ? `${detail}; ${written.detail}` : written.detail;
      }
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

  // The plugin is how a session gets its hooks and the iii skill; the flag is
  // the same one the Agent SDK emits for a headless turn.
  const args = plugin ? [...config.args, '--plugin-dir', plugin] : config.args;
  return { ...base, args, executable, detail, bridge, plugin };
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
async function writeNotes(
  iii: IIIClient,
  workspace: string,
  context: { text: string; detail: string },
): Promise<void> {
  const engineUrl = process.env.III_URL ?? 'ws://127.0.0.1:49134';
  // The iii half of this block belongs to `iii-directory`; this worker only
  // says where the workspace and the engine are.
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
 * The iii plugin, materialised in the workspace.
 *
 * Claude Code loads it with `--plugin-dir`, which is also what the Agent SDK
 * emits for a local plugin — so this is the same directory a headless turn
 * gets, from the same description (`../plugin.ts`). The hooks that report
 * activity live in it, which is why nothing writes `.claude/settings.json` any
 * more: that file is the operator's, and a plugin directory is this worker's.
 *
 * Returns the CLI it found, or '' — the plugin is written either way (a CLI
 * installed later then works), but an empty answer means the hooks are mute and
 * the caller says so out loud.
 */
export async function writePlugin(
  iii: IIIClient,
  workspace: string,
  context: { text: string; detail: string },
): Promise<{ bridge: string; dir: string; detail: string }> {
  const found = await probe(iii, 'command -v iii');
  const dir = `${workspace}/${PLUGIN_DIR_NAME}`;
  const options = { cli: found || 'iii', context: context.text, contextDetail: context.detail };
  for (const file of pluginFiles(options)) {
    const path = `${dir}/${file.path}`;
    // Written whole, every boot: the plugin is this worker's directory, so
    // there is no operator content in it to merge around.
    if ((await readFile(iii, path)) !== file.content) await writeFile(iii, path, file.content);
  }
  return { bridge: found, dir, detail: pluginDetail(options) };
}
