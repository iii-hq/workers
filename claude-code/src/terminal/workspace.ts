/**
 * Everything the terminal needs before it opens: the CLI installed and the
 * workspace equipped with the iii plugin (the hooks that report what Claude
 * does, the iii skills and commands). All of it runs on the terminal host
 * through the `shell::*` functions of the `ide` worker, so this worker never
 * touches a local path.
 */

import type { IIIClient } from 'iii-sdk';
import type { TerminalConfig } from '../config.js';
import { installOnHost } from '../iii-plugin/install.js';
import { MODE_FILE } from '../plugin.js';
import {
  exec,
  fileExists,
  hostRoot,
  mkdir,
  probe,
  readFile,
  removeFile,
  writeFile,
} from './host.js';
import { fetchIiiContext } from '../iii-context.js';

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
   * nothing reaches `claude::on-agent-event`. Worth knowing, because it is what breaks
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
  let mode = false;
  if (config.setup_workspace) {
    // Equipping the workspace is not what makes a terminal usable: a read-only
    // `CLAUDE.md` or an unreadable `.claude/` still leaves a working CLI in a
    // working directory. A failure here is reported, and the terminal opens.
    try {
      const context = await fetchIiiContext(iii);
      await removeLegacyNotes(iii, workspace);
      // The `iii` plugin: activity hooks, the runtime skill, the engine's
      // mcp-server as its MCP server, and the /iii:* commands
      // (../iii-plugin/install.ts).
      const written = await installOnHost(iii, workspace, context);
      bridge = written.bridge;
      plugin = written.dir;
      mode = Boolean(written.mode);
      if (written.detail) {
        console.warn(`claude-code: ${written.detail}`);
        detail = detail ? `${detail}; ${written.detail}` : written.detail;
      }
      if (!bridge) {
        const mute =
          'the `iii` CLI is not on the terminal host, so the activity hooks cannot reach the bus: the terminal works, but no turn will reach claude::on-agent-event';
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
  const args = plugin ? [...config.args, '--plugin-dir', plugin] : [...config.args];
  // default-mcp (only with mcp-server) is appended to Claude Code's
  // own system prompt. An agent (`--agent`) would replace that prompt. An
  // append the operator configured is kept, and ours is left out.
  if (plugin && mode) {
    const ownAppend = config.args.some((a) => a.startsWith('--append-system-prompt'));
    if (ownAppend) {
      const skipped =
        'default-mcp was not appended: the terminal args already carry an --append-system-prompt';
      console.warn(`claude-code: ${skipped}`);
      detail = detail ? `${detail}; ${skipped}` : skipped;
    } else {
      args.push('--append-system-prompt-file', `${plugin}/${MODE_FILE}`);
    }
  }
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

/** The block earlier versions kept in the workspace's CLAUDE.md. */
const LEGACY_BEGIN = '<!-- iii:begin';
const LEGACY_END = '<!-- iii:end -->';

/**
 * Earlier versions kept a marked block in the workspace's `CLAUDE.md`. What it
 * said now comes from the plugin (the `iii-runtime` skill, default-mcp
 * appended), and what it got wrong (the `shell` worker, the `iii` CLI as the
 * only way in) contradicted default-mcp, so the block is taken out. Text
 * outside the markers is the operator's and stays; a file left empty is
 * removed. A read that fails for any reason but "absent" throws: the file is
 * left alone and the caller reports it.
 */
async function removeLegacyNotes(iii: IIIClient, workspace: string): Promise<void> {
  const path = `${workspace}/CLAUDE.md`;
  // Unknown (null) is read anyway: only a sure "absent" skips the read.
  if ((await fileExists(iii, path)) === false) return;
  const current = await readFile(iii, path);
  if (current === null) return;
  const start = current.indexOf(LEGACY_BEGIN);
  const end = start < 0 ? -1 : current.indexOf(LEGACY_END, start);
  if (end < 0) return;
  const before = current.slice(0, start).trim();
  const after = current.slice(end + LEGACY_END.length).trim();
  const rest = [before, after].filter(Boolean).join('\n\n');
  if (rest) await writeFile(iii, path, `${rest}\n`);
  else await removeFile(iii, path);
}
