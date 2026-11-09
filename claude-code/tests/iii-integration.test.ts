import { describe, expect, it } from 'vitest';
import { Directory, renderSkill, renderSubagent, slug } from '../src/iii-integration/directory.js';
import { normalizeUrl, resolveEngine } from '../src/iii-integration/engine.js';
import { createHandler } from '../src/iii-integration/server.js';
import { INTEGRATION_FILES } from '../src/iii-integration/assets.generated.js';
import { buildId, installWith } from '../src/iii-integration/install.js';

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
        env: { ...env, III_INTEGRATION_URL: 'ws://h:1' },
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
      env: { III_INTEGRATION_URL: '${user_config.engine_url}', III_URL: 'ws://h:3' },
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
      env: { III_INTEGRATION_PORTS: '5000', III_INTEGRATION_HOSTS: 'a' },
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

  it('lists visible agents as prompts and skills as resources', async () => {
    const prompts = (await handle({ jsonrpc: '2.0', id: 1, method: 'prompts/list' })) as {
      prompts: { name: string }[];
    };
    expect(prompts.prompts.map((p) => p.name)).toEqual(['writer']);
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

  it('turns an agent profile into a prompt with the task', async () => {
    const res = (await handle({
      jsonrpc: '2.0',
      id: 5,
      method: 'prompts/get',
      params: { name: 'writer', arguments: { task: 'draft it' } },
    })) as { messages: { content: { text: string } }[] };
    const textOut = res.messages[0].content.text;
    expect(textOut).toContain('You write.');
    expect(textOut).toContain('Task: draft it');
  });

  it('rejects unknown methods with -32601', async () => {
    await expect(handle({ jsonrpc: '2.0', id: 6, method: 'nope' })).rejects.toMatchObject({
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

describe('installer', () => {
  const files = [
    { path: '.claude-plugin/plugin.json', content: '{}' },
    { path: 'server/index.mjs', content: 'x' },
  ];

  it('ships the plugin inside the worker build', () => {
    const paths = INTEGRATION_FILES.map((f) => f.path);
    expect(paths).toEqual(
      expect.arrayContaining(['.claude-plugin/plugin.json', '.mcp.json', 'server/index.mjs']),
    );
    expect(
      JSON.parse(INTEGRATION_FILES.find((f) => f.path === '.claude-plugin/plugin.json')!.content)
        .name,
    ).toBe('claude-cli-iii-integration');
    // Under the shell worker's common 1 MiB per-file cap.
    for (const f of INTEGRATION_FILES)
      expect(Buffer.byteLength(f.content)).toBeLessThan(1024 * 1024);
  });

  it('writes every file, then the build id, and skips an unchanged install', async () => {
    const disk: Record<string, string> = {};
    const read = async (p: string) => disk[p] ?? null;
    const writes: string[] = [];
    const write = async (p: string, c: string) => {
      writes.push(p);
      disk[p] = c;
    };
    expect(await installWith('/w/p', read, write, files)).toEqual({ dir: '/w/p', detail: '' });
    expect(writes).toEqual([
      '/w/p/.claude-plugin/plugin.json',
      '/w/p/server/index.mjs',
      '/w/p/.build-id',
    ]);
    expect(disk['/w/p/.build-id'].trim()).toBe(buildId(files));
    writes.length = 0;
    await installWith('/w/p', read, write, files);
    expect(writes).toEqual([]);
    await installWith('/w/p', read, write, [...files, { path: 'README.md', content: 'new' }]);
    expect(writes).toContain('/w/p/README.md');
  });

  it('reports a build without the embedded server instead of installing half a plugin', async () => {
    const r = await installWith(
      '/w/p',
      async () => null,
      async () => {},
      [files[0]],
    );
    expect(r.dir).toBe('');
    expect(r.detail).toContain('build:iii-integration');
  });
});
