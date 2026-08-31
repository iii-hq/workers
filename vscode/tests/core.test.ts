import { describe, expect, it } from 'vitest';
import {
  instanceIdFor,
  isLoopback,
  publicInstance,
  serverArgs,
  validateId,
  validateWorkspacePath,
} from '../src/core.js';

describe('vscode worker core', () => {
  it('validates stable ids', () => {
    expect(validateId('console-vscode')).toBe('console-vscode');
    expect(() => validateId('../bad')).toThrow();
    expect(() => validateId('Upper')).toThrow();
    expect(() => validateId('')).toThrow();
  });

  it('accepts only absolute workspace paths', () => {
    expect(validateWorkspacePath('/tmp/work')).toBe('/tmp/work');
    expect(validateWorkspacePath('/tmp/work/../work')).toBe('/tmp/work');
    expect(() => validateWorkspacePath('relative')).toThrow(/absolute/);
    expect(() => validateWorkspacePath(undefined)).toThrow(/absolute/);
  });

  it('derives one stable id per workspace', () => {
    const id = instanceIdFor('/home/dev/project');
    expect(id).toBe(instanceIdFor('/home/dev/project'));
    expect(id).not.toBe(instanceIdFor('/home/dev/other'));
    expect(validateId(id)).toBe(id);
  });

  it('recognises loopback hosts only', () => {
    expect(isLoopback('127.0.0.1')).toBe(true);
    expect(isLoopback('localhost')).toBe(true);
    expect(isLoopback('::1')).toBe(true);
    expect(isLoopback('0.0.0.0')).toBe(false);
    expect(isLoopback('192.168.1.10')).toBe(false);
  });

  it('builds code serve-web arguments in loopback cookie-free mode', () => {
    expect(
      serverArgs({
        bindHost: '127.0.0.1',
        port: 18080,
        serverData: '/data/server',
        cliData: '/data/cli',
        workspace: '/work',
      }),
    ).toEqual([
      '--cli-data-dir',
      '/data/cli',
      'serve-web',
      '--host',
      '127.0.0.1',
      '--port',
      '18080',
      '--without-connection-token',
      '--accept-server-license-terms',
      '--server-data-dir',
      '/data/server',
      '--default-folder',
      '/work',
      '--disable-telemetry',
    ]);
  });

  it('refuses cookie-free mode on a network listener', () => {
    expect(() =>
      serverArgs({
        bindHost: '0.0.0.0',
        port: 18080,
        serverData: '/data/server',
        cliData: '/data/cli',
        workspace: '/work',
      }),
    ).toThrow(/loopback/);
  });

  it('exposes host and port but not the internal url or process', () => {
    const item = publicInstance({
      id: 'x',
      name: 'VS Code',
      workspace: '/work',
      host: '127.0.0.1',
      port: 18080,
      url: 'http://127.0.0.1:18080/',
      pid: 1,
      started_at: 'now',
      status: 'running',
      process: {} as never,
    });
    expect(item).toEqual({
      id: 'x',
      name: 'VS Code',
      workspace: '/work',
      host: '127.0.0.1',
      port: 18080,
      pid: 1,
      started_at: 'now',
      status: 'running',
      exit_code: null,
    });
    expect(item).not.toHaveProperty('url');
    expect(item).not.toHaveProperty('process');
  });
});
