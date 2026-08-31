/**
 * The exit-reason verdict: one pill, one claim. Exit 0 is ok; a
 * timeout names the duration and wins over the exit code; 127/126 and
 * the daemon's synthetic `exec: <cmd>: not found` stderr read as what
 * they are; a missing exit code is "no exit", never an invented one.
 */
import { describe, expect, it } from 'vitest'
import { exitReason, formatTimeoutSecs, isSyntheticNotFound, streamTruncatedAtCap } from '../format'

function resp(extra: { exit_code?: number | null; timed_out?: boolean; duration_ms?: number; stderr?: string }) {
  return {
    exit_code: extra.exit_code === undefined ? 0 : extra.exit_code,
    timed_out: extra.timed_out ?? false,
    duration_ms: extra.duration_ms ?? 12,
    stderr: extra.stderr ?? '',
  }
}

describe('exitReason', () => {
  it('exit 0 is the ok verdict', () => {
    expect(exitReason(resp({}))).toEqual({ label: 'exit 0', tone: 'ok' })
  })

  it('a non-zero exit is alert with the code named', () => {
    expect(exitReason(resp({ exit_code: 2 }))).toEqual({
      label: 'exit 2',
      tone: 'alert',
    })
  })

  it('a timeout names the duration in seconds and is warn-toned', () => {
    expect(exitReason(resp({ exit_code: null, timed_out: true, duration_ms: 1500 }))).toEqual({
      label: 'timed out @ 1.5s',
      tone: 'warn',
    })
    expect(exitReason(resp({ exit_code: null, timed_out: true, duration_ms: 30_000 }))).toEqual({
      label: 'timed out @ 30s',
      tone: 'warn',
    })
  })

  it('timed_out wins over whatever exit code rides along', () => {
    expect(exitReason(resp({ exit_code: 137, timed_out: true, duration_ms: 5000 }))).toEqual({
      label: 'timed out @ 5s',
      tone: 'warn',
    })
  })

  it('exit 127 reads as not found', () => {
    expect(exitReason(resp({ exit_code: 127 }))).toEqual({
      label: 'not found (127)',
      tone: 'alert',
    })
  })

  it("detects the daemon's synthetic not-found stderr", () => {
    expect(exitReason(resp({ exit_code: 127, stderr: 'exec: pytohn: not found\n' }))).toEqual({
      label: 'not found (127)',
      tone: 'alert',
    })
    // Some wires report the failure without the shell's own code.
    expect(exitReason(resp({ exit_code: null, stderr: 'exec: cargo: not found' }))).toEqual({
      label: 'not found',
      tone: 'alert',
    })
    expect(exitReason(resp({ exit_code: 1, stderr: 'exec: cargo: not found' }))).toEqual({
      label: 'not found (1)',
      tone: 'alert',
    })
  })

  it('the stderr signature never overrides a clean exit', () => {
    expect(exitReason(resp({ exit_code: 0, stderr: 'exec: x: not found' }))).toEqual({ label: 'exit 0', tone: 'ok' })
  })

  it('exit 126 reads as not executable', () => {
    expect(exitReason(resp({ exit_code: 126 }))).toEqual({
      label: 'not executable (126)',
      tone: 'alert',
    })
  })

  it('a null exit without a timeout is "no exit", warn', () => {
    expect(exitReason(resp({ exit_code: null }))).toEqual({
      label: 'no exit',
      tone: 'warn',
    })
  })
})

describe('isSyntheticNotFound', () => {
  it('matches the daemon shape anywhere in the stream', () => {
    expect(isSyntheticNotFound('exec: node: not found')).toBe(true)
    expect(isSyntheticNotFound('warmup\nexec: rg: not found\n')).toBe(true)
  })

  it('ignores ordinary not-found prose', () => {
    expect(isSyntheticNotFound('file not found')).toBe(false)
    expect(isSyntheticNotFound('ENOENT: no such file')).toBe(false)
  })
})

describe('formatTimeoutSecs', () => {
  it('keeps one decimal under 10s, whole seconds above', () => {
    expect(formatTimeoutSecs(500)).toBe('0.5s')
    expect(formatTimeoutSecs(1500)).toBe('1.5s')
    expect(formatTimeoutSecs(9940)).toBe('9.9s')
    expect(formatTimeoutSecs(59_600)).toBe('60s')
  })

  it('never claims a number for junk input', () => {
    expect(formatTimeoutSecs(Number.NaN)).toBe('?s')
    expect(formatTimeoutSecs(-1)).toBe('?s')
  })
})

describe('streamTruncatedAtCap', () => {
  const CAP = 1024 * 1024

  it('is false below the cap and true at it', () => {
    expect(streamTruncatedAtCap('x'.repeat(CAP - 1))).toBe(false)
    expect(streamTruncatedAtCap('x'.repeat(CAP))).toBe(true)
  })

  it('measures BYTES, not UTF-16 length', () => {
    // 'é' is 1 UTF-16 unit but 2 UTF-8 bytes: half the cap in units
    // already fills the byte budget.
    expect(streamTruncatedAtCap('é'.repeat(CAP / 2))).toBe(true)
    expect(streamTruncatedAtCap('é'.repeat(CAP / 2 - 1))).toBe(false)
  })

  it('short strings never allocate an encoder pass', () => {
    expect(streamTruncatedAtCap('')).toBe(false)
    expect(streamTruncatedAtCap('hello')).toBe(false)
  })
})
