import { mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { loadConfig } from '../src/config.js';

describe('loadConfig', () => {
  it('returns full defaults when the file is missing', async () => {
    const cfg = await loadConfig('/nonexistent/config.yaml');
    expect(cfg.engine_url).toBe('ws://127.0.0.1:49134');
    expect(cfg.defaults.thinking_level).toBe('medium');
    expect(cfg.defaults.tools).toEqual([]);
    expect(cfg.events_stream).toBe('agent::events');
    expect(cfg.raw_events_stream).toBe('pi::events');
    expect(cfg.iii_context).toBe(true);
  });

  it('merges a partial file over defaults', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'pi-config-'));
    const path = join(dir, 'config.yaml');
    await writeFile(
      path,
      [
        'engine_url: ws://10.0.0.1:49134',
        'defaults:',
        '  thinking_level: high',
        'iii_context: false',
      ].join('\n'),
    );
    const cfg = await loadConfig(path);
    expect(cfg.engine_url).toBe('ws://10.0.0.1:49134');
    expect(cfg.defaults.thinking_level).toBe('high');
    expect(cfg.defaults.tools).toEqual([]);
    expect(cfg.iii_context).toBe(false);
  });

  it('rethrows YAML parse errors instead of silently using defaults', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'pi-config-'));
    const path = join(dir, 'config.yaml');
    await writeFile(path, 'defaults: [unclosed\n  bad: {');
    await expect(loadConfig(path)).rejects.toThrow();
  });

  it('rejects an invalid thinking level', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'pi-config-'));
    const path = join(dir, 'config.yaml');
    await writeFile(path, 'defaults:\n  thinking_level: ultra\n');
    await expect(loadConfig(path)).rejects.toThrow();
  });
});
