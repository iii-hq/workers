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
    expect(cfg).not.toHaveProperty('events_stream');
    expect(cfg).not.toHaveProperty('raw_events_stream');
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

  it('still loads a stored config that carries the legacy stream keys', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'opencode-config-'));
    const path = join(dir, 'config.yaml');
    await writeFile(
      path,
      ['events_stream: agent::events', 'raw_events_stream: opencode::events'].join('\n'),
    );
    const cfg = await loadConfig(path);
    expect(cfg.iii_context).toBe(true);
    expect(cfg.defaults.model).toBe('');
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
    expect(rt.iii_context).toBe(true);
    expect(rt).not.toHaveProperty('raw_events_stream');
  });
  it('marks the legacy stream keys deprecated, optional and ignored', () => {
    const s = runtimeJsonSchema() as {
      properties: Record<string, { deprecated?: boolean }>;
      required?: string[];
      additionalProperties?: boolean;
    };
    expect(s.additionalProperties).toBe(false);
    expect(s.properties.events_stream?.deprecated).toBe(true);
    expect(s.properties.raw_events_stream?.deprecated).toBe(true);
    expect(s.required).not.toContain('events_stream');
    expect(s.required).not.toContain('raw_events_stream');
  });
});
