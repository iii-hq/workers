import { mkdtemp, readdir, readFile as fsRead } from 'node:fs/promises';
import { tmpdir as osTmp } from 'node:os';
import { join as pjoin } from 'node:path';
import { describe, expect, it } from 'vitest';
import { PLUGIN_SOURCE, PLUGIN_STATIC_FILES } from '../src/iii-plugin/assets.generated.js';
import {
  atomicWrite,
  buildId,
  installWith,
  pluginSourceLine,
  resolveWorkerMcp,
} from '../src/iii-plugin/install.js';
import { approveHookEntry } from '../src/iii-plugin/shared/mode.js';
import { pluginFiles } from '../src/plugin.js';

function memoryOps() {
  const disk: Record<string, string> = {};
  const writes: string[] = [];
  const removes: string[] = [];
  return {
    disk,
    writes,
    removes,
    ops: {
      read: async (p: string) => disk[p] ?? null,
      write: async (p: string, c: string) => {
        writes.push(p);
        disk[p] = c;
      },
      remove: async (p: string) => {
        removes.push(p);
        delete disk[p];
      },
    },
  };
}

describe('the embedded plugin', () => {
  it('comes from iii-hq/agent-plugins-claude, at the commit the build fetched', () => {
    expect(PLUGIN_SOURCE.repo).toBe('https://github.com/iii-hq/agent-plugins-claude');
    expect(PLUGIN_SOURCE.commit).toMatch(/^([0-9a-f]{40}|local:.+)$/);
  });

  it('is named in one line at worker start', () => {
    expect(
      pluginSourceLine({
        repo: 'https://github.com/iii-hq/agent-plugins-claude',
        commit: 'abc123',
      }),
    ).toBe('claude-code: iii plugin from https://github.com/iii-hq/agent-plugins-claude at abc123');
    expect(pluginSourceLine()).toContain(PLUGIN_SOURCE.commit);
  });
});

describe('worker plugin files', () => {
  const files = (mcpUrl: string) => pluginFiles({ cli: 'iii', context: '', mcpUrl });
  const file = (mcpUrl: string, path: string) =>
    files(mcpUrl).find((f) => f.path === path)?.content ?? '';

  it('carry the read-only approval hook after the activity hook', () => {
    const hooks = JSON.parse(file('http://x/mcp', 'hooks/hooks.json') || '{}').hooks;
    expect(hooks.PreToolUse[0].hooks[0].command).toContain('claude::terminal::activity');
    expect(hooks.PreToolUse[1]).toEqual(approveHookEntry());
  });

  it('point the session at the mcp-server the worker found', () => {
    expect(JSON.parse(file('http://127.0.0.1:3211/mcp', '.mcp.json') || '{}')).toEqual({
      mcpServers: { iii: { type: 'http', url: 'http://127.0.0.1:3211/mcp' } },
    });
  });

  it('have no session-start hook: the worker appends default-mcp itself', () => {
    expect(file('http://127.0.0.1:3211/mcp', 'hooks/hooks.json')).not.toContain('session-start');
  });
});

describe('installer', () => {
  const embedded = [
    { path: '.claude-plugin/plugin.json', content: '{"name":"iii"}' },
    { path: '.mcp.json', content: '{}' },
    { path: 'hooks/hooks.json', content: 'standalone sync hook' },
    { path: 'commands/profile.md', content: 'p' },
    { path: 'cli/index.mjs', content: 'x' },
  ];
  const options = {
    cli: '/usr/bin/iii',
    context: { text: '# iii runtime', detail: '' },
    agents: [{ id: 'writer' }],
    mcp: { url: 'http://127.0.0.1:3111/mcp', detail: '', available: true },
    defaultProfile: { id: 'default-mcp', system_prompt: 'The default-mcp prompt.' },
  };

  it('ships one plugin named iii inside the worker build', () => {
    const paths = PLUGIN_STATIC_FILES.map((f) => f.path);
    expect(paths).toEqual(
      expect.arrayContaining([
        '.claude-plugin/plugin.json',
        '.mcp.json',
        'commands/profile.md',
        'commands/skills.md',
        'hooks/hooks.json',
        'cli/index.mjs',
      ]),
    );
    // No MCP server of its own: the plugin uses the mcp-server worker.
    expect(paths.filter((p) => p.startsWith('server/'))).toEqual([]);
    const manifest = PLUGIN_STATIC_FILES.find((f) => f.path === '.claude-plugin/plugin.json');
    expect(JSON.parse(manifest?.content ?? '{}').name).toBe('iii');
    // A `commands/skill.md` silently disables every command in the plugin.
    expect(paths).not.toContain('commands/skill.md');
    // Under the ide worker's common 1 MiB per-file cap.
    for (const f of PLUGIN_STATIC_FILES)
      expect(Buffer.byteLength(f.content)).toBeLessThan(1024 * 1024);
  });

  it('writes static files, the worker hooks and skill, and the profile commands', async () => {
    const m = memoryOps();
    expect(await installWith('/w/p', m.ops, options, embedded)).toEqual({
      dir: '/w/p',
      detail: '',
      mode: 'The default-mcp prompt.',
      mcp: true,
    });
    expect(m.disk['/w/p/cli/index.mjs']).toBe('x');
    // The worker points the session at the mcp-server it found.
    expect(JSON.parse(m.disk['/w/p/.mcp.json'])).toEqual({
      mcpServers: { iii: { type: 'http', url: 'http://127.0.0.1:3111/mcp' } },
    });
    expect(m.disk['/w/p/.build-id'].trim()).toBe(
      // The worker writes its own hooks and .mcp.json.
      buildId(embedded.filter((f) => !['hooks/hooks.json', '.mcp.json'].includes(f.path))),
    );
    // The worker's activity hooks replace the standalone sync hook.
    expect(m.disk['/w/p/hooks/hooks.json']).toContain('claude::terminal::activity');
    // With mcp-server, default-mcp guides the session, not the CLI-only text.
    expect(m.disk['/w/p/skills/iii-runtime/SKILL.md']).toBeUndefined();
    expect(m.disk['/w/p/commands/writer.md']).toContain('`writer`');

    m.writes.length = 0;
    await installWith('/w/p', m.ops, options, embedded);
    expect(m.writes).toEqual([]);
  });

  it('keeps the existing commands and says so when profiles cannot be read', async () => {
    const m = memoryOps();
    const r = await installWith(
      '/w/p',
      m.ops,
      { ...options, agents: null, agentsDetail: 'down' },
      embedded,
    );
    expect(r.dir).toBe('/w/p');
    expect(r.detail).toContain('down');
  });

  it('reports a build without the embedded plugin instead of installing half a plugin', async () => {
    const m = memoryOps();
    const r = await installWith('/w/p', m.ops, options, [embedded[2]]);
    expect(r.dir).toBe('');
    expect(r.detail).toContain('build:iii-plugin');
    expect(m.writes).toEqual([]);
  });
});

describe('installer review fixes', () => {
  it('removes the runtime skill when the context becomes empty', async () => {
    const m = memoryOps();
    const embedded = [
      { path: '.claude-plugin/plugin.json', content: '{"name":"iii"}' },
      { path: '.mcp.json', content: '{}' },
      { path: 'cli/index.mjs', content: 'x' },
    ];
    const base = {
      cli: 'iii',
      agents: [],
      // The runtime skill is the session's guide only without mcp-server.
      mcp: { url: 'http://127.0.0.1:3111/mcp', detail: '', available: false },
      defaultProfile: null,
    };
    await installWith(
      '/w/p',
      m.ops,
      { ...base, context: { text: '# iii runtime', detail: '' } },
      embedded,
    );
    expect(m.disk['/w/p/skills/iii-runtime/SKILL.md']).toBeDefined();
    await installWith(
      '/w/p',
      m.ops,
      { ...base, context: { text: '', detail: 'unavailable' } },
      embedded,
    );
    expect(m.disk['/w/p/skills/iii-runtime/SKILL.md']).toBeUndefined();
  });

  it('writes files atomically: a temp file renamed into place, none left behind', async () => {
    const dir = await mkdtemp(pjoin(osTmp(), 'iii-plugin-'));
    const path = pjoin(dir, 'cli', 'index.mjs');
    await atomicWrite(path, 'old');
    // Two concurrent installers: the file is always one complete version.
    await Promise.all([
      atomicWrite(path, 'a'.repeat(100_000)),
      atomicWrite(path, 'b'.repeat(100_000)),
    ]);
    const content = await fsRead(path, 'utf8');
    expect(content === 'a'.repeat(100_000) || content === 'b'.repeat(100_000)).toBe(true);
    expect(await readdir(pjoin(dir, 'cli'))).toEqual(['index.mjs']);
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
    expect(r.detail).toContain('iii tools');
  });

  it('falls back to the default address when the http worker does not answer', async () => {
    const r = await resolveWorkerMcp(
      fakeIii({ 'engine::functions::list': running['engine::functions::list'] }),
      {},
    );
    expect(r.url).toBe('http://127.0.0.1:3111/mcp');
    expect(r.detail).toContain('http::status');
  });

  it('passes the detail on through the installer', async () => {
    const m = memoryOps();
    const r = await installWith(
      '/w/p',
      m.ops,
      {
        cli: 'iii',
        context: { text: '# iii runtime', detail: '' },
        agents: [],
        mcp: {
          url: 'http://127.0.0.1:3111/mcp',
          detail: 'mcp-server is not running on this engine',
          available: false,
        },
      },
      [
        { path: '.claude-plugin/plugin.json', content: '{"name":"iii"}' },
        { path: '.mcp.json', content: '{}' },
      ],
    );
    expect(r.dir).toBe('/w/p');
    expect(r.detail).toContain('mcp-server is not running on this engine');
  });
});

describe('the iii agent and the mcp-server fallback', () => {
  const STUB =
    '---\nname: iii\ndescription: the iii agent\n---\n\nLoad default-mcp with iii_get_agent.\n';
  const embedded = [
    { path: '.claude-plugin/plugin.json', content: '{"name":"iii"}' },
    {
      path: '.mcp.json',
      content: '{"mcpServers":{"iii":{"type":"http","url":"${user_config.mcp_url}"}}}',
    },
    { path: 'agents/iii.md', content: STUB },
    { path: 'cli/index.mjs', content: 'x' },
  ];
  const profile = { id: 'default-mcp', system_prompt: 'You are an iii agent. Use iii_call.' };
  const base = {
    cli: 'iii',
    context: { text: '# iii runtime', detail: '' },
    agents: [],
  };
  const mcp = (available: boolean) => ({
    url: 'http://127.0.0.1:3211/mcp',
    detail: available ? '' : 'mcp-server is not running',
    available,
  });

  it('with mcp-server: default-mcp is the agent and the appended text, and nothing hand-written is', async () => {
    const m = memoryOps();
    const r = await installWith(
      '/w/p',
      m.ops,
      { ...base, mcp: mcp(true), defaultProfile: profile },
      embedded,
    );
    expect(r.mcp).toBe(true);
    expect(r.mode).toBe('You are an iii agent. Use iii_call.');
    expect(m.disk['/w/p/default-mcp.md']).toBe('You are an iii agent. Use iii_call.\n');
    expect(m.disk['/w/p/agents/iii.md']).toContain('name: iii');
    expect(m.disk['/w/p/agents/iii.md']).toContain('You are an iii agent. Use iii_call.');
    expect(JSON.parse(m.disk['/w/p/.mcp.json']).mcpServers.iii.url).toBe(
      'http://127.0.0.1:3211/mcp',
    );
    // Appended by the worker, so no hook adds it a second time.
    expect(m.disk['/w/p/hooks/hooks.json']).not.toContain('session-start');
    // The CLI-only runtime skill would contradict it, and is not missed.
    expect(m.disk['/w/p/skills/iii-runtime/SKILL.md']).toBeUndefined();
    expect(r.detail).toBe('');
  });

  it("appends the profile's skills too: the worker installs no SessionStart hook", async () => {
    const m = memoryOps();
    const r = await installWith(
      '/w/p',
      m.ops,
      { ...base, mcp: mcp(true), defaultProfile: { ...profile, skills: ['ide'] } },
      embedded,
    );
    const line = 'read each of its skills with `iii_get_skill`: `ide`.';
    expect(r.mode).toContain('You are an iii agent. Use iii_call.');
    expect(r.mode).toContain(line);
    expect(m.disk['/w/p/default-mcp.md']).toContain(line);
    expect(m.disk['/w/p/agents/iii.md']).toContain(line);
  });

  it('removes a runtime skill left from before mcp-server was running', async () => {
    const m = memoryOps();
    await installWith(
      '/w/p',
      m.ops,
      { ...base, mcp: mcp(false), defaultProfile: profile },
      embedded,
    );
    expect(m.disk['/w/p/skills/iii-runtime/SKILL.md']).toContain('# iii runtime');

    await installWith(
      '/w/p',
      m.ops,
      { ...base, mcp: mcp(true), defaultProfile: profile },
      embedded,
    );

    expect(m.disk['/w/p/skills/iii-runtime/SKILL.md']).toBeUndefined();
  });

  it('says so, and keeps the stub agent, when default-mcp cannot be read', async () => {
    const m = memoryOps();
    const r = await installWith(
      '/w/p',
      m.ops,
      {
        ...base,
        mcp: mcp(true),
        defaultProfile: null,
        defaultProfileDetail: 'iii-directory did not answer',
      },
      embedded,
    );
    expect(r.mode).toBe('');
    expect(r.detail).toContain('default-mcp');
    expect(r.detail).toContain('iii-directory did not answer');
    expect(m.disk['/w/p/agents/iii.md']).toBe(STUB);
    expect(m.disk['/w/p/default-mcp.md']).toBeUndefined();
  });

  it('without mcp-server: no MCP server, nothing appended, the stub agent back, stale files removed', async () => {
    const m = memoryOps();
    m.disk['/w/p/.mcp.json'] = '{"mcpServers":{}}';
    m.disk['/w/p/default-mcp.md'] = 'old profile';
    m.disk['/w/p/agents/iii.md'] = 'an agent built from an older profile';
    m.disk['/w/p/.build-id'] = 'stale';
    const r = await installWith(
      '/w/p',
      m.ops,
      { ...base, mcp: mcp(false), defaultProfile: profile },
      embedded,
    );
    expect(r.dir).toBe('/w/p');
    expect(r.mcp).toBe(false);
    expect(r.mode).toBe('');
    expect(m.disk['/w/p/.mcp.json']).toBeUndefined();
    expect(m.disk['/w/p/default-mcp.md']).toBeUndefined();
    expect(m.disk['/w/p/agents/iii.md']).toBe(STUB);
    expect(r.detail).toContain('mcp-server is not running');
    // The session's guide is then the CLI runtime skill.
    expect(m.disk['/w/p/skills/iii-runtime/SKILL.md']).toContain('# iii runtime');
  });
});
