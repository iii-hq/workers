import { describe, expect, it } from 'vitest';
import { renderSkill, renderSubagent, slug } from '../src/iii-plugin/directory.js';
import { PLUGIN_STATIC_FILES } from '../src/iii-plugin/assets.generated.js';
import { atomicWrite, buildId, installWith, resolveWorkerMcp } from '../src/iii-plugin/install.js';
import { skillDirectories } from '../src/iii-plugin/directory.js';
import { mkdtemp, readdir, readFile as fsRead } from 'node:fs/promises';
import { tmpdir as osTmp } from 'node:os';
import { join as pjoin } from 'node:path';
import {
  profileCommand,
  profileCommandName,
  profileCommands,
  writeProfileCommands,
} from '../src/iii-plugin/commands.js';

describe('renderers', () => {
  it('writes a Claude Code subagent with the preloaded skills named', () => {
    const md = renderSubagent({
      id: 'Deep Work',
      description: 'Runs goals.',
      system_prompt: 'Be deep.',
      skills: ['kanban'],
    });
    expect(md).toMatch(
      /^---\nname: iii-deep-work\ndescription: "Runs goals\. \(iii agent profile `Deep Work`\)"\n---/,
    );
    expect(md).toContain('Be deep.');
    expect(md).toContain('`kanban`');
  });

  it('writes a skill file with a slug name', () => {
    expect(slug('kanban/tickets/ticket-worker')).toBe('kanban-tickets-ticket-worker');
    expect(renderSkill({ id: 'a/b', description: 'D', body: 'Body' })).toBe(
      '---\nname: a-b\ndescription: "D"\n---\n\nBody\n',
    );
  });
});

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

describe('profile commands', () => {
  it('names a command after the profile, never shadowing /iii:profile or /iii:skills, nor taking `skill`', () => {
    expect(profileCommandName('Deep Work')).toBe('deep-work');
    expect(profileCommandName('profile')).toBe('profile-profile');
    expect(profileCommandName('skill')).toBe('profile-skill');
    expect(profileCommandName('skills')).toBe('profile-skills');
  });

  it('is user-invoked only, so it costs no tokens until used', () => {
    const cmd = profileCommand({ id: 'writer', description: 'Writes.' });
    expect(cmd.path).toBe('commands/writer.md');
    expect(cmd.content).toContain('disable-model-invocation: true');
    expect(cmd.content).toContain('description: "iii profile writer: Writes."');
    expect(cmd.content).toContain('argument-hint: [task]');
    expect(cmd.content).toContain('mcp__plugin_iii_iii__iii_get_agent');
    expect(cmd.content).toContain('$ARGUMENTS');
  });

  it('writes visible profiles, removes gone ones, and touches nothing it did not generate', async () => {
    const m = memoryOps();
    m.disk['/p/commands/profile.md'] = 'static';
    const first = await writeProfileCommands(
      '/p',
      [{ id: 'writer' }, { id: 'deepwork' }, { id: 'helper', hidden: true }],
      m.ops,
    );
    expect(first.written.sort()).toEqual(['commands/deepwork.md', 'commands/writer.md']);
    expect(m.disk['/p/commands/helper.md']).toBeUndefined();

    m.writes.length = 0;
    await writeProfileCommands('/p', [{ id: 'writer' }, { id: 'deepwork' }], m.ops);
    expect(m.writes).toEqual([]);

    const next = await writeProfileCommands('/p', [{ id: 'writer' }], m.ops);
    expect(next.removed).toEqual(['commands/deepwork.md']);
    expect(m.disk['/p/commands/deepwork.md']).toBeUndefined();
    expect(m.disk['/p/commands/profile.md']).toBe('static');
  });

  it('never removes a path outside commands/ even if the manifest names one', async () => {
    const m = memoryOps();
    m.disk['/p/.generated-commands.json'] = JSON.stringify([
      '../../etc/passwd',
      'server/index.mjs',
    ]);
    await writeProfileCommands('/p', [], m.ops);
    expect(m.removes).toEqual([]);
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
    // Under the shell worker's common 1 MiB per-file cap.
    for (const f of PLUGIN_STATIC_FILES)
      expect(Buffer.byteLength(f.content)).toBeLessThan(1024 * 1024);
  });

  it('writes static files, the worker hooks and skill, and the profile commands', async () => {
    const m = memoryOps();
    expect(await installWith('/w/p', m.ops, options, embedded)).toEqual({
      dir: '/w/p',
      detail: '',
      mode: '',
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
    expect(m.disk['/w/p/skills/iii-runtime/SKILL.md']).toContain('# iii runtime');
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

describe('review fixes', () => {
  it('skips profiles whose command name is empty or already taken', () => {
    const files = profileCommands([
      { id: 'Deep Work' },
      { id: 'deep-work' },
      { id: 'a/b' },
      { id: 'a-b' },
      { id: '***' },
    ]);
    expect(files.map((f) => f.path)).toEqual(['commands/deep-work.md', 'commands/a-b.md']);
    // The first profile keeps the name.
    expect(files[0].content).toContain('`Deep Work`');
  });

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
      mcp: { url: 'http://127.0.0.1:3111/mcp', detail: '', available: true },
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

  it('refuses a skill export whose ids land in one directory', () => {
    expect(skillDirectories(['foo/bar', 'baz'])).toEqual(
      new Map([
        ['foo-bar', 'foo/bar'],
        ['baz', 'baz'],
      ]),
    );
    expect(() => skillDirectories(['foo/bar-baz', 'foo-bar/baz'])).toThrow(
      /foo\/bar-baz.*foo-bar\/baz|foo-bar\/baz.*foo\/bar-baz/,
    );
  });
});

describe('the plugin talks to mcp-server', () => {
  const file = (path: string) => PLUGIN_STATIC_FILES.find((f) => f.path === path)?.content ?? '';

  it('declares mcp-server over HTTP at the mcp_url option', () => {
    expect(JSON.parse(file('.mcp.json'))).toEqual({
      mcpServers: { iii: { type: 'http', url: '${user_config.mcp_url}' } },
    });
    const manifest = JSON.parse(file('.claude-plugin/plugin.json'));
    expect(manifest.userConfig.engine_url).toBeUndefined();
    expect(manifest.userConfig.mcp_url).toMatchObject({
      type: 'string',
      default: 'http://127.0.0.1:3111/mcp',
    });
    expect(manifest.userConfig.mcp_url.description).toContain('mcp-server');
  });

  it('ships a small CLI without iii-sdk: every engine call goes through mcp-server', () => {
    const cli = file('cli/index.mjs');
    expect(cli.length).toBeGreaterThan(0);
    expect(Buffer.byteLength(cli)).toBeLessThan(64 * 1024);
    expect(cli).not.toContain('registerWorker');
    expect(cli).not.toContain('iii-sdk');
  });

  it('syncs the profile commands with the CLI at session start', () => {
    const hooks = JSON.parse(file('hooks/hooks.json')).hooks;
    expect(hooks.SessionStart[0].hooks[2].command).toContain('/cli/index.mjs" sync');
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

describe('iii mode and the mcp-server fallback', () => {
  const embedded = [
    { path: '.claude-plugin/plugin.json', content: '{"name":"iii"}' },
    {
      path: '.mcp.json',
      content: '{"mcpServers":{"iii":{"type":"http","url":"${user_config.mcp_url}"}}}',
    },
    {
      path: 'agents/iii.md',
      content: '---\nname: iii\ndescription: iii mode\n---\n\nUse iii_call.\n',
    },
    { path: 'cli/index.mjs', content: 'x' },
  ];
  const base = {
    cli: 'iii',
    context: { text: '# iii runtime', detail: '' },
    agents: [],
  };

  it('with mcp-server: .mcp.json, the session-start hook and the iii mode rules', async () => {
    const m = memoryOps();
    const r = await installWith(
      '/w/p',
      m.ops,
      { ...base, mcp: { url: 'http://127.0.0.1:3211/mcp', detail: '', available: true } },
      embedded,
    );
    expect(r.mcp).toBe(true);
    expect(r.mode).toBe('Use iii_call.');
    expect(m.disk['/w/p/iii-mode.md']).toBe('Use iii_call.\n');
    expect(JSON.parse(m.disk['/w/p/.mcp.json']).mcpServers.iii.url).toBe(
      'http://127.0.0.1:3211/mcp',
    );
    expect(m.disk['/w/p/hooks/hooks.json']).toContain('session-start');
  });

  it('without mcp-server: no MCP server, no hook, no rules, and an old .mcp.json removed', async () => {
    const m = memoryOps();
    m.disk['/w/p/.mcp.json'] = '{"mcpServers":{}}';
    m.disk['/w/p/iii-mode.md'] = 'old rules';
    const r = await installWith(
      '/w/p',
      m.ops,
      {
        ...base,
        mcp: {
          url: 'http://127.0.0.1:3111/mcp',
          detail: 'mcp-server is not running',
          available: false,
        },
      },
      embedded,
    );
    expect(r.dir).toBe('/w/p');
    expect(r.mcp).toBe(false);
    expect(r.mode).toBe('');
    expect(m.disk['/w/p/.mcp.json']).toBeUndefined();
    expect(m.disk['/w/p/iii-mode.md']).toBeUndefined();
    expect(m.disk['/w/p/hooks/hooks.json']).not.toContain('session-start');
    expect(r.detail).toContain('mcp-server is not running');
    // The rest of the plugin still lands.
    expect(m.disk['/w/p/.claude-plugin/plugin.json']).toBeDefined();
  });
});
