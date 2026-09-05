import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext'
import {
  $getSelection,
  $isNodeSelection,
  COMMAND_PRIORITY_NORMAL,
  KEY_ARROW_DOWN_COMMAND,
  KEY_ARROW_LEFT_COMMAND,
  KEY_ARROW_RIGHT_COMMAND,
  KEY_ARROW_UP_COMMAND,
  type LexicalEditor,
  mergeRegister,
} from 'lexical'
import { useEffect } from 'react'

/**
 * Lets the caret leave a selected pill with the arrow keys. The pills are
 * keyboard-selectable decorators, so arrowing onto one turns the range
 * selection into a node selection on it — and `@lexical/plain-text`
 * (unlike rich-text) has no arrow handling for node selections: its
 * handlers bail on anything that isn't a range selection, the browser
 * moves a DOM caret that Lexical resolves straight back onto the same
 * pill, and the caret is stuck. This mirrors rich-text's handling:
 * Left/Up step to just before the pill, Right/Down to just after it, as a
 * range selection again. NORMAL priority so it runs ahead of the LOW
 * history-nav handler on Up/Down (a selected pill should be left, not
 * browsed away from), and it never touches a range selection.
 */
export function registerPillArrowNav(editor: LexicalEditor): () => void {
  const step = (direction: 'before' | 'after') => (event: KeyboardEvent) => {
    const selection = $getSelection()
    if (!$isNodeSelection(selection)) return false
    const [node] = selection.getNodes()
    if (!node) return false
    event.preventDefault()
    if (direction === 'before') {
      node.selectPrevious()
    } else {
      node.selectNext(0, 0)
    }
    return true
  }
  return mergeRegister(
    editor.registerCommand(
      KEY_ARROW_LEFT_COMMAND,
      step('before'),
      COMMAND_PRIORITY_NORMAL,
    ),
    editor.registerCommand(
      KEY_ARROW_RIGHT_COMMAND,
      step('after'),
      COMMAND_PRIORITY_NORMAL,
    ),
    editor.registerCommand(
      KEY_ARROW_UP_COMMAND,
      step('before'),
      COMMAND_PRIORITY_NORMAL,
    ),
    editor.registerCommand(
      KEY_ARROW_DOWN_COMMAND,
      step('after'),
      COMMAND_PRIORITY_NORMAL,
    ),
  )
}

export function PillArrowNavPlugin() {
  const [editor] = useLexicalComposerContext()
  useEffect(() => registerPillArrowNav(editor), [editor])
  return null
}
