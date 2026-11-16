import { mkdir, mkdtemp, readdir, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { runCli } from '../src/iii-plugin/cli.js';

/** A minimal mcp-server: answers initialize and tools/call from `tools`. */
function fakeFetch(tools: Record<string, (args: Record<string, unknown>) => string>): typeof fetch {
  return (async (_url: string | URL, init: RequestInit = {}) => {
    if (init.method === 'DELETE') return new Response(null, { status: 204 });
    const msg = JSON.parse(String(init.body)) as {
      id?: number;
      method: string;
      params?: { name: string; arguments: Record<string, unknown> };
    };
    if (msg.id === undefined) return new Response(null, { status: 202 });
    const result =
      msg.method === 'initialize'
        ? { protocolVersion: '2025-06-18', capabilities: {} }
        : {
            content: [
              { type: 'text', text: tools[msg.params?.name ?? '']?.(msg.params?.arguments ?? {}) },
            ],
          };
    return Response.json(
      { jsonrpc: '2.0', id: msg.id, result },
      { headers: { 'mcp-session-id': 's' } },
    );
  }) as typeof fetch;
}

function io(env: NodeJS.ProcessEnv = {}, fetchFn?: typeof fetch) {
  const out: string[] = [];
  const err: string[] = [];
  return {
    out,
    err,
    io: {
      env,
      fetchFn: fetchFn ?? fakeFetch({}),
      stdout: (s: string) => out.push(s),
      stderr: (s: string) => err.push(s),
    },
  };
}

async function pluginDir(): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), 'iii-cli-test-'));
  await mkdir(join(dir, 'agents'), { recursive: true });
  await writeFile(join(dir, 'agents', 'iii.md'), '---\nname: iii\n---\n\nUse iii_call.\n');
  return dir;
}

const hookContext = (out: string[]) =>
  JSON.parse(out.join('')).hookSpecificOutput.additionalContext as string;

describe('runCli', () => {
  it('sync builds the iii agent from default-mcp', async () => {
    const dir = await pluginDir();
    const fetchFn = fakeFetch({
      iii_list_agents: () => JSON.stringify([{ id: 'writer' }]),
      iii_get_agent: (a) =>
        JSON.stringify({ id: a.id, system_prompt: `${a.id} profile text, with iii_call.` }),
    });
    const t = io({}, fetchFn);
    expect(await runCli(['sync', '--plugin-dir', dir, '--quiet'], t.io)).toBe(0);
    const agent = await readFile(join(dir, 'agents', 'iii.md'), 'utf8');
    expect(agent).toContain('name: iii');
    expect(agent).toContain('default-mcp profile text, with iii_call.');
  });

  it('sync keeps the agent it has when default-mcp cannot be read', async () => {
    const dir = await pluginDir();
    const fetchFn = fakeFetch({ iii_list_agents: () => JSON.stringify([]) });
    const t = io({}, fetchFn);
    await runCli(['sync', '--plugin-dir', dir, '--quiet'], t.io);
    expect(await readFile(join(dir, 'agents', 'iii.md'), 'utf8')).toContain('Use iii_call.');
  });

  it('session-start explains a bad address in the hook output instead of failing', async () => {
    const t = io({ III_PLUGIN_MCP_URL: 'ws://127.0.0.1:49134' });
    expect(await runCli(['session-start'], t.io)).toBe(0);
    const text = hookContext(t.out);
    expect(text).toContain('could not load');
    expect(text).toContain('WebSocket');
  });

  it('export refuses profiles whose files collide, before writing any', async () => {
    const target = await mkdtemp(join(tmpdir(), 'iii-export-test-'));
    const fetchFn = fakeFetch({
      iii_list_agents: () => JSON.stringify([{ id: 'Deep Work' }, { id: 'deep-work' }]),
      iii_get_agent: (a) => JSON.stringify({ id: a.id, system_prompt: 'p' }),
    });
    const t = io({}, fetchFn);
    expect(await runCli(['export', '--target', target], t.io)).toBe(1);
    expect(t.err.join('')).toMatch(/Deep Work.*deep-work|deep-work.*Deep Work/);
    expect(await readdir(target)).toEqual([]);
  });

  it('export writes one file per profile when nothing collides', async () => {
    const target = await mkdtemp(join(tmpdir(), 'iii-export-test-'));
    const fetchFn = fakeFetch({
      iii_list_agents: () => JSON.stringify([{ id: 'writer' }, { id: 'reviewer' }]),
      iii_get_agent: (a) => JSON.stringify({ id: a.id, system_prompt: 'p' }),
    });
    const t = io({}, fetchFn);
    expect(await runCli(['export', '--target', target], t.io)).toBe(0);
    expect((await readdir(join(target, 'agents'))).sort()).toEqual([
      'iii-reviewer.md',
      'iii-writer.md',
    ]);
  });
});
