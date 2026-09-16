import {
  copyText,
  errorCode,
  errorMessage,
  formatBytes,
  formatDuration,
  formatRelative,
} from '@iii-dev/console-ui/format'
import { afterEach, describe, expect, it, vi } from 'vitest'

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('formatRelative', () => {
  const now = Date.UTC(2026, 8, 16, 12, 0, 0)
  const ago = (ms: number) => now - ms

  it('cascades through the units', () => {
    expect(formatRelative(ago(3_000), now)).toBe('just now')
    expect(formatRelative(ago(42_000), now)).toBe('42s')
    expect(formatRelative(ago(5 * 60_000), now)).toBe('5m')
    expect(formatRelative(ago(3 * 3_600_000), now)).toBe('3h')
    expect(formatRelative(ago(2 * 86_400_000), now)).toBe('2d')
    expect(formatRelative(ago(45 * 86_400_000), now)).toBe('1mo')
    expect(formatRelative(ago(400 * 86_400_000), now)).toBe('1y')
  })

  it('accepts unix seconds, numeric strings, ISO strings and Dates', () => {
    const t = ago(5 * 60_000)
    expect(formatRelative(Math.floor(t / 1000), now)).toBe('5m')
    expect(formatRelative(String(t), now)).toBe('5m')
    expect(formatRelative(new Date(t).toISOString(), now)).toBe('5m')
    expect(formatRelative(new Date(t), now)).toBe('5m')
  })

  it('clamps the future and rejects garbage', () => {
    expect(formatRelative(now + 60_000, now)).toBe('just now')
    expect(formatRelative('not a date', now)).toBe('')
    expect(formatRelative(Number.NaN, now)).toBe('')
  })
})

describe('formatDuration', () => {
  it('formats ms, s, m and h', () => {
    expect(formatDuration(842)).toBe('842ms')
    expect(formatDuration(1_400)).toBe('1.4s')
    expect(formatDuration(125_000)).toBe('2m 05s')
    expect(formatDuration(72 * 60_000)).toBe('1h 12m')
    expect(formatDuration(-5)).toBe('0ms')
  })
})

describe('formatBytes', () => {
  it('uses binary units with a space and a dash for null', () => {
    expect(formatBytes(512)).toBe('512 B')
    expect(formatBytes(1024)).toBe('1.0 KiB')
    expect(formatBytes(3.2 * 1024 * 1024)).toBe('3.2 MiB')
    expect(formatBytes(null)).toBe('—')
    expect(formatBytes(undefined)).toBe('—')
  })
})

describe('errorMessage', () => {
  it('passes Errors and strings through', () => {
    expect(errorMessage(new Error('plain'))).toBe('plain')
    expect(errorMessage('text')).toBe('text')
  })

  it('renders the wire error object with its code', () => {
    expect(
      errorMessage({
        code: 'function_not_found',
        message: 'Function x::y not found',
      }),
    ).toBe('function_not_found: Function x::y not found')
  })

  it('unwraps handler errors nested inside the transport envelope', () => {
    expect(
      errorMessage({
        code: 'invocation_failed',
        message: 'handler error: {"code":"D214","message":"already exists."}',
      }),
    ).toBe('D214: already exists.')
    expect(
      errorMessage({
        code: 'invocation_failed',
        message: 'handler error: D214 invalid_input: plain prose',
      }),
    ).toBe('D214 invalid_input: plain prose')
  })

  it('reads error / reason / detail shapes', () => {
    expect(errorMessage({ error: 'boom' })).toBe('boom')
    expect(errorMessage({ error: { code: 'E1', message: 'nested' } })).toBe(
      'E1: nested',
    )
    expect(errorMessage({ reason: 'why' })).toBe('why')
    expect(errorMessage({ detail: 'more' })).toBe('more')
  })

  it('never yields [object Object]', () => {
    expect(errorMessage({ weird: true })).toBe('{"weird":true}')
    const cyclic: Record<string, unknown> = {}
    cyclic.self = cyclic
    expect(errorMessage(cyclic)).toBe('Unknown error')
    expect(errorMessage(undefined)).toBe('Unknown error')
    expect(errorMessage(42)).toBe('42')
  })

  it('exposes the innermost code', () => {
    expect(
      errorCode({
        code: 'invocation_failed',
        message: 'handler error: {"code":"FUNCTION_NOT_FOUND","message":"x"}',
      }),
    ).toBe('FUNCTION_NOT_FOUND')
    expect(errorCode('nope')).toBeUndefined()
  })
})

describe('copyText', () => {
  it('uses navigator.clipboard when available', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    vi.stubGlobal('navigator', { clipboard: { writeText } })
    await expect(copyText('hello')).resolves.toBe(true)
    expect(writeText).toHaveBeenCalledWith('hello')
  })

  it('falls back to execCommand and always removes the textarea', async () => {
    vi.stubGlobal('navigator', {})
    const textarea = {
      value: '',
      setAttribute: vi.fn(),
      style: {} as Record<string, string>,
      select: vi.fn(() => {
        throw new Error('no selection')
      }),
      remove: vi.fn(),
    }
    vi.stubGlobal('document', {
      activeElement: null,
      createElement: vi.fn(() => textarea),
      body: { appendChild: vi.fn() },
      execCommand: vi.fn(() => true),
    })
    await expect(copyText('x')).resolves.toBe(false)
    expect(textarea.remove).toHaveBeenCalled()
  })

  it('returns false with no navigator and no document', async () => {
    vi.stubGlobal('navigator', undefined)
    vi.stubGlobal('document', undefined)
    await expect(copyText('x')).resolves.toBe(false)
  })
})
