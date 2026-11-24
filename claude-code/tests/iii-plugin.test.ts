/**
 * The published `iii` plugin, installed with Claude Code's own commands in
 * the folder a session runs in (`src/iii-plugin/install.ts`). No copy of the
 * plugin is written anywhere: the worker only asks Claude Code whether
 * `iii@iii` already covers the folder, and installs it (scope local) when not.
 */

import { describe, expect, it } from 'vitest';
import {
  ensurePlugin,
  MARKETPLACE_NAME,
  MARKETPLACE_SOURCE,
  PLUGIN_ID,
  pluginCoverage,
  mcpEnv,
  pluginSourceLine,
  readDefaultMcp,
  resolveWorkerMcp,
} from '../src/iii-plugin/install.js';
import { CLAUDE, fakeClaude } from './_helpers/fake-claude.js';

describe('the plugin the worker installs', () => {
  it('is iii@iii from the iii marketplace, iii-hq/agent-plugins-claude', () => {
    expect(PLUGIN_ID).toBe('iii@iii');
    expect(MARKETPLACE_NAME).toBe('iii');
    expect(MARKETPLACE_SOURCE).toBe('iii-hq/agent-plugins-claude');
  });

  it('names the plugin source of the shared modules in one line at worker start', () => {
    expect(
      pluginSourceLine({ repo: 'https://github.com/iii-hq/agent-plugins-claude', commit: 'abc' }),
    ).toBe(
      'claude-code: iii plugin modules from https://github.com/iii-hq/agent-plugins-claude at abc',
    );
  });
});

describe('pluginCoverage', () => {
  const entry = (over: Record<string, unknown>) => ({
    id: 'iii@iii',
    scope: 'local',
    enabled: true,
    projectPath: '/p/a',
    ...over,
  });

  it('counts a local or project install for this folder, enabled', () => {
    expect(pluginCoverage([entry({})], '/p/a')).toBe('enabled');
    expect(pluginCoverage([entry({ scope: 'project' })], '/p/a')).toBe('enabled');
  });

  it('counts a user install for every folder', () => {
    expect(pluginCoverage([entry({ scope: 'user', projectPath: undefined })], '/p/z')).toBe(
      'enabled',
    );
  });

  it("does not count another folder's install", () => {
    expect(pluginCoverage([entry({ projectPath: '/p/b' })], '/p/a')).toBe('absent');
  });

  it('does not count another plugin, or a disabled one', () => {
    expect(pluginCoverage([entry({ id: 'iii@other' })], '/p/a')).toBe('absent');
    expect(pluginCoverage([entry({ enabled: false })], '/p/a')).toBe('disabled');
    expect(pluginCoverage([entry({ scope: 'user', enabled: false })], '/p/a')).toBe('absent');
  });

  it('reads anything that is not a list as nothing installed', () => {
    expect(pluginCoverage({ plugins: [] }, '/p/a')).toBe('absent');
    expect(pluginCoverage(null, '/p/a')).toBe('absent');
  });
});

describe('ensurePlugin', () => {
  it('installs iii@iii with scope local in the folder, and nothing else', async () => {
    const claude = fakeClaude();
    const status = await ensurePlugin(claude.run, CLAUDE, '/p/a');
    expect(status).toMatchObject({ state: 'installed', folder: '/p/a' });
    expect(claude.claudeCalls()).toEqual([
      'plugin list --json',
      'plugin marketplace list --json',
      `plugin install iii@iii --scope local --json`,
    ]);
    // Every command runs IN the folder: local scope is that folder's.
    for (const run of claude.runs) expect(run.cwd).toBe('/p/a');
    expect(claude.installs).toEqual([
      { id: 'iii@iii', scope: 'local', enabled: true, projectPath: '/p/a' },
    ]);
  });

  it('leaves a folder alone when iii@iii is already installed and enabled for it', async () => {
    const claude = fakeClaude({
      installs: [{ id: 'iii@iii', scope: 'local', enabled: true, projectPath: '/p/a' }],
    });
    const status = await ensurePlugin(claude.run, CLAUDE, '/p/a');
    expect(status.state).toBe('present');
    expect(claude.claudeCalls()).toEqual(['plugin list --json']);
  });

  it('leaves it alone under a user-scope install too', async () => {
    const claude = fakeClaude({ installs: [{ id: 'iii@iii', scope: 'user', enabled: true }] });
    expect((await ensurePlugin(claude.run, CLAUDE, '/p/a')).state).toBe('present');
    expect(claude.claudeCalls()).toEqual(['plugin list --json']);
  });

  it('installs in a folder even when another folder has it', async () => {
    const claude = fakeClaude({
      installs: [{ id: 'iii@iii', scope: 'local', enabled: true, projectPath: '/p/b' }],
    });
    expect((await ensurePlugin(claude.run, CLAUDE, '/p/a')).state).toBe('installed');
    expect(claude.installs.map((i) => i.projectPath)).toEqual(['/p/b', '/p/a']);
  });

  it('adds the marketplace first, only when it is missing', async () => {
    const claude = fakeClaude({ marketplaces: [] });
    const status = await ensurePlugin(claude.run, CLAUDE, '/p/a');
    expect(status.state).toBe('installed');
    expect(claude.claudeCalls()).toEqual([
      'plugin list --json',
      'plugin marketplace list --json',
      'plugin marketplace add iii-hq/agent-plugins-claude --json',
      `plugin install iii@iii --scope local --json`,
    ]);
  });

  it('enables a local install that was disabled in this folder, instead of reinstalling', async () => {
    const claude = fakeClaude({
      installs: [{ id: 'iii@iii', scope: 'local', enabled: false, projectPath: '/p/a' }],
    });
    const status = await ensurePlugin(claude.run, CLAUDE, '/p/a');
    expect(status.state).toBe('enabled');
    expect(claude.claudeCalls()).toEqual([
      'plugin list --json',
      'plugin enable iii@iii --scope local --json',
    ]);
    expect(claude.installs[0].enabled).toBe(true);
  });

  it('installs nothing when the list cannot be read, and says why', async () => {
    const claude = fakeClaude();
    claude.fail.add('plugin list');
    const status = await ensurePlugin(claude.run, CLAUDE, '/p/a');
    expect(status.state).toBe('failed');
    expect(status.detail).toContain('claude plugin list');
    expect(status.detail).toContain('plugin list failed');
    expect(claude.claudeCalls()).toEqual(['plugin list --json']);
  });

  it('installs nothing when the list is not JSON (an older claude)', async () => {
    const claude = fakeClaude();
    claude.garbledList = true;
    const status = await ensurePlugin(claude.run, CLAUDE, '/p/a');
    expect(status.state).toBe('failed');
    expect(status.detail).toContain('not JSON');
    expect(claude.installs).toEqual([]);
  });

  it('reports a failed install with what Claude Code said', async () => {
    const claude = fakeClaude();
    claude.fail.add('plugin install');
    const status = await ensurePlugin(claude.run, CLAUDE, '/p/a');
    expect(status.state).toBe('failed');
    expect(status.detail).toContain('plugin install failed');
    expect(status.detail).toContain('/p/a');
  });

  it('reports a failed marketplace add and does not try to install', async () => {
    const claude = fakeClaude({ marketplaces: [] });
    claude.fail.add('plugin marketplace add');
    const status = await ensurePlugin(claude.run, CLAUDE, '/p/a');
    expect(status.state).toBe('failed');
    expect(status.detail).toContain('marketplace');
    expect(claude.claudeCalls().some((c) => c.startsWith('plugin install'))).toBe(false);
  });

  it('never throws when the runner itself throws', async () => {
    const status = await ensurePlugin(
      async () => {
        throw new Error('S216 host exec error');
      },
      CLAUDE,
      '/p/a',
    );
    expect(status.state).toBe('failed');
    expect(status.detail).toContain('S216');
  });

  it('runs one check at a time per folder: two sessions starting together install once', async () => {
    const claude = fakeClaude();
    const slow = async (program: string, args: string[], cwd: string) => {
      await new Promise((r) => setTimeout(r, 5));
      return claude.run(program, args, cwd);
    };
    const [a, b] = await Promise.all([
      ensurePlugin(slow, CLAUDE, '/p/a'),
      ensurePlugin(slow, CLAUDE, '/p/a'),
    ]);
    expect([a.state, b.state].sort()).toEqual(['installed', 'present']);
    expect(claude.installs).toHaveLength(1);
  });
});

describe('mcpEnv: where one Claude process finds mcp-server', () => {
  // The plugin's `.mcp.json` reads III_MCP_URL from the process environment;
  // nothing is stored, so each process carries its own engine's address.
  it('sets III_MCP_URL (and the released CLI hooks name) and III_CLI', () => {
    expect(mcpEnv('http://127.0.0.1:4111/mcp', '/usr/local/bin/iii')).toEqual({
      III_MCP_URL: 'http://127.0.0.1:4111/mcp',
      III_PLUGIN_MCP_URL: 'http://127.0.0.1:4111/mcp',
      III_CLI: '/usr/local/bin/iii',
    });
  });

  it('leaves III_CLI out when there is no iii CLI, so the hooks fall back to PATH', () => {
    expect(mcpEnv('http://127.0.0.1:3111/mcp', '')).not.toHaveProperty('III_CLI');
  });

  it('never configures the plugin: the install carries no --config', async () => {
    const claude = fakeClaude();
    await ensurePlugin(claude.run, CLAUDE, '/p/a');
    await ensurePlugin(claude.run, CLAUDE, '/p/b');
    for (const call of claude.claudeCalls()) expect(call).not.toContain('--config');
    expect(claude.claudeCalls().some((c) => c.startsWith('plugin configure'))).toBe(false);
  });
});

describe('readDefaultMcp', () => {
  const iiiWith = (answer: () => unknown) => ({ call: async () => answer() }) as never;

  it("is default-mcp's prompt, with its skills to read", async () => {
    const r = await readDefaultMcp(
      iiiWith(() => ({
        id: 'default-mcp',
        system_prompt: 'You are an iii agent.',
        skills: ['iii'],
      })),
    );
    expect(r.text).toContain('You are an iii agent.');
    expect(r.text).toContain('`iii`');
    expect(r.detail).toBe('');
  });

  it('is empty, saying why, when iii-directory does not answer', async () => {
    const r = await readDefaultMcp(
      iiiWith(() => {
        throw new Error('function_not_found');
      }),
    );
    expect(r.text).toBe('');
    expect(r.detail).toContain('function_not_found');
  });

  it('is empty when the profile has no prompt', async () => {
    const r = await readDefaultMcp(iiiWith(() => ({ id: 'default-mcp' })));
    expect(r.text).toBe('');
    expect(r.detail).toContain('no prompt');
  });
});

describe('resolveWorkerMcp', () => {
  type Calls = Record<string, (payload: Record<string, unknown>) => unknown>;
  const fakeIii = (calls: Calls) =>
    ({
      call: async ({
        function_id,
        payload,
      }: {
        function_id: string;
        payload: Record<string, unknown>;
      }) => {
        const fn = calls[function_id];
        if (!fn) throw new Error(`function_not_found: ${function_id}`);
        return fn(payload);
      },
    }) as never;
  const running = {
    'http::status': () => ({ host: '127.0.0.1', port: 3211, url: 'http://127.0.0.1:3211' }),
    'engine::functions::list': (p: Record<string, unknown>) =>
      p.prefix === 'mcp-server::'
        ? { functions: [{ function_id: 'mcp-server::handler' }] }
        : { functions: [] },
  };

  it("uses the http worker's address plus /mcp when mcp-server is on the engine", async () => {
    expect(await resolveWorkerMcp(fakeIii(running), {})).toEqual({
      url: 'http://127.0.0.1:3211/mcp',
      detail: '',
      available: true,
    });
  });

  it('asks http::status every time: a new port shows on the next call', async () => {
    let port = 3211;
    const iii = fakeIii({
      ...running,
      'http::status': () => ({ url: `http://127.0.0.1:${port}` }),
    });
    expect((await resolveWorkerMcp(iii, {})).url).toBe('http://127.0.0.1:3211/mcp');
    port = 4311;
    expect((await resolveWorkerMcp(iii, {})).url).toBe('http://127.0.0.1:4311/mcp');
  });

  it('lets III_PLUGIN_MCP_URL override the address (a custom api_path)', async () => {
    const r = await resolveWorkerMcp(fakeIii(running), {
      III_PLUGIN_MCP_URL: 'http://127.0.0.1:3211/team/mcp',
    });
    expect(r.url).toBe('http://127.0.0.1:3211/team/mcp');
  });

  it('says clearly when mcp-server is not running, and how to add it', async () => {
    const r = await resolveWorkerMcp(
      fakeIii({ ...running, 'engine::functions::list': () => ({ functions: [] }) }),
      {},
    );
    expect(r.url).toBe('http://127.0.0.1:3211/mcp');
    expect(r.available).toBe(false);
    expect(r.detail).toContain('mcp-server is not running on this engine');
    expect(r.detail).toContain('rbac-proxy');
  });

  it('falls back to the default address when the http worker does not answer', async () => {
    const r = await resolveWorkerMcp(
      fakeIii({ 'engine::functions::list': running['engine::functions::list'] }),
      {},
    );
    expect(r.url).toBe('http://127.0.0.1:3111/mcp');
    expect(r.detail).toContain('http::status');
  });
});
