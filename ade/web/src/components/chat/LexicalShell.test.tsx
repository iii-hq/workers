// @vitest-environment jsdom

import {
  $createParagraphNode,
  $createTextNode,
  $getRoot,
  type LexicalEditor,
} from 'lexical'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { LexicalShell } from './LexicalShell'

let container: HTMLDivElement
let root: Root

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  vi.unstubAllGlobals()
})

/** Mount a real Lexical composer with a selected draft and a submit spy. */
async function renderComposer(disabled = false) {
  const onSubmit = vi.fn()
  let editor!: LexicalEditor
  await act(async () => {
    root.render(
      <LexicalShell
        clearToken={0}
        disabled={disabled}
        onChange={() => {}}
        onSubmit={onSubmit}
        initialContent={(instance) => {
          editor = instance
          const paragraph = $createParagraphNode()
          paragraph.append($createTextNode('hello'))
          $getRoot().append(paragraph)
        }}
      />,
    )
  })
  await act(async () => {
    editor.update(() => $getRoot().selectEnd(), { discrete: true })
    // Let the DOM selectionchange event settle inside React's act scope.
    await new Promise<void>((resolve) => setTimeout(resolve, 0))
  })
  const editable = container.querySelector<HTMLElement>(
    '[aria-label="message composer"]',
  )
  if (!editable) throw new Error('Composer not mounted')
  return { editor, editable, onSubmit }
}

/** Dispatch a cancelable Enter keydown and flush the resulting editor updates. */
async function pressEnter(editable: HTMLElement, modifiers: KeyboardEventInit) {
  const event = new KeyboardEvent('keydown', {
    key: 'Enter',
    code: 'Enter',
    keyCode: 13,
    bubbles: true,
    cancelable: true,
    ...modifiers,
  })
  await act(async () => {
    editable.dispatchEvent(event)
  })
  return event
}

describe('composer keyboard submission', () => {
  it.each([
    ['Enter', {}],
    ['Cmd+Enter', { metaKey: true }],
  ] as const)(
    '%s submits once without inserting a newline',
    async (_, modifiers) => {
      const { editor, editable, onSubmit } = await renderComposer()
      const event = await pressEnter(editable, modifiers)

      expect(onSubmit).toHaveBeenCalledTimes(1)
      expect(event.defaultPrevented).toBe(true)
      const text = editor
        .getEditorState()
        .read(() => $getRoot().getTextContent())
      expect(text).toBe('hello')
    },
  )

  it.each([
    ['Shift+Enter', { shiftKey: true }],
    ['Cmd+Shift+Enter', { metaKey: true, shiftKey: true }],
    ['Ctrl+Enter', { ctrlKey: true }],
  ] as const)(
    '%s keeps the existing newline behavior',
    async (_, modifiers) => {
      const { editor, editable, onSubmit } = await renderComposer()
      await pressEnter(editable, modifiers)

      expect(onSubmit).not.toHaveBeenCalled()
      const text = editor
        .getEditorState()
        .read(() => $getRoot().getTextContent())
      expect(text).toBe('hello\n')
    },
  )

  it('does not submit a disabled composer with Cmd+Enter', async () => {
    const { editable, onSubmit } = await renderComposer(true)
    await pressEnter(editable, { metaKey: true })

    expect(onSubmit).not.toHaveBeenCalled()
  })
})
