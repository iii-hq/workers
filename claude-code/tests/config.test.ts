import { mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { loadConfig } from '../src/config.js';

describe('loadConfig', () => {
  it('returns full defaults when the file is missing', async () => {
    const cfg = await loadConfig('/nonexistent/config.yaml');
    expect(cfg.engine_url).toBe('ws://127.0.0.1:49134');
    expect(cfg.defaults.permission_mode).toBe('acceptEdits');
    expect(cfg.defaults.max_turns).toBe(50);
    expect(cfg.approval_gate).toBe(false);
    expect(cfg.events_stream).toBe('agent::events');
    expect(cfg.raw_events_stream).toBe('claude::events');
    expect(cfg.claude_executable).toBe('');
  });

  it('merges a partial file over defaults', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'claude-code-config-'));
    const path = join(dir, 'config.yaml');
    await writeFile(
      path,
      [
        'engine_url: ws://10.0.0.1:49134',
        'defaults:',
        '  permission_mode: plan',
        'approval_gate: true',
      ].join('\n'),
    );
    const cfg = await loadConfig(path);
    expect(cfg.engine_url).toBe('ws://10.0.0.1:49134');
    expect(cfg.defaults.permission_mode).toBe('plan');
    expect(cfg.defaults.max_turns).toBe(50);
    expect(cfg.approval_gate).toBe(true);
  });

  it('rejects an invalid permission mode', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'claude-code-config-'));
    const path = join(dir, 'config.yaml');
    await writeFile(path, 'defaults:\n  permission_mode: yolo\n');
    await expect(loadConfig(path)).rejects.toThrow();
  });
});
