/**
 * Everything the terminal needs before it opens: pi installed, the workspace
 * equipped with the engine notes (whose iii half comes from `iii-directory`)
 * and the extension that reports what pi does. All of it runs on the terminal host through the
 * `shell` worker, so this worker never touches a local path.
 */

import type { IIIClient } from 'iii-sdk';
import type { TerminalConfig } from '../config.js';
import { EXTENSION_PATH, extensionSource } from './extension.js';
import { exec, hostRoot, mkdir, probe, readFile, writeFile } from './host.js';
import { fetchIiiContext } from '../iii-context.js';
import { NOTES_BEGIN, NOTES_END, engineNotes } from './notes.js';

const INSTALL_CMD = 'curl -fsSL https://pi.dev/install.sh | sh';

export type Prepared = {
  workspace: string;
  executable: string;
  args: string[];
  env: Record<string, string>;
  /** Empty when the terminal is ready; otherwise why it is not. */
  detail: string;
  /**
   * The `iii` CLI on the terminal host, which is how the extension reaches the
   * bus. Empty means the extension is installed but mute: the terminal works
   * and nothing reaches `agent::events`. Worth knowing, because it is what
   * breaks first if the worker that owns the terminal moves into a guest of
   * its own.
   */
  bridge: string;
};

export async function prepareWorkspace(iii: IIIClient, config: TerminalConfig): Promise<Prepared> {
  const workspace = config.workspace_dir || `${await hostRoot(iii)}/pi`;
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
      ? 'pi could not be installed on the terminal host'
      : 'pi is not on the terminal host and auto_install is off';
  }

  let bridge = '';
  if (config.setup_workspace) {
    // Equipping the workspace is not what makes a terminal usable: a read-only
    // `AGENTS.md` or an unwritable `.pi/` still leaves a working CLI in a
    // working directory. A failure here is reported, and the terminal opens.
    try {
      await writeNotes(iii, workspace);
      bridge = await writeExtension(iii, workspace);
      if (!bridge) {
        const mute =
          'the `iii` CLI is not on the terminal host, so the activity extension cannot reach the bus: the terminal works, but no run will reach agent::events';
        console.warn(`pi terminal: ${mute}`);
        detail = detail ? `${detail}; ${mute}` : mute;
      }
    } catch (err) {
      const failed = `the workspace could not be equipped (${String(err)}); the terminal opens without the iii notes or the activity extension`;
      console.warn(`pi terminal: ${failed}`);
      detail = detail ? `${detail}; ${failed}` : failed;
    }
  }

  return { ...base, executable, detail, bridge };
}

/**
 * What a session runs with, beyond what the terminal host already exports.
 *
 * `III_URL` is the reason a worker pi writes registers against the engine this
 * worker is talking to.
 *
 * `USER` is the reason a provider login is found at all: credentials live
 * under the current user, and a worker's environment is NOT the operator's
 * shell — compose clears it and re-seeds an allowlist from the daemon's own
 * environment, so a daemon started without `USER` hands every child a blank
 * one. Asking the terminal host who it is costs one call and removes the
 * dependency on how the daemon was started.
 *
 * `COLORFGBG` says light-on-dark. The page paints a dark terminal and pi
 * picks its palette from this; without it, parts of its interface arrive in
 * dark ink on a dark background.
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
      'pi terminal: the terminal host did not name its user; a stored provider login may read as absent',
    );
  }
  return env;
}

async function resolveExecutable(iii: IIIClient, config: TerminalConfig): Promise<string> {
  if (config.executable) return config.executable;
  const found = await probe(iii, 'command -v pi');
  if (found) return found;
  if (!config.auto_install) return '';
  console.log(`pi not found on the terminal host — installing: ${INSTALL_CMD}`);
  try {
    await exec(iii, INSTALL_CMD, { timeoutMs: 10 * 60_000 });
  } catch (err) {
    console.warn(`pi install failed: ${String(err)}`);
    return '';
  }
  // The installer lands wherever npm's global bin is, which a non-login shell
  // may not have on PATH yet.
  return (await probe(iii, 'command -v pi')) || (await probe(iii, 'ls ~/.local/bin/pi'));
}

/** The worker's block inside AGENTS.md; text outside the markers is the operator's. */
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
  const path = `${workspace}/AGENTS.md`;
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
 * pi auto-discovers `.pi/extensions/*.ts` in its working directory, so the
 * extension is written in rather than passed with `-e`: a session an operator
 * starts by hand in the same workspace then reports its turns too. The `iii`
 * CLI path is resolved on the terminal host and baked in, because the
 * extension runs there.
 *
 * Returns the CLI it found, or '' — the extension is written either way (a CLI
 * installed later then works), but an empty answer means it is mute and the
 * caller says so out loud.
 */
export async function writeExtension(iii: IIIClient, workspace: string): Promise<string> {
  const found = await probe(iii, 'command -v iii');
  const path = `${workspace}/${EXTENSION_PATH}`;
  const next = extensionSource(found || 'iii');
  if ((await readFile(iii, path)) !== next) await writeFile(iii, path, next);
  return found;
}
