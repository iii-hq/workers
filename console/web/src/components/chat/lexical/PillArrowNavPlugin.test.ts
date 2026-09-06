import {
  $createNodeSelection,
  $createParagraphNode,
  $createTextNode,
  $getRoot,
  $getSelection,
  $isRangeSelection,
  $setSelection,
  createEditor,
  KEY_ARROW_DOWN_COMMAND,
  KEY_ARROW_LEFT_COMMAND,
  KEY_ARROW_RIGHT_COMMAND,
  KEY_ARROW_UP_COMMAND,
  type LexicalCommand,
  type LexicalEditor,
} from 'lexical'
import { describe, expect, it, vi } from 'vitest'
import { $createFileMentionNode, FileMentionNode } from './FileMentionNode'
import { registerPillArrowNav } from './PillArrowNavPlugin'

/* A headless editor holding `see [pill] now` with the pill node-selected —
   the state arrowing onto a pill leaves behind. */
function editorWithSelectedPill(): LexicalEditor {
  const editor = createEditor({
    nodes: [FileMentionNode],
    onError: (error) => {
      throw error
    },
  })
  registerPillArrowNav(editor)
  editor.update(
    () => {
      const paragraph = $createParagraphNode()
      const pill = $createFileMentionNode('src/a.ts')
      paragraph.append($createTextNode('see '), pill, $createTextNode(' now'))
      $getRoot().append(paragraph)
      const selection = $createNodeSelection()
      selection.add(pill.getKey())
      $setSelection(selection)
    },
    { discrete: true },
  )
  return editor
}

function press(editor: LexicalEditor, command: LexicalCommand<KeyboardEvent>) {
  const event = { preventDefault: vi.fn() }
  const handled = editor.dispatchCommand(
    command,
    event as unknown as KeyboardEvent,
  )
  return { handled, prevented: event.preventDefault.mock.calls.length > 0 }
}

function caret(editor: LexicalEditor) {
  return editor.read(() => {
    const selection = $getSelection()
    if (!$isRangeSelection(selection) || !selection.isCollapsed()) return null
    return {
      text: selection.anchor.getNode().getTextContent(),
      offset: selection.anchor.offset,
    }
  })
}

describe('registerPillArrowNav', () => {
  it('Left leaves a selected pill to just before it', () => {
    const editor = editorWithSelectedPill()
    expect(press(editor, KEY_ARROW_LEFT_COMMAND)).toEqual({
      handled: true,
      prevented: true,
    })
    expect(caret(editor)).toEqual({ text: 'see ', offset: 4 })
  })

  it('Right leaves a selected pill to just after it', () => {
    const editor = editorWithSelectedPill()
    expect(press(editor, KEY_ARROW_RIGHT_COMMAND).handled).toBe(true)
    expect(caret(editor)).toEqual({ text: ' now', offset: 0 })
  })

  it('Up and Down step off the pill the same way', () => {
    const up = editorWithSelectedPill()
    press(up, KEY_ARROW_UP_COMMAND)
    expect(caret(up)).toEqual({ text: 'see ', offset: 4 })
    const down = editorWithSelectedPill()
    press(down, KEY_ARROW_DOWN_COMMAND)
    expect(caret(down)).toEqual({ text: ' now', offset: 0 })
  })

  it('never touches a range selection', () => {
    const editor = editorWithSelectedPill()
    press(editor, KEY_ARROW_RIGHT_COMMAND)
    const before = caret(editor)
    expect(press(editor, KEY_ARROW_LEFT_COMMAND)).toEqual({
      handled: false,
      prevented: false,
    })
    expect(caret(editor)).toEqual(before)
  })
})
