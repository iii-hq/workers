// @vitest-environment jsdom

import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { CodeEditor } from './CodeEditor'

// A stand-in for the Monaco editor: just the text, the content and
// selection listeners, and the calls the value sync makes.
const { editor, create } = vi.hoisted(() => {
  let text = ''
  let domNode: HTMLElement | null = null
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
    getDomNode: () => domNode,
    hasTextFocus: () =>
      !!domNode?.querySelector(
        'textarea.inputarea:focus, .native-edit-context:focus',
      ),
    getTargetAtClientPoint: vi.fn(() => ({ type: 6 })),
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
    dispose: () => {
      domNode?.remove()
      domNode = null
    },
  }
  const create = vi.fn((host: HTMLElement, options: { value: string }) => {
    text = options.value
    domNode = document.createElement('div')
    domNode.className = 'monaco-editor'
    const input = document.createElement('textarea')
    input.className = 'inputarea'
    input.setAttribute('role', 'textbox')
    domNode.append(input)
    host.append(domNode)
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

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.clearAllMocks()
  editor.getTargetAtClientPoint.mockReturnValue({ type: 6 })
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  vi.unstubAllGlobals()
})

describe('CodeEditor value sync', () => {
  beforeEach(async () => {
    await act(async () => root.render(<Controlled />))
    await vi.waitFor(() => expect(create).toHaveBeenCalled())
    await act(async () => {})
    vi.clearAllMocks()
  })

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

describe('CodeEditor validation accessibility', () => {
  it('connects fallback validation to the input and clears resolved errors', () => {
    act(() =>
      root.render(
        <CodeEditor
          value="broken: ["
          onChange={() => {}}
          language="yaml"
          aria-label="Worker configuration"
          aria-invalid
          aria-describedby="configuration-help configuration-error"
          aria-errormessage="configuration-error"
        />,
      ),
    )

    const input = container.querySelector('textarea')
    expect(input?.getAttribute('aria-label')).toBe('Worker configuration')
    expect(input?.getAttribute('aria-invalid')).toBe('true')
    expect(input?.getAttribute('aria-describedby')).toBe(
      'configuration-help configuration-error',
    )
    expect(input?.getAttribute('aria-errormessage')).toBe('configuration-error')

    act(() =>
      root.render(
        <CodeEditor
          value="valid: true"
          onChange={() => {}}
          language="yaml"
          aria-label="Worker configuration"
          aria-describedby="configuration-help"
        />,
      ),
    )

    expect(input?.hasAttribute('aria-invalid')).toBe(false)
    expect(input?.getAttribute('aria-describedby')).toBe('configuration-help')
    expect(input?.hasAttribute('aria-errormessage')).toBe(false)
  })

  it.each(['textarea', 'native edit context'])(
    'updates validation on the Monaco %s and clears resolved errors',
    async (mode) => {
      await act(async () =>
        root.render(
          <CodeEditor
            value="broken: ["
            onChange={() => {}}
            language="yaml"
            aria-invalid
            aria-describedby="configuration-help configuration-error"
            aria-errormessage="configuration-error"
          />,
        ),
      )

      let input = container.querySelector<HTMLElement>('[role="textbox"]')
      if (mode === 'native edit context') {
        const nativeInput = document.createElement('div')
        nativeInput.className = 'native-edit-context'
        nativeInput.setAttribute('role', 'textbox')
        nativeInput.tabIndex = 0
        await act(async () => input?.replaceWith(nativeInput))
        input = nativeInput
      }

      expect(input?.getAttribute('aria-invalid')).toBe('true')
      expect(input?.getAttribute('aria-describedby')).toBe(
        'configuration-help configuration-error',
      )
      expect(input?.getAttribute('aria-errormessage')).toBe(
        'configuration-error',
      )

      await act(async () =>
        root.render(
          <CodeEditor
            value="another: ["
            onChange={() => {}}
            language="yaml"
            aria-invalid="grammar"
            aria-describedby="next-error"
            aria-errormessage="next-error"
          />,
        ),
      )

      expect(input?.getAttribute('aria-invalid')).toBe('grammar')
      expect(input?.getAttribute('aria-describedby')).toBe('next-error')
      expect(input?.getAttribute('aria-errormessage')).toBe('next-error')

      await act(async () =>
        root.render(
          <CodeEditor
            value="valid: true"
            onChange={() => {}}
            language="yaml"
          />,
        ),
      )

      expect(input?.hasAttribute('aria-invalid')).toBe(false)
      expect(input?.hasAttribute('aria-describedby')).toBe(false)
      expect(input?.hasAttribute('aria-errormessage')).toBe(false)
    },
  )
})

describe('CodeEditor cold touch focus', () => {
  it.each(['textarea', 'native edit context'])(
    'prepares the %s before the original completed tap reaches Monaco',
    async (mode) => {
      await act(async () =>
        root.render(
          <CodeEditor value="one" onChange={() => {}} language="yaml" />,
        ),
      )
      const domNode = editor.getDomNode()
      let input = domNode?.querySelector<HTMLElement>('textarea.inputarea')
      if (mode === 'native edit context') {
        const nativeInput = document.createElement('div')
        nativeInput.className = 'native-edit-context'
        nativeInput.setAttribute('role', 'textbox')
        nativeInput.tabIndex = 0
        input?.replaceWith(nativeInput)
        input = nativeInput
      }
      const text = document.createElement('span')
      text.textContent = 'one'
      domNode?.append(text)
      let focusedWhenTapContinues = false
      text.addEventListener('pointerup', () => {
        focusedWhenTapContinues = document.activeElement === input
      })
      const start = new PointerEvent('pointerdown', {
        bubbles: true,
        pointerType: 'touch',
        pointerId: 1,
        button: 0,
        clientX: 80,
        clientY: 400,
      })
      const end = new PointerEvent('pointerup', {
        bubbles: true,
        cancelable: true,
        pointerType: 'touch',
        pointerId: 1,
        button: 0,
        clientX: 80,
        clientY: 400,
      })
      act(() => text.dispatchEvent(start))
      expect(document.activeElement).not.toBe(input)
      act(() => text.dispatchEvent(end))
      expect(focusedWhenTapContinues).toBe(true)
      expect(document.activeElement).toBe(input)
      expect(end.defaultPrevented).toBe(false)
    },
  )

  it.each([
    { gesture: 'pan', y: 280, duration: 100 },
    { gesture: 'held touch', y: 400, duration: 1000 },
  ])(
    'keeps a cold $gesture from focusing the editor',
    async ({ y, duration }) => {
      await act(async () =>
        root.render(
          <CodeEditor value="one" onChange={() => {}} language="yaml" />,
        ),
      )
      const domNode = editor.getDomNode()
      const input = domNode?.querySelector('textarea.inputarea')
      const text = document.createElement('span')
      text.textContent = 'one'
      domNode?.append(text)
      const start = new PointerEvent('pointerdown', {
        bubbles: true,
        pointerType: 'touch',
        pointerId: 1,
        button: 0,
        clientX: 80,
        clientY: 400,
      })
      const end = new PointerEvent('pointerup', {
        bubbles: true,
        cancelable: true,
        pointerType: 'touch',
        pointerId: 1,
        button: 0,
        clientX: 80,
        clientY: y,
      })
      Object.defineProperty(start, 'timeStamp', { value: 10 })
      Object.defineProperty(end, 'timeStamp', { value: 10 + duration })
      act(() => text.dispatchEvent(start))
      expect(document.activeElement).not.toBe(input)
      act(() => text.dispatchEvent(end))
      expect(document.activeElement).not.toBe(input)
      expect(end.defaultPrevented).toBe(false)
    },
  )
})

describe('CodeEditor native touch routing', () => {
  it.each([
    { surface: 'growing code', fill: false, targetType: 6, prevented: false },
    { surface: 'editor widget', fill: false, targetType: 9, prevented: true },
    { surface: 'filled code', fill: true, targetType: 6, prevented: true },
  ])(
    'preserves the scrolling owner for $surface',
    async ({ fill, targetType, prevented }) => {
      await act(async () =>
        root.render(
          <CodeEditor
            value="one"
            onChange={() => {}}
            language="yaml"
            fill={fill}
          />,
        ),
      )
      editor.getTargetAtClientPoint.mockReturnValue({ type: targetType })
      const text = document.createElement('span')
      text.textContent = 'one'
      editor.getDomNode()?.append(text)
      const touch: Touch = {
        identifier: 1,
        target: text,
        clientX: 80,
        clientY: 400,
        pageX: 80,
        pageY: 400,
        screenX: 80,
        screenY: 400,
        radiusX: 1,
        radiusY: 1,
        rotationAngle: 0,
        force: 1,
      }
      const types = ['touchstart', 'touchmove', 'touchend', 'touchcancel']
      // Monaco's document gesture listener prevents browser scrolling for
      // every touch it receives, even when a growing editor cannot scroll.
      const consumeGesture = (event: Event) => event.preventDefault()
      for (const type of types)
        document.addEventListener(type, consumeGesture, { passive: false })
      try {
        for (const finish of ['touchend', 'touchcancel'])
          for (const type of ['touchstart', 'touchmove', finish]) {
            const event = new TouchEvent(type, {
              bubbles: true,
              cancelable: true,
              touches: type === finish ? [] : [touch],
              changedTouches: [touch],
            })
            act(() => text.dispatchEvent(event))
            expect(event.defaultPrevented, type).toBe(prevented)
          }
      } finally {
        for (const type of types)
          document.removeEventListener(type, consumeGesture)
      }
    },
  )
})

describe('CodeEditor settled pointer gestures', () => {
  it.each(['pointerup', 'pointercancel'])(
    'keeps a code pan isolated when a second widget touch finishes with %s',
    async (finish) => {
      await act(async () =>
        root.render(
          <CodeEditor value="one\ntwo" onChange={() => {}} language="yaml" />,
        ),
      )
      const input = editor
        .getDomNode()
        ?.querySelector<HTMLTextAreaElement>('textarea.inputarea')
      if (!input) throw new Error('The mounted editor input is required')
      input.value = 'one\ntwo'
      input.setSelectionRange(3, 3)
      input.focus()
      const text = document.createElement('span')
      text.textContent = 'two'
      editor.getDomNode()?.append(text)
      const continueSelection = (event: PointerEvent) => {
        if (event.buttons === 1) input.setSelectionRange(3, 7)
      }
      const dispatch = (type: string, pointerId: number) => {
        const event = new PointerEvent(type, {
          bubbles: true,
          cancelable: true,
          pointerType: 'touch',
          pointerId,
          button: 0,
          buttons: 1,
          clientX: 80,
          clientY: 400,
        })
        act(() => text.dispatchEvent(event))
        return event
      }
      window.addEventListener('pointermove', continueSelection)
      try {
        editor.getTargetAtClientPoint.mockReturnValue({ type: 6 })
        dispatch('pointerdown', 2)
        editor.getTargetAtClientPoint.mockReturnValue({ type: 9 })
        dispatch('pointerdown', 3)
        dispatch(finish, 3)
        editor.getTargetAtClientPoint.mockReturnValue({ type: 6 })
        const move = dispatch('pointermove', 2)
        expect(input.selectionStart).toBe(3)
        expect(input.selectionEnd).toBe(3)
        expect(move.defaultPrevented).toBe(false)

        // Ending the code gesture removes its route; an unrelated future move
        // must still reach the original selection consumer.
        dispatch(finish, 2)
        dispatch('pointermove', 2)
        expect(input.selectionEnd).toBe(7)
      } finally {
        window.removeEventListener('pointermove', continueSelection)
      }
    },
  )

  it.each([
    {
      surface: 'growing touch',
      fill: false,
      targetType: 6,
      pointerType: 'touch',
      end: 3,
    },
    {
      surface: 'widget touch',
      fill: false,
      targetType: 9,
      pointerType: 'touch',
      end: 7,
    },
    {
      surface: 'filled touch',
      fill: true,
      targetType: 6,
      pointerType: 'touch',
      end: 7,
    },
    {
      surface: 'mouse drag',
      fill: false,
      targetType: 6,
      pointerType: 'mouse',
      end: 7,
    },
    {
      surface: 'pen drag',
      fill: false,
      targetType: 6,
      pointerType: 'pen',
      end: 7,
    },
  ])(
    'preserves the selection owner for $surface',
    async ({ fill, targetType, pointerType, end }) => {
      await act(async () =>
        root.render(
          <CodeEditor
            value="one\ntwo"
            onChange={() => {}}
            language="yaml"
            fill={fill}
          />,
        ),
      )
      editor.getTargetAtClientPoint.mockReturnValue({ type: targetType })
      const input = editor
        .getDomNode()
        ?.querySelector<HTMLTextAreaElement>('textarea.inputarea')
      if (!input) throw new Error('The mounted editor input is required')
      input.value = 'one\ntwo'
      input.setSelectionRange(3, 3)
      input.focus()
      const text = document.createElement('span')
      text.textContent = 'two'
      editor.getDomNode()?.append(text)
      // A compatibility mousedown can leave Monaco's drag monitor on window
      // after the original touch pointerup. It consumes the next buttons=1 move.
      const continueSelection = (event: PointerEvent) => {
        if (event.buttons === 1) input.setSelectionRange(3, 7)
      }
      window.addEventListener('pointermove', continueSelection)
      try {
        act(() =>
          text.dispatchEvent(
            new PointerEvent('pointerdown', {
              bubbles: true,
              pointerType,
              pointerId: 2,
              button: 0,
              buttons: 1,
              clientX: 80,
              clientY: 400,
            }),
          ),
        )
        // A widget-origin drag can enter text; ownership stays at its start.
        editor.getTargetAtClientPoint.mockReturnValue({ type: 6 })
        const move = new PointerEvent('pointermove', {
          bubbles: true,
          cancelable: true,
          pointerType,
          pointerId: 2,
          buttons: 1,
          clientX: 80,
          clientY: 380,
        })
        act(() => text.dispatchEvent(move))
        expect(input.selectionStart).toBe(3)
        expect(input.selectionEnd).toBe(end)
        expect(move.defaultPrevented).toBe(false)
      } finally {
        window.removeEventListener('pointermove', continueSelection)
      }
    },
  )
})
