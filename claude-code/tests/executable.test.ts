import { chmodSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { resolveClaudeExecutable } from '../src/executable.js';

describe('resolveClaudeExecutable', () => {
  const originalPath = process.env.PATH;

  beforeEach(() => {
    process.env.PATH = '';
  });

  afterEach(() => {
    process.env.PATH = originalPath;
  });

  it('returns the configured path untouched', () => {
    expect(resolveClaudeExecutable('/opt/claude')).toBe('/opt/claude');
  });

  it('finds an executable claude on PATH', () => {
    const dir = mkdtempSync(join(tmpdir(), 'claude-exe-'));
    const bin = join(dir, 'claude');
    writeFileSync(bin, '#!/bin/sh\n');
    chmodSync(bin, 0o755);
    process.env.PATH = dir;
    expect(resolveClaudeExecutable('')).toBe(bin);
  });

  it('returns empty when nothing is found', () => {
    expect(resolveClaudeExecutable('')).toBe('');
  });
});
