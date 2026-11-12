import { describe, expect, it } from 'vitest';
import {
  Directory,
  pluginWorkerName,
  renderSkill,
  renderSubagent,
  slug,
} from '../src/iii-plugin/directory.js';
import { normalizeUrl, resolveEngine } from '../src/iii-plugin/engine.js';
import { createHandler } from '../src/iii-plugin/server.js';
import { PLUGIN_STATIC_FILES } from '../src/iii-plugin/assets.generated.js';
import { buildId, installWith } from '../src/iii-plugin/install.js';
import {
  profileCommand,
  profileCommandName,
  writeProfileCommands,
} from '../src/iii-plugin/commands.js';

const none = async () => false;

describe('resolveEngine', () => {
  it('prefers the --url flag over every env var', async () => {
    const r = await resolveEngine({
      argv: ['serve', '--url', '49000'],
      env: { III_URL: 'ws://x:1' },
      probe: none,
    });
    expect(r).toEqual({ url: 'ws://127.0.0.1:49000', source: 'flag' });
  });

  it('walks plugin env, plugin option, III_URL, III_ENGINE_URL in order', async () => {
    const env = {
      CLAUDE_PLUGIN_OPTION_ENGINE_URL: 'h:2',
      III_URL: 'ws://h:3',
      III_ENGINE_URL: 'ws://h:4',
    };
    expect(
      await resolveEngine({
        argv: [],
        env: { ...env, III_PLUGIN_URL: 'ws://h:1' },
        probe: none,
      }),
    ).toEqual({
      url: 'ws://h:1',
      source: 'plugin-env',
    });
    expect(await resolveEngine({ argv: [], env, probe: none })).toEqual({
      url: 'ws://h:2',
      source: 'plugin-option',
    });
    expect(
      await resolveEngine({ argv: [], env: { III_ENGINE_URL: 'ws://h:4' }, probe: none }),
    ).toEqual({
      url: 'ws://h:4',
      source: 'III_ENGINE_URL',
    });
  });

  it('ignores an unexpanded user_config placeholder', async () => {
    const r = await resolveEngine({
      argv: [],
      env: { III_PLUGIN_URL: '${user_config.engine_url}', III_URL: 'ws://h:3' },
      probe: none,
    });
    expect(r.source).toBe('III_URL');
  });

  it('auto-detects the first listening standard port', async () => {
    const seen: string[] = [];
    const probe = async (host: string, port: number) => {
      seen.push(`${host}:${port}`);
      return port === 49135 && host === 'localhost';
    };
    const r = await resolveEngine({ argv: [], env: {}, probe });
    expect(r).toEqual({ url: 'ws://localhost:49135', source: 'auto-detect' });
    expect(seen).toEqual([
      '127.0.0.1:49134',
      'localhost:49134',
      '127.0.0.1:49135',
      'localhost:49135',
    ]);
  });

  it('honours custom probe ports and reports what it tried', async () => {
    const r = await resolveEngine({
      argv: [],
      env: { III_PLUGIN_PORTS: '5000', III_PLUGIN_HOSTS: 'a' },
      probe: none,
    });
    expect(r.source).toBe('none');
    expect(r.source === 'none' && r.detail).toContain('a:5000');
  });

  it('normalizes shorthand addresses', () => {
    expect(normalizeUrl(':9')).toBe('ws://127.0.0.1:9');
    expect(normalizeUrl('host:9')).toBe('ws://host:9');
    expect(normalizeUrl('wss://e.example')).toBe('wss://e.example');
    expect(normalizeUrl('  ')).toBe('');
  });
});

function fakeDirectory(): Directory {
  const responses: Record<string, (p: Record<string, unknown>) => unknown> = {
    'directory::agents::list': () => ({
      agents: [
        { id: 'writer', name: 'Writer', description: 'Writes.' },
        { id: 'helper', hidden: true },
      ],
    }),
    'directory::agents::get': (p) => ({
      id: p.id,
      description: 'Writes.',
      system_prompt: 'You write.',
      skills: ['kanban'],
    }),
    'directory::skills::list': () => ({
      skills: [{ id: 'kanban', title: 'kanban', description: 'Boards.' }],
    }),
    'directory::skills::get': (p) => ({ id: p.id, body: '# kanban body' }),
  };
  const dir = new Directory(
    async () => ({ url: 'ws://fake', source: 'flag' }),
    () => ({}) as never,
  );
  // biome-ignore lint/suspicious/noExplicitAny: test seam over the bus call
  (dir as any).call = async (id: string, payload: Record<string, unknown>) =>
    responses[id](payload);
  return dir;
}

describe('MCP handler', () => {
  const handle = createHandler(fakeDirectory());

  it('lists skills as resources', async () => {
    const resources = (await handle({ jsonrpc: '2.0', id: 2, method: 'resources/list' })) as {
      resources: { uri: string }[];
    };
    expect(resources.resources[0].uri).toBe('iii-skill://kanban');
  });

  it('serves a skill body through the tool and the resource', async () => {
    const tool = (await handle({
      jsonrpc: '2.0',
      id: 3,
      method: 'tools/call',
      params: { name: 'iii_get_skill', arguments: { id: 'kanban' } },
    })) as { content: { text: string }[] };
    expect(tool.content[0].text).toBe('# kanban body');
    const res = (await handle({
      jsonrpc: '2.0',
      id: 4,
      method: 'resources/read',
      params: { uri: 'iii-skill://kanban' },
    })) as { contents: { text: string }[] };
    expect(res.contents[0].text).toBe('# kanban body');
  });

  it('offers no MCP prompts: profiles are /iii:* commands instead', async () => {
    const init = (await handle({ jsonrpc: '2.0', id: 5, method: 'initialize', params: {} })) as {
      capabilities: Record<string, unknown>;
    };
    expect(init.capabilities).not.toHaveProperty('prompts');
    await expect(handle({ jsonrpc: '2.0', id: 6, method: 'prompts/list' })).rejects.toMatchObject({
      code: -32601,
    });
  });

  it('rejects unknown methods with -32601', async () => {
    await expect(handle({ jsonrpc: '2.0', id: 7, method: 'nope' })).rejects.toMatchObject({
      code: -32601,
    });
  });
});

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
    { path: 'hooks/hooks.json', content: 'standalone sync hook' },
    { path: 'commands/profile.md', content: 'p' },
    { path: 'server/index.mjs', content: 'x' },
  ];
  const options = {
    cli: '/usr/bin/iii',
    context: { text: '# iii runtime', detail: '' },
    agents: [{ id: 'writer' }],
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
        'server/index.mjs',
      ]),
    );
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
    });
    expect(m.disk['/w/p/server/index.mjs']).toBe('x');
    expect(m.disk['/w/p/.build-id'].trim()).toBe(
      buildId(embedded.filter((f) => f.path !== 'hooks/hooks.json')),
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

  it('reports a build without the embedded server instead of installing half a plugin', async () => {
    const m = memoryOps();
    const r = await installWith('/w/p', m.ops, options, [embedded[0]]);
    expect(r.dir).toBe('');
    expect(r.detail).toContain('build:iii-plugin');
    expect(m.writes).toEqual([]);
  });
});

describe('pluginWorkerName', () => {
  it('is unique per process and ignores an inherited III_WORKER_NAME', () => {
    const env: NodeJS.ProcessEnv = { III_WORKER_NAME: 'claude-code' };
    const name = pluginWorkerName(env);
    expect(name).toMatch(new RegExp(`^iii-plugin:.+:${process.pid}$`));
    expect(env.III_WORKER_NAME).toBeUndefined();
  });
});
