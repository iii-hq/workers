import { mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { loadConfig, runtimeJsonSchema, toRuntime } from '../src/config.js';

describe('loadConfig', () => {
  it('returns full defaults when the file is missing', async () => {
    const cfg = await loadConfig('/nonexistent/config.yaml');
    expect(cfg.engine_url).toBe('ws://127.0.0.1:49134');
    expect(cfg.defaults.model).toBe('');
    expect(cfg.events_stream).toBe('agent::events');
    expect(cfg.raw_events_stream).toBe('opencode::events');
    expect(cfg.iii_context).toBe(true);
    expect(cfg.opencode_executable).toBe('');
  });

  it('merges a partial file over defaults', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'opencode-config-'));
    const path = join(dir, 'config.yaml');
    await writeFile(
      path,
      ['defaults:', '  model: anthropic/claude-sonnet-4-5', 'iii_context: false'].join('\n'),
    );
    const cfg = await loadConfig(path);
    expect(cfg.defaults.model).toBe('anthropic/claude-sonnet-4-5');
    expect(cfg.defaults.cwd).toBe('');
    expect(cfg.iii_context).toBe(false);
  });

  it('rethrows YAML parse errors instead of using defaults', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'opencode-config-'));
    const path = join(dir, 'config.yaml');
    await writeFile(path, 'defaults: [unclosed\n  bad: {');
    await expect(loadConfig(path)).rejects.toThrow();
  });
});

describe('runtimeJsonSchema', () => {
  it('excludes engine_url and the $schema meta-ref, stays typed', () => {
    const s = runtimeJsonSchema() as {
      properties?: Record<string, unknown>;
      $schema?: unknown;
      type?: string;
    };
    expect(s.$schema).toBeUndefined();
    expect(s.type).toBe('object');
    expect(s.properties).not.toHaveProperty('engine_url');
    expect(s.properties).toHaveProperty('defaults');
  });
  it('toRuntime drops engine_url, keeps the rest', async () => {
    const rt = toRuntime(await loadConfig('/nonexistent/config.yaml')) as Record<string, unknown>;
    expect(rt).not.toHaveProperty('engine_url');
    expect(rt.raw_events_stream).toBe('opencode::events');
  });
});
