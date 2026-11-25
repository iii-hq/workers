/**
 * scripts/build-iii-plugin.mjs: the worker's copy of the iii plugin comes from
 * iii-hq/agent-plugins-claude, fetched at build (or III_PLUGIN_SOURCE).
 */

import { execFileSync } from 'node:child_process';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  generate,
  PLUGIN_REPO,
  resolveSource,
  SHARED_MODULES,
} from '../scripts/build-iii-plugin.mjs';

/** A checkout of the plugin repo, as far as the worker reads it. */
async function fixture(): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), 'iii-plugin-source-'));
  for (const path of ['.claude-plugin/plugin.json', '.mcp.json', 'hooks/hooks.json']) {
    await mkdir(dirname(join(dir, 'plugins/iii', path)), { recursive: true });
    await writeFile(join(dir, 'plugins/iii', path), `content of ${path}`);
  }
  await mkdir(join(dir, 'src'), { recursive: true });
  for (const name of SHARED_MODULES)
    await writeFile(join(dir, 'src', name), `export const name = '${name}';\n`);
  return dir;
}

const out = () => mkdtemp(join(tmpdir(), 'iii-plugin-out-'));

describe('the plugin source', () => {
  it('is the latest default branch of iii-hq/agent-plugins-claude', () => {
    expect(PLUGIN_REPO).toBe('https://github.com/iii-hq/agent-plugins-claude');
    expect(SHARED_MODULES).toEqual([
      'commands.ts',
      'directory.ts',
      'mcp.ts',
      'session.ts',
      'mode.ts',
    ]);
  });

  it('is a local checkout when III_PLUGIN_SOURCE is set', async () => {
    const dir = await fixture();
    const source = await resolveSource({ III_PLUGIN_SOURCE: dir });
    expect(source.dir).toBe(dir);
    expect(source.commit).toBe(`local:${dir}`);
  });

  it('names III_PLUGIN_SOURCE when the local checkout is not one', async () => {
    await expect(resolveSource({ III_PLUGIN_SOURCE: '/nonexistent/plugins' })).rejects.toThrow(
      /III_PLUGIN_SOURCE/,
    );
  });

  it('clones the repository shallowly and records the commit it got', async () => {
    const dir = await fixture();
    const git = (...args: string[]) => execFileSync('git', args, { cwd: dir, stdio: 'pipe' });
    git('init', '-q', '-b', 'main');
    git('add', '-A');
    git('-c', 'user.name=t', '-c', 'user.email=t@t', 'commit', '-q', '-m', 'x');
    const sha = git('rev-parse', 'HEAD').toString().trim();
    const source = await resolveSource({}, `file://${dir}`);
    try {
      expect(source.commit).toBe(sha);
      expect(await readFile(join(source.dir, 'plugins/iii/.mcp.json'), 'utf8')).toBe(
        'content of .mcp.json',
      );
    } finally {
      await source.cleanup?.();
    }
  });

  it('fails a build it cannot fetch, saying how to build offline', async () => {
    await expect(resolveSource({}, 'file:///nonexistent/agent-plugins-claude')).rejects.toThrow(
      /could not fetch .*III_PLUGIN_SOURCE/s,
    );
  });
});

describe('generate', () => {
  it('records the source it came from, and embeds none of the plugin files', async () => {
    // Claude Code installs the published plugin itself (`claude plugin
    // install iii@iii`): the worker carries no copy of it.
    const dir = await fixture();
    const root = await out();
    await generate({ dir, commit: 'abc123' }, root);
    const source = await readFile(join(root, 'src/iii-plugin/source.generated.ts'), 'utf8');
    expect(source).toContain(
      `export const PLUGIN_SOURCE = ${JSON.stringify({ repo: PLUGIN_REPO, commit: 'abc123' })};`,
    );
    expect(source).not.toContain('content of');
    await expect(readFile(join(root, 'src/iii-plugin/assets.generated.ts'))).rejects.toThrow();
  });

  it('vendors the shared modules into src/iii-plugin/shared, marked as generated', async () => {
    const dir = await fixture();
    const root = await out();
    await mkdir(join(root, 'src/iii-plugin/shared'), { recursive: true });
    await writeFile(join(root, 'src/iii-plugin/shared/gone.ts'), 'stale');
    await generate({ dir, commit: 'abc123' }, root);
    for (const name of SHARED_MODULES) {
      const text = await readFile(join(root, 'src/iii-plugin/shared', name), 'utf8');
      expect(text.split('\n')[0]).toMatch(/^\/\/ Generated .*do not edit/);
      expect(text).toContain('git-ignored');
      expect(text).toContain('abc123');
      expect(text).toContain(`export const name = '${name}';`);
    }
    // The folder is the fetched modules exactly: nothing left from before.
    await expect(readFile(join(root, 'src/iii-plugin/shared/gone.ts'))).rejects.toThrow();
  });

  it('also vendors every module the shared ones import, however deep', async () => {
    // The plugin's mcp.ts imports troubleshoot.ts, which no list named; a
    // module it imports in turn comes along too.
    const dir = await fixture();
    await writeFile(
      join(dir, 'src/mcp.ts'),
      "import type { X } from './directory.js';\nimport { help } from './troubleshoot.js';\nexport const name = help;\n",
    );
    await writeFile(
      join(dir, 'src/troubleshoot.ts'),
      "export { deep } from './deep.js';\nexport const help = 'h';\n",
    );
    await writeFile(join(dir, 'src/deep.ts'), "export const deep = 'd';\n");
    await writeFile(join(dir, 'src/unused.ts'), "export const unused = 'u';\n");
    const root = await out();
    await generate({ dir, commit: 'abc123' }, root);
    const shared = join(root, 'src/iii-plugin/shared');
    for (const name of ['troubleshoot.ts', 'deep.ts'])
      expect(await readFile(join(shared, name), 'utf8'), name).toContain('export const');
    // Only what the shared modules reach: the plugin's other files stay out.
    await expect(readFile(join(shared, 'unused.ts'))).rejects.toThrow();
  });

  it('refuses a shared module whose local import is missing, naming it', async () => {
    const dir = await fixture();
    await writeFile(join(dir, 'src/mcp.ts'), "import { help } from './troubleshoot.js';\n");
    await expect(generate({ dir, commit: 'abc123' }, await out())).rejects.toThrow(
      /src\/troubleshoot\.ts/,
    );
  });

  it('takes a checkout even if the plugin folder changes shape: it reads only src/', async () => {
    const dir = await fixture();
    await rm(join(dir, 'plugins/iii/hooks/hooks.json'));
    await expect(generate({ dir, commit: 'abc123' }, await out())).resolves.toMatchObject({
      commit: 'abc123',
    });
  });
});
