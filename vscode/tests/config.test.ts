import { mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { expandHome, loadConfig, runtimeJsonSchema, toRuntime } from '../src/config.js';

describe('config', () => {
  it('falls back to defaults when the seed file is missing', async () => {
    const cfg = await loadConfig(join(tmpdir(), 'vscode-config-missing.yaml'));
    expect(cfg.engine_url).toBe('ws://127.0.0.1:49134');
    expect(cfg.code_executable).toBe('');
    expect(cfg.data_dir).toBe('~/.iii/vscode');
    expect(cfg.bind_host).toBe('127.0.0.1');
    expect([cfg.port_min, cfg.port_max]).toEqual([18080, 18180]);
    expect(cfg.start_timeout_ms).toBe(180_000);
    expect(cfg.stop_grace_ms).toBe(5_000);
  });

  it('parses a seed file and keeps unspecified defaults', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'vscode-config-'));
    const path = join(dir, 'config.yaml');
    await writeFile(path, 'code_executable: /opt/code\nport_min: 20000\nport_max: 20010\n');
    const cfg = await loadConfig(path);
    expect(cfg.code_executable).toBe('/opt/code');
    expect([cfg.port_min, cfg.port_max]).toEqual([20000, 20010]);
    expect(cfg.bind_host).toBe('127.0.0.1');
  });

  it('rejects an inverted port range', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'vscode-config-'));
    const path = join(dir, 'config.yaml');
    await writeFile(path, 'port_min: 19000\nport_max: 18000\n');
    await expect(loadConfig(path)).rejects.toThrow(/port_min must not exceed port_max/);
  });

  it('publishes a JSON schema without engine_url or $schema', () => {
    const schema = runtimeJsonSchema();
    const properties = schema.properties as Record<string, unknown>;
    expect(schema.$schema).toBeUndefined();
    expect(properties.engine_url).toBeUndefined();
    expect(Object.keys(properties)).toEqual([
      'code_executable',
      'data_dir',
      'bind_host',
      'port_min',
      'port_max',
      'start_timeout_ms',
      'stop_grace_ms',
    ]);
  });

  it('strips engine_url from the runtime slice', async () => {
    const runtime = toRuntime(await loadConfig(join(tmpdir(), 'vscode-config-missing.yaml')));
    expect(runtime).not.toHaveProperty('engine_url');
    expect(runtime.data_dir).toBe('~/.iii/vscode');
  });

  it('expands a leading tilde', () => {
    expect(expandHome('~/.iii/vscode', '/home/dev')).toBe('/home/dev/.iii/vscode');
    expect(expandHome('~', '/home/dev')).toBe('/home/dev');
    expect(expandHome('/var/data', '/home/dev')).toBe('/var/data');
  });
});
