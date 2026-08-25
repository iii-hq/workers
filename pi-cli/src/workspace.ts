/**
 * Everything the terminal needs before it opens: pi installed, the workspace
 * equipped with the iii skills and the engine notes, and the extension that
 * reports what pi does. All of it runs on the terminal host through the
 * `shell` worker, so this worker never touches a local path.
 */

import type { IIIClient } from 'iii-sdk';
import type { Config } from './config.js';
import { EXTENSION_PATH, extensionSource } from './extension.js';
import { exec, hostRoot, mkdir, probe, readFile, writeFile } from './host.js';
import { NOTES_BEGIN, NOTES_END, engineNotes } from './notes.js';

const INSTALL_CMD = 'curl -fsSL https://pi.dev/install.sh | sh';
const SKILLS_CMD = 'npx -y skills add iii-hq/iii --all -y';
const SKILLS_MARKER = '.iii/skills-installed';

export type Prepared = {
  workspace: string;
  executable: string;
  args: string[];
  env: Record<string, string>;
  /** Empty when the terminal is ready; otherwise why it is not. */
  detail: string;
};

export async function prepareWorkspace(iii: IIIClient, config: Config): Promise<Prepared> {
  const workspace = config.workspace_dir || `${await hostRoot(iii)}/pi-cli`;
  // Sessions inherit the engine address, so a worker pi writes registers
  // against the same engine it is talking to.
  const env = { III_URL: process.env.III_URL ?? 'ws://127.0.0.1:49134' };
  const base = { workspace, args: config.args, env };
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

  if (config.setup_workspace) {
    await installSkills(iii, workspace);
    await writeNotes(iii, workspace);
    await writeExtension(iii, workspace);
  }

  return { ...base, executable, detail };
}

async function resolveExecutable(iii: IIIClient, config: Config): Promise<string> {
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

/**
 * The iii skills, from the monorepo that publishes them. Best-effort and once
 * per workspace: the terminal must open even when the skills CLI is
 * unreachable. The workspace gets its own `package.json` first, because the
 * skills CLI installs at "project level" — the nearest manifest — and without
 * one the skills land above the workspace.
 */
async function installSkills(iii: IIIClient, workspace: string): Promise<void> {
  if ((await readFile(iii, `${workspace}/${SKILLS_MARKER}`)) !== null) return;
  if ((await readFile(iii, `${workspace}/package.json`)) === null) {
    await writeFile(
      iii,
      `${workspace}/package.json`,
      `${JSON.stringify({ name: 'pi-cli-workspace', private: true, version: '0.0.0' }, null, 2)}\n`,
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

/** The worker's block inside AGENTS.md; text outside the markers is the operator's. */
async function writeNotes(iii: IIIClient, workspace: string): Promise<void> {
  const engineUrl = process.env.III_URL ?? 'ws://127.0.0.1:49134';
  const block = `${NOTES_BEGIN}\n${engineNotes({ workspace, engineUrl }).trim()}\n${NOTES_END}`;
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
 */
export async function writeExtension(iii: IIIClient, workspace: string): Promise<void> {
  const cli = (await probe(iii, 'command -v iii')) || 'iii';
  const path = `${workspace}/${EXTENSION_PATH}`;
  const next = extensionSource(cli);
  if ((await readFile(iii, path)) !== next) await writeFile(iii, path, next);
}
