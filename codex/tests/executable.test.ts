import { chmodSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { resolveCodexExecutable } from '../src/executable.js';

describe('resolveCodexExecutable', () => {
  const originalPath = process.env.PATH;

  beforeEach(() => {
    process.env.PATH = '';
  });

  afterEach(() => {
    process.env.PATH = originalPath;
  });

  it('returns the configured path untouched', () => {
    expect(resolveCodexExecutable('/opt/codex')).toBe('/opt/codex');
  });

  it('finds an executable codex on PATH', () => {
    const dir = mkdtempSync(join(tmpdir(), 'codex-exe-'));
    const bin = join(dir, 'codex');
    writeFileSync(bin, '#!/bin/sh\n');
    chmodSync(bin, 0o755);
    process.env.PATH = dir;
    expect(resolveCodexExecutable('')).toBe(bin);
  });

  it('returns empty when nothing is found', () => {
    expect(resolveCodexExecutable('')).toBe('');
  });
});
