import { afterEach, describe, expect, it, vi } from 'vitest'
import { copyTextToClipboard } from './clipboard'

afterEach(() => {
  vi.unstubAllGlobals()
})

/** Minimal document standing in for exactly the DOM the fallback touches. */
function fakeDocument(copySucceeds: boolean) {
  const execCommand = vi.fn(() => copySucceeds)
  const textarea = {
    value: '',
    setAttribute: vi.fn(),
    style: {} as Record<string, string>,
    select: vi.fn(),
    remove: vi.fn(),
  }
  const doc = {
    activeElement: null,
    createElement: vi.fn(() => textarea),
    body: { appendChild: vi.fn() },
    execCommand,
  }
  return { doc, execCommand, textarea }
}

describe('copyTextToClipboard', () => {
  it('uses navigator.clipboard when available', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    vi.stubGlobal('navigator', { clipboard: { writeText } })
    await expect(copyTextToClipboard('hello')).resolves.toBe(true)
    expect(writeText).toHaveBeenCalledWith('hello')
  })

  it('falls back to execCommand when the async API is missing (insecure origin)', async () => {
    vi.stubGlobal('navigator', {})
    const { doc, execCommand, textarea } = fakeDocument(true)
    vi.stubGlobal('document', doc)
    await expect(copyTextToClipboard('over lan')).resolves.toBe(true)
    expect(execCommand).toHaveBeenCalledWith('copy')
    expect(textarea.value).toBe('over lan')
    expect(textarea.remove).toHaveBeenCalled()
  })

  it('falls back when the async API rejects (permissions)', async () => {
    const writeText = vi.fn().mockRejectedValue(new Error('denied'))
    vi.stubGlobal('navigator', { clipboard: { writeText } })
    const { doc, execCommand } = fakeDocument(true)
    vi.stubGlobal('document', doc)
    await expect(copyTextToClipboard('x')).resolves.toBe(true)
    expect(execCommand).toHaveBeenCalledWith('copy')
  })

  it('returns false when both strategies fail', async () => {
    vi.stubGlobal('navigator', {})
    const { doc } = fakeDocument(false)
    vi.stubGlobal('document', doc)
    await expect(copyTextToClipboard('x')).resolves.toBe(false)
  })

  it('returns false with no navigator and no document at all', async () => {
    vi.stubGlobal('navigator', undefined)
    vi.stubGlobal('document', undefined)
    await expect(copyTextToClipboard('x')).resolves.toBe(false)
  })
})
