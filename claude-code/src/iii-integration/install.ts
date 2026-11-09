/**
 * The iii-integration plugin, installed wherever Claude Code runs.
 *
 * The worker carries the whole plugin inside its bundle (`assets.generated.ts`,
 * written by `scripts/build-iii-integration.mjs`) and writes it out itself, so
 * no operator step and no file beside the bundle is needed:
 *
 *   - headless `claude::run` turns run in this process: the plugin goes under
 *     the OS temp directory (never the caller's repository);
 *   - terminal sessions run on the `shell` worker's host, which need not be
 *     this machine: the plugin goes into the session workspace through
 *     `shell::fs::write`, like the hooks plugin beside it.
 *
 * A `.build-id` file (a hash of every file) makes a re-install a single small
 * read when nothing changed, rather than re-reading the 500 KB server.
 */

import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import type { IIIClient } from 'iii-sdk';
import { readFile as hostRead, writeFile as hostWrite } from '../terminal/host.js';
import { INTEGRATION_FILES } from './assets.generated.js';

export const INTEGRATION_NAME = 'iii-integration';
/** Where it lands inside a terminal workspace. */
export const INTEGRATION_DIR_NAME = `.${INTEGRATION_NAME}`;
const LOCAL_ROOT = join(tmpdir(), INTEGRATION_NAME);
const MARKER = '.build-id';

export type PluginFile = { path: string; content: string };
export type Installed = { dir: string; detail: string };

export function buildId(files: PluginFile[] = INTEGRATION_FILES): string {
  const hash = createHash('sha256');
  for (const f of files) hash.update(f.path).update('\0').update(f.content).update('\0');
  return hash.digest('hex').slice(0, 16);
}

function missing(files: PluginFile[]): string {
  return files.some((f) => f.path === 'server/index.mjs')
    ? ''
    : `the ${INTEGRATION_NAME} plugin was not embedded in this build (run \`npm run build:iii-integration\`); agent profiles and skills are not available over MCP`;
}

/** Write `files` through `read`/`write` unless the marker already matches. */
export async function installWith(
  dir: string,
  read: (path: string) => Promise<string | null>,
  write: (path: string, content: string) => Promise<void>,
  files: PluginFile[] = INTEGRATION_FILES,
): Promise<Installed> {
  const detail = missing(files);
  if (detail) return { dir: '', detail };
  const id = buildId(files);
  if ((await read(`${dir}/${MARKER}`))?.trim() === id) return { dir, detail: '' };
  for (const f of files) await write(`${dir}/${f.path}`, f.content);
  // Last, so an interrupted install is retried rather than trusted.
  await write(`${dir}/${MARKER}`, `${id}\n`);
  return { dir, detail: '' };
}

/**
 * The plugin on this worker's own disk, for headless turns. Best-effort, and
 * re-checked every turn (one small read) so a cleaned temp dir is restored.
 */
export async function installLocal(): Promise<Installed> {
  try {
    return await installWith(
      LOCAL_ROOT,
      (path) => readFile(path, 'utf8').catch(() => null),
      async (path, content) => {
        await mkdir(dirname(path), { recursive: true });
        await writeFile(path, content, 'utf8');
      },
    );
  } catch (err) {
    return {
      dir: '',
      detail: `the ${INTEGRATION_NAME} plugin could not be written to ${LOCAL_ROOT}: ${String(err)}`,
    };
  }
}

/** The plugin inside a terminal workspace on the shell worker's host. */
export async function installOnHost(iii: IIIClient, workspace: string): Promise<Installed> {
  const dir = `${workspace}/${INTEGRATION_DIR_NAME}`;
  try {
    return await installWith(
      dir,
      (path) => hostRead(iii, path),
      (path, content) => hostWrite(iii, path, content),
    );
  } catch (err) {
    return {
      dir: '',
      detail: `the ${INTEGRATION_NAME} plugin could not be installed in ${dir}: ${String(err)}`,
    };
  }
}
