import { afterEach, describe, expect, it, vi } from 'vitest'
import { copyTextToClipboard } from './clipboard'

afterEach(() => {
  vi.unstubAllGlobals()
})

/** Minimal document standing in for exactly the DOM the fallback touches. */
function fakeDocument(copySucceeds: boolean) {
  const execCommand = vi.fn(() => copySucceeds)
  const doc: { activeElement: unknown } & Record<string, unknown> = {
    activeElement: null,
    body: { appendChild: vi.fn() },
    execCommand,
  }
  const textarea = {
    value: '',
    setAttribute: vi.fn(),
    style: {} as Record<string, string>,
    // A real select() focuses the textarea.
    select: vi.fn(() => {
      doc.activeElement = textarea
    }),
    remove: vi.fn(),
  }
  doc.createElement = vi.fn(() => textarea)
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

  it('fails honestly when a focus trap steals the selection', async () => {
    vi.stubGlobal('navigator', {})
    const { doc, execCommand, textarea } = fakeDocument(true)
    textarea.select = vi.fn()
    vi.stubGlobal('document', doc)
    await expect(copyTextToClipboard('x')).resolves.toBe(false)
    expect(execCommand).not.toHaveBeenCalled()
    expect(textarea.remove).toHaveBeenCalled()
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
