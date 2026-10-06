// @vitest-environment jsdom

import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { CodeEditor } from './CodeEditor'

// A stand-in for the Monaco editor: just the text, the content and
// selection listeners, and the calls the value sync makes.
const { editor, create } = vi.hoisted(() => {
  let text = ''
  let onContent: (() => void) | undefined
  let onSelection:
    | ((event: { selection: { isEmpty(): boolean } }) => void)
    | undefined
  // Like Monaco: setValue collapses the selection, restoreViewState puts
  // the saved (here non-empty) one back, each reported as a change.
  const select = (empty: boolean) =>
    onSelection?.({ selection: { isEmpty: () => empty } })
  const noop = () => {}
  const editor = {
    /** The person typing: the text changes, then Monaco reports it. */
    type(next: string) {
      text = next
      onContent?.()
    },
    getValue: vi.fn(() => text),
    setValue: vi.fn((next: string) => {
      text = next
      onContent?.()
      select(true)
    }),
    saveViewState: vi.fn(() => ({ scrollTop: 400 })),
    restoreViewState: vi.fn(() => select(false)),
    onDidChangeModelContent(listener: () => void) {
      onContent = listener
    },
    onDidChangeCursorSelection(listener: typeof onSelection) {
      onSelection = listener
    },
    getModel: () => ({ updateOptions: noop, getValueInRange: () => 'one' }),
    getSelection: () => ({
      isEmpty: () => false,
      getEndPosition: () => ({ lineNumber: 1, column: 4 }),
      startLineNumber: 1,
      startColumn: 1,
      endLineNumber: 1,
      endColumn: 4,
    }),
    addContentWidget: vi.fn(),
    layoutContentWidget: vi.fn(),
    removeContentWidget: noop,
    getContentHeight: () => 100,
    onDidContentSizeChange: noop,
    onMouseDown: noop,
    onMouseUp: noop,
    updateOptions: noop,
    focus: noop,
    dispose: noop,
  }
  const create = vi.fn((_host: HTMLElement, options: { value: string }) => {
    text = options.value
    return editor
  })
  return { editor, create }
})

vi.mock('@/lib/monaco', () => ({
  CONSOLE_THEME: 'iii-console',
  codeFontFamily: () => 'monospace',
  monaco: { editor: { create, setModelLanguage: () => {} } },
}))

let container: HTMLDivElement
let root: Root
let setText: (next: string) => void = () => {}
const onChange = vi.fn()

function Controlled() {
  const [text, set] = useState('one\ntwo\n')
  setText = set
  return (
    <CodeEditor
      value={text}
      onChange={(next) => {
        onChange(next)
        set(next)
      }}
      language="plaintext"
      fill
      selectionActions={[{ id: 'ref', label: 'Reference', run: () => {} }]}
    />
  )
}

beforeEach(async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
  await act(async () => root.render(<Controlled />))
  await vi.waitFor(() => expect(create).toHaveBeenCalled())
  await act(async () => {})
  vi.clearAllMocks()
})

afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  vi.unstubAllGlobals()
})

describe('CodeEditor value sync', () => {
  it('emits an edit with one buffer read and does not write it back', async () => {
    await act(async () => editor.type('one\ntwo\nthree\n'))

    expect(onChange).toHaveBeenCalledWith('one\ntwo\nthree\n')
    expect(editor.getValue).toHaveBeenCalledTimes(1)
    expect(editor.setValue).not.toHaveBeenCalled()
  })

  it('keeps the view state across a replacement from outside', async () => {
    await act(async () => setText('rewritten\n'))

    expect(editor.setValue).toHaveBeenCalledWith('rewritten\n')
    expect(editor.restoreViewState).toHaveBeenCalledWith({ scrollTop: 400 })
    const [saved] = editor.saveViewState.mock.invocationCallOrder
    const [replaced] = editor.setValue.mock.invocationCallOrder
    const [restored] = editor.restoreViewState.mock.invocationCallOrder
    expect(saved).toBeLessThan(replaced)
    expect(replaced).toBeLessThan(restored)
    // The applied text is not echoed back as an edit.
    expect(onChange).not.toHaveBeenCalled()

    // The mirror follows the replacement: the next edit syncs cleanly.
    await act(async () => editor.type('rewritten\nmore\n'))
    expect(onChange).toHaveBeenCalledWith('rewritten\nmore\n')
    expect(editor.setValue).toHaveBeenCalledTimes(1)
  })

  it('does not raise the selection bar for the restored selection', async () => {
    vi.useFakeTimers()
    try {
      await act(async () => setText('rewritten\n'))
      expect(editor.restoreViewState).toHaveBeenCalled()
      await act(async () => vi.advanceTimersByTime(1000))
      expect(editor.addContentWidget).not.toHaveBeenCalled()
    } finally {
      vi.useRealTimers()
    }
  })
})
