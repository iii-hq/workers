import { mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { loadConfig } from '../src/config.js';

describe('loadConfig', () => {
  it('returns full defaults when the file is missing', async () => {
    const cfg = await loadConfig('/nonexistent/config.yaml');
    expect(cfg.engine_url).toBe('ws://127.0.0.1:49134');
    expect(cfg.defaults.sandbox_mode).toBe('workspace-write');
    expect(cfg.defaults.approval_policy).toBe('never');
    expect(cfg.defaults.skip_git_repo_check).toBe(true);
    expect(cfg.events_stream).toBe('agent::events');
    expect(cfg.raw_events_stream).toBe('codex::events');
    expect(cfg.iii_context).toBe(true);
    expect(cfg.codex_executable).toBe('');
    expect(cfg.base_url).toBe('');
  });

  it('merges a partial file over defaults', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'codex-config-'));
    const path = join(dir, 'config.yaml');
    await writeFile(
      path,
      ['engine_url: ws://10.0.0.1:49134', 'defaults:', '  sandbox_mode: read-only'].join('\n'),
    );
    const cfg = await loadConfig(path);
    expect(cfg.engine_url).toBe('ws://10.0.0.1:49134');
    expect(cfg.defaults.sandbox_mode).toBe('read-only');
    expect(cfg.defaults.approval_policy).toBe('never');
  });

  it('rethrows YAML parse errors instead of silently using defaults', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'codex-config-'));
    const path = join(dir, 'config.yaml');
    await writeFile(path, 'defaults: [unclosed\n  bad: {');
    await expect(loadConfig(path)).rejects.toThrow();
  });

  it('rejects an invalid sandbox mode', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'codex-config-'));
    const path = join(dir, 'config.yaml');
    await writeFile(path, 'defaults:\n  sandbox_mode: yolo\n');
    await expect(loadConfig(path)).rejects.toThrow();
  });
});
