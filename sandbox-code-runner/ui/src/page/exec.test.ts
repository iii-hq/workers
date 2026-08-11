/**
 * Exec-shaping checks: the argv builder (the detached shape is
 * load-bearing — see DETACH_TITLE), payload assembly, the copy-as-CLI
 * formatter's quoting, response parsing, the exit pill and the base64
 * download decoder.
 */

import { describe, expect, it } from 'vitest'
import {
  buildExecArgv,
  buildExecPayload,
  decodeBase64,
  exitPill,
  formatTriggerCommand,
  parseExecResult,
  quoteShellArg,
  buildShellBgPayload,
  parseJobStatus,
} from './exec'
import { parseRecords } from './records'

describe('buildExecArgv', () => {
  it('plain: hands the daemon the shell line to shlex-split', () => {
    expect(buildExecArgv('ls -la /tmp', { shell: false, detached: false })).toEqual({
      cmd: 'ls -la /tmp',
    })
  })

  it('shell: wraps in sh -c with the line as ONE argv slot', () => {
    expect(buildExecArgv('ls | wc -l', { shell: true, detached: false })).toEqual({
      argv: ['sh', '-c', 'ls | wc -l'],
    })
  })

  it('detached: setsid sh -c with brace group + background + /dev/null redirect', () => {
    expect(
      buildExecArgv('sleep 300', { shell: true, detached: true }),
    ).toEqual({
      argv: ['setsid', 'sh', '-c', '{ sleep 300 ; } >/dev/null 2>&1 &'],
    })
  })

  it('detached wins regardless of the shell toggle (it implies a shell)', () => {
    expect(
      buildExecArgv('node server.js', { shell: false, detached: true }),
    ).toEqual({
      argv: ['setsid', 'sh', '-c', '{ node server.js ; } >/dev/null 2>&1 &'],
    })
  })

  it('detached groups a multi-statement line — redirect and & cover it all', () => {
    expect(
      buildExecArgv('cd /app; node server.js', { shell: true, detached: true }),
    ).toEqual({
      argv: ['setsid', 'sh', '-c', '{ cd /app; node server.js ; } >/dev/null 2>&1 &'],
    })
  })
})

describe('buildExecPayload', () => {
  const form = {
    line: 'env',
    shell: true,
    detached: false,
    workdir: '',
    timeoutMs: '',
    env: [] as { key: string; value: string }[],
  }

  it('omits empty optionals — no phantom "" / 0 fields on the wire', () => {
    expect(buildExecPayload('sbx-1', form)).toEqual({
      sandbox_id: 'sbx-1',
      argv: ['sh', '-c', 'env'],
    })
  })

  it('carries workdir, timeout and env when present', () => {
    expect(
      buildExecPayload('sbx-1', {
        ...form,
        workdir: ' /app ',
        timeoutMs: '9000',
        env: [
          { key: 'FOO', value: 'bar' },
          { key: '  ', value: 'dropped — blank key' },
        ],
      }),
    ).toEqual({
      sandbox_id: 'sbx-1',
      argv: ['sh', '-c', 'env'],
      workdir: '/app',
      timeout_ms: 9000,
      env: { FOO: 'bar' },
    })
  })

  it('rejects junk timeouts instead of sending them', () => {
    expect(
      buildExecPayload('sbx-1', { ...form, timeoutMs: 'soon' }),
    ).not.toHaveProperty('timeout_ms')
    expect(
      buildExecPayload('sbx-1', { ...form, timeoutMs: '-5' }),
    ).not.toHaveProperty('timeout_ms')
  })
})

describe('formatTriggerCommand', () => {
  it('single-quotes the JSON payload for a paste-able CLI line', () => {
    expect(
      formatTriggerCommand('sandbox::exec', { sandbox_id: 'sbx-1', cmd: 'ls' }),
    ).toBe(`iii trigger sandbox::exec '{"sandbox_id":"sbx-1","cmd":"ls"}'`)
  })

  it("escapes embedded single quotes via the POSIX '\\'' dance", () => {
    const command = formatTriggerCommand('sandbox::exec', {
      sandbox_id: 'sbx-1',
      argv: ['sh', '-c', "echo 'hi'"],
    })
    expect(command).toContain("'\\''")
    // The round-trip stays intact: undoing the quoting yields the JSON.
    const inner = command
      .slice('iii trigger sandbox::exec '.length)
      .replace(/^'|'$/g, '')
      .replaceAll(`'\\''`, `'`)
    expect(JSON.parse(inner)).toEqual({
      sandbox_id: 'sbx-1',
      argv: ['sh', '-c', "echo 'hi'"],
    })
  })
})

describe('quoteShellArg', () => {
  it('leaves safe tokens bare and quotes the rest', () => {
    expect(quoteShellArg('ls')).toBe('ls')
    expect(quoteShellArg('/tmp/a-b_c.txt')).toBe('/tmp/a-b_c.txt')
    expect(quoteShellArg('')).toBe("''")
    expect(quoteShellArg('a b')).toBe("'a b'")
    expect(quoteShellArg("it's")).toBe(`'it'\\''s'`)
  })
})

describe('parseExecResult', () => {
  it('reads a full daemon response', () => {
    expect(
      parseExecResult({
        stdout: 'hi\n',
        stderr: '',
        exit_code: 0,
        timed_out: false,
        duration_ms: 12,
        success: true,
      }),
    ).toEqual({
      stdout: 'hi\n',
      stderr: '',
      exit_code: 0,
      timed_out: false,
      duration_ms: 12,
      success: true,
    })
  })

  it('degrades missing fields instead of throwing', () => {
    const out = parseExecResult({ stdout: 'x' })
    expect(out.exit_code).toBeNull()
    expect(out.duration_ms).toBeNull()
    expect(out.timed_out).toBe(false)
    expect(parseExecResult(null).stdout).toBe('')
    expect(parseExecResult('garbage').stderr).toBe('')
  })

  it('peels a harness result envelope', () => {
    expect(
      parseExecResult({
        content: [{ type: 'text', text: 'hi' }],
        details: { stdout: 'hi', stderr: '', exit_code: 0 },
      }).stdout,
    ).toBe('hi')
  })
})

describe('exitPill', () => {
  it('exit 0 / exit N / timed out / no exit / infra error', () => {
    expect(exitPill({ timed_out: false, exit_code: 0 })).toEqual({
      label: 'exit 0',
      tone: 'ok',
    })
    expect(exitPill({ timed_out: false, exit_code: 2 })).toEqual({
      label: 'exit 2',
      tone: 'warn',
    })
    expect(exitPill({ timed_out: true, exit_code: null })).toEqual({
      label: 'timed out',
      tone: 'warn',
    })
    expect(exitPill({ timed_out: false, exit_code: null })).toEqual({
      label: 'no exit',
      tone: 'warn',
    })
    expect(
      exitPill({ timed_out: false, exit_code: null, error: 'runtime gone' }),
    ).toEqual({ label: 'error', tone: 'alert' })
  })
})

describe('decodeBase64', () => {
  it('decodes wrapped `base64` tool output (newlines and all)', () => {
    // `printf 'hello world' | base64` with a 8-char wrap + trailing newline.
    const wrapped = 'aGVsbG8g\nd29ybGQ=\n'
    expect(new TextDecoder().decode(decodeBase64(wrapped))).toBe('hello world')
  })

  it('round-trips binary bytes exactly', () => {
    const bytes = new Uint8Array([0, 1, 2, 250, 255])
    const b64 = btoa(String.fromCharCode(...bytes))
    expect([...decodeBase64(`${b64}\n`)]).toEqual([0, 1, 2, 250, 255])
  })

  it('throws on non-base64 input (the caller shows the error card)', () => {
    expect(() => decodeBase64('not base64 at all!')).toThrow()
  })
})

describe('parseRecords (state::get value — the bare array, house gotcha)', () => {
  it('reads a stored array and caps at 50 newest', () => {
    const stored = Array.from({ length: 60 }, (_, i) => ({
      id: `r-${i}`,
      at: i,
      cmd: `cmd-${i}`,
      stdout: '',
      stderr: '',
      exit_code: 0,
      timed_out: false,
      duration_ms: 1,
    }))
    const out = parseRecords(stored)
    expect(out).toHaveLength(50)
    expect(out[0].cmd).toBe('cmd-10')
    expect(out[49].cmd).toBe('cmd-59')
  })

  it('rejects non-arrays (a `{ value }` wrapper would be a bug upstream)', () => {
    expect(parseRecords({ value: [] })).toEqual([])
    expect(parseRecords(null)).toEqual([])
    expect(parseRecords(undefined)).toEqual([])
  })

  it('drops rows without a cmd and defaults the rest', () => {
    const [rec] = parseRecords([{ cmd: 'ls' }, { stdout: 'no cmd' }])
    expect(rec.cmd).toBe('ls')
    expect(rec.exit_code).toBeNull()
    expect(rec.source).toBe('exec')
    expect(parseRecords([{ cmd: 'ls' }, { stdout: 'no cmd' }])).toHaveLength(1)
  })
})

describe('buildShellBgPayload', () => {
  const form = (over: Partial<Parameters<typeof buildShellBgPayload>[1]> = {}) => ({
    line: 'sleep 30; echo done',
    workdir: '/ignored',
    timeoutMs: '60000',
    env: [{ key: 'K', value: 'v' }],
    shell: true,
    detached: true,
    ...over,
  })

  it('wraps as sh -c with a sandbox target and carries env + timeout', () => {
    const p = buildShellBgPayload('sb-1', form())
    expect(p.command).toBe('sh')
    expect(p.args).toEqual(['-c', 'sleep 30; echo done'])
    expect(p.target).toEqual({ kind: 'sandbox', sandbox_id: 'sb-1' })
    expect(p.env).toEqual({ K: 'v' })
    expect(p.timeout_ms).toBe(60000)
    expect('workdir' in p).toBe(false)
    expect('cwd' in p).toBe(false)
  })

  it('sends the bare command when the shell toggle is off', () => {
    const p = buildShellBgPayload('sb-1', form({ shell: false, line: 'sleep 30' }))
    expect(p.command).toBe('sleep 30')
    expect('args' in p).toBe(false)
  })
})

describe('parseJobStatus', () => {
  it('reads a finished job into record fields', () => {
    const patch = parseJobStatus({
      job: {
        id: 'j1',
        status: 'exited',
        exit_code: 0,
        stdout: 'done',
        stderr: '',
        started_at_ms: 1000,
        finished_at_ms: 4000,
      },
    })
    expect(patch).toEqual({
      job_state: 'exited',
      exit_code: 0,
      stdout: 'done',
      stderr: '',
      timed_out: false,
      duration_ms: 3000,
    })
  })

  it('keeps duration null while running and flags timed_out status', () => {
    expect(
      parseJobStatus({ job: { status: 'running', stdout: 'tick', stderr: '' } }),
    ).toMatchObject({ job_state: 'running', duration_ms: null })
    expect(
      parseJobStatus({ job: { status: 'timed_out', stdout: '', stderr: '' } }),
    ).toMatchObject({ timed_out: true })
  })

  it('returns null for non-job envelopes', () => {
    expect(parseJobStatus(null)).toBeNull()
    expect(parseJobStatus({})).toBeNull()
    expect(parseJobStatus({ job: { no_status: true } })).toBeNull()
  })
})
