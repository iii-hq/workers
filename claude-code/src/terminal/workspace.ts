/**
 * Everything a terminal session needs, in two steps, all of it on the
 * terminal host through the `shell::*` functions of the `ide` worker (this
 * worker never touches a local path for a terminal):
 *
 *   - `prepareHost`, at worker start and on a config change: the CLI
 *     installed, the `iii` CLI the plugin's activity hooks report through,
 *     the user a keychain login is looked up by. Slow on a cold host, and
 *     the same for every session.
 *   - `prepareSession`, right before EVERY terminal start (what
 *     `claude::terminal::describe` runs): the folder the session runs in,
 *     the published `iii` plugin installed there when it is not
 *     (`../iii-plugin/install.ts`), the http worker's address read now, and
 *     the default-mcp prompt written again for `--append-system-prompt-file`.
 *     Nothing a session depends on is decided at worker start any more, so a
 *     moved http worker reaches the very next session.
 */

import type { IIIClient } from 'iii-sdk';
import type { TerminalConfig } from '../config.js';
import {
  ensurePlugin,
  mcpEnv,
  type PluginStatus,
  readDefaultMcp,
  resolveWorkerMcp,
  type Runner,
} from '../iii-plugin/install.js';
import { exec, fileExists, hostRoot, mkdir, probe, writeFile } from './host.js';

const INSTALL_CMD = 'curl -fsSL https://claude.ai/install.sh | bash';

/** The default-mcp prompt, in the worker's own folder on the terminal host. */
export const MODE_FILE = 'default-mcp.md';

export type HostPrepared = {
  /**
   * The worker's own folder on the terminal host (`workspace_dir`, else
   * `claude-code` under the ide worker's root): where it keeps the
   * default-mcp file. Never where a session runs.
   */
  home: string;
  executable: string;
  /** What every session runs with, beyond what the host already exports. */
  env: Record<string, string>;
  /**
   * The `iii` CLI on the terminal host, which is how the plugin's activity
   * hooks reach the bus (passed as `III_CLI`). Empty: the hooks fall back to
   * `iii` on PATH, and nothing may reach `claude::on-agent-event`.
   */
  bridge: string;
  /** Empty when the host is ready; otherwise why it is not. */
  detail: string;
};

export type SessionSpec = {
  program: string;
  args: string[];
  /** The folder the session runs in, resolved (`pwd -P`). */
  cwd: string;
  env: Record<string, string>;
  activity_bridge: string;
  /** Anything wrong that does not stop the session from opening. */
  detail: string;
  /** The plugin check in `cwd`; null when `setup_workspace` is off. */
  plugin: PluginStatus | null;
  /** The compose project's folder: where a session runs when none is asked for. */
  project_dir: string;
};

const join = (...parts: string[]) => parts.filter(Boolean).join('; ');

export async function prepareHost(iii: IIIClient, config: TerminalConfig): Promise<HostPrepared> {
  const home = config.workspace_dir || `${await hostRoot(iii)}/claude-code`;
  const env = await sessionEnv(iii);
  const executable = await resolveExecutable(iii, config);
  let detail = '';
  if (!executable) {
    detail = config.auto_install
      ? 'claude could not be installed on the terminal host'
      : 'claude is not on the terminal host and auto_install is off';
  }
  const bridge = await probe(iii, 'command -v iii');
  if (!bridge && config.setup_workspace) {
    detail = join(
      detail,
      'the `iii` CLI is not on the terminal host, so the plugin activity hooks cannot reach the bus: the terminal works, but no turn will reach claude::on-agent-event',
    );
  }
  return { home, executable, env, bridge, detail };
}

/** `run` for the plugin commands: argv through `shell::exec`, in `cwd`. */
function hostRunner(iii: IIIClient): Runner {
  return (program, args, cwd) => exec(iii, program, { args, cwd, timeoutMs: 5 * 60_000 });
}

/**
 * The folder holding the compose file of the project this worker runs in
 * (the project with a `claude-code` container, else the only/first one), or
 * '' when there is no compose daemon to ask.
 */
export async function composeProjectDir(iii: IIIClient): Promise<string> {
  try {
    const res = await iii.call<
      unknown,
      { projects?: { file?: string; containers?: { container?: string }[] }[] }
    >({ function_id: 'compose::list', payload: {}, timeoutMs: 15_000 });
    const projects = (res?.projects ?? []).filter((p) => typeof p.file === 'string' && p.file);
    const mine =
      projects.find((p) => (p.containers ?? []).some((c) => c.container === 'claude-code')) ??
      projects[0];
    if (!mine?.file) return '';
    const cut = mine.file.lastIndexOf('/');
    return cut > 0 ? mine.file.slice(0, cut) : '/';
  } catch (err) {
    console.warn(`claude-code terminal: compose::list did not answer: ${String(err)}`);
    return '';
  }
}

export async function prepareSession(
  iii: IIIClient,
  config: TerminalConfig,
  host: HostPrepared,
  request: { cwd?: string },
): Promise<SessionSpec> {
  if (!host.executable) {
    throw new Error(host.detail || 'claude is not available on the terminal host');
  }
  const run = hostRunner(iii);
  const project = await composeProjectDir(iii);
  const asked = request.cwd?.trim() || project;
  if (!asked) {
    throw new Error(
      'no folder to run Claude in: pass `cwd`, or run the worker in a compose project (compose::list names none)',
    );
  }
  // The folder must exist on the terminal host, and Claude Code records its
  // local installs by the resolved path, so the session runs in that one.
  const real = await run('pwd', ['-P'], asked).catch((err) => ({
    stdout: '',
    stderr: String(err),
    exit_code: 1,
  }));
  const folder = real.exit_code === 0 ? real.stdout.trim() : '';
  if (!folder) {
    throw new Error(
      `the folder ${asked} is not reachable on the terminal host: ${real.stderr.trim()}`,
    );
  }

  // Read now, for this session: http::status, then mcp-server's presence.
  const mcp = await resolveWorkerMcp(iii);
  const env = { ...host.env, ...mcpEnv(mcp.url, host.bridge) };
  const args = [...config.args];
  let detail = mcp.detail;
  let plugin: PluginStatus | null = null;

  if (config.setup_workspace) {
    plugin = await ensurePlugin(run, host.executable, folder);
    if (plugin.detail) detail = join(detail, plugin.detail);
    // default-mcp is appended to Claude Code's own prompt (an agent would
    // replace it); an append the operator configured is kept, ours left out.
    if (mcp.available) {
      if (config.args.some((a) => a.startsWith('--append-system-prompt'))) {
        detail = join(
          detail,
          'default-mcp was not appended: the terminal args already carry an --append-system-prompt',
        );
      } else {
        const appended = await writeDefaultMcp(iii, host.home);
        if (appended.file) args.push('--append-system-prompt-file', appended.file);
        else detail = join(detail, appended.detail);
      }
    }
  }
  return {
    program: host.executable,
    args,
    cwd: folder,
    env,
    activity_bridge: host.bridge,
    detail,
    plugin,
    project_dir: project,
  };
}

/**
 * Write the default-mcp prompt to the worker's folder and check it is there:
 * the session is never launched with a path to a missing file.
 */
async function writeDefaultMcp(
  iii: IIIClient,
  home: string,
): Promise<{ file: string; detail: string }> {
  const profile = await readDefaultMcp(iii);
  if (!profile.text) return { file: '', detail: `default-mcp was not appended: ${profile.detail}` };
  const file = `${home}/${MODE_FILE}`;
  try {
    await mkdir(iii, home);
    await writeFile(iii, file, `${profile.text.trim()}\n`);
  } catch (err) {
    return {
      file: '',
      detail: `default-mcp was not appended: ${file} could not be written (${String(err)})`,
    };
  }
  if ((await fileExists(iii, file)) !== true) {
    return {
      file: '',
      detail: `default-mcp was not appended: ${file} is not there after writing it`,
    };
  }
  return { file, detail: '' };
}

/**
 * What a session runs with, beyond what the terminal host already exports.
 *
 * `III_URL` is the engine this worker talks to: the `iii` CLI the plugin's
 * hooks run, and any worker Claude writes, connect there.
 *
 * `USER` is the reason it can log in at all. Claude Code keeps its
 * subscription credentials in the OS keychain and looks them up by the current
 * user, and a worker's environment is NOT the operator's shell: compose clears
 * it and re-seeds an allowlist, so a daemon started without `USER` hands every
 * child a blank one. Asking the terminal host who it is costs one call.
 *
 * `COLORFGBG` says light-on-dark: the page paints a dark terminal.
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
    await exec(iii, 'bash', { args: ['-c', INSTALL_CMD], timeoutMs: 10 * 60_000 });
  } catch (err) {
    console.warn(`claude install failed: ${String(err)}`);
    return '';
  }
  // The installer lands in ~/.local/bin, which a non-login shell may not have
  // on PATH yet.
  return (await probe(iii, 'command -v claude')) || (await probe(iii, 'ls ~/.local/bin/claude'));
}
