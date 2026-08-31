/**
 * The plugin, materialised on THIS worker's disk for a headless turn.
 *
 * The terminal half writes the same files onto the terminal host through the
 * `shell` worker; a headless turn runs here, in this process, so the SDK needs
 * a path it can read locally. Same description (`plugin.ts`), same hooks, same
 * skill — two hosts, one definition.
 *
 * It lives under the OS temp directory rather than the workspace: a headless
 * turn's `cwd` is the caller's repository, and this worker has no business
 * leaving a directory in it.
 */

import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import type { IIIClient } from 'iii-sdk';
import { fetchIiiContext } from './iii-context.js';
import { pluginDetail, pluginFiles } from './plugin.js';

const ROOT = join(tmpdir(), 'iii-claude-code-plugin');

/**
 * Write the plugin and return its path, or '' when it could not be written.
 *
 * Best-effort by design: a turn that cannot have its hooks is still a turn, and
 * the alternative — failing the run over a plugin directory — trades an agent
 * for a log line.
 */
export async function localPluginDir(iii: IIIClient): Promise<{ dir: string; detail: string }> {
  try {
    const context = await fetchIiiContext(iii);
    const options = {
      cli: process.env.III_CLI ?? 'iii',
      context: context.text,
      contextDetail: context.detail,
    };
    for (const file of pluginFiles(options)) {
      const path = join(ROOT, file.path);
      const current = await readFile(path, 'utf8').catch(() => null);
      if (current === file.content) continue;
      await mkdir(dirname(path), { recursive: true });
      await writeFile(path, file.content, 'utf8');
    }
    return { dir: ROOT, detail: pluginDetail(options) };
  } catch (err) {
    const detail = `the iii plugin could not be written to ${ROOT}: ${String(err)}`;
    console.warn(`claude-code: ${detail}`);
    return { dir: '', detail };
  }
}
