import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext'
import { useLexicalNodeSelection } from '@lexical/react/useLexicalNodeSelection'
import {
  $getNodeByKey,
  CLICK_COMMAND,
  COMMAND_PRIORITY_LOW,
  KEY_BACKSPACE_COMMAND,
  KEY_DELETE_COMMAND,
  mergeRegister,
  type NodeKey,
} from 'lexical'
import { type RefObject, useEffect, useRef } from 'react'

/**
 * Selection for an inline decorator pill (a function mention, a slash
 * command): a click on the pill selects it — shift-click toggles — and
 * Backspace / Delete remove it while selected, so it can be cut, copied,
 * pasted or deleted as one token. Returns the click-target ref the pill
 * must carry (the `CLICK_COMMAND` hit-test is scoped to it) and whether it
 * is currently selected.
 */
export function usePillSelection(nodeKey: NodeKey): {
  pillRef: RefObject<HTMLSpanElement | null>
  isSelected: boolean
} {
  const [editor] = useLexicalComposerContext()
  const [isSelected, setSelected, clearSelection] =
    useLexicalNodeSelection(nodeKey)
  const pillRef = useRef<HTMLSpanElement | null>(null)

  useEffect(() => {
    const removeIfSelected = (event: KeyboardEvent): boolean => {
      if (!isSelected) return false
      event.preventDefault()
      editor.update(() => {
        const node = $getNodeByKey(nodeKey)
        if (node) node.remove()
      })
      return true
    }
    return mergeRegister(
      editor.registerCommand(
        CLICK_COMMAND,
        (event: MouseEvent) => {
          const target = event.target as Node | null
          if (
            !pillRef.current ||
            !target ||
            !pillRef.current.contains(target)
          ) {
            return false
          }
          /* preventDefault keeps the caret from landing inside the decorator
             host; Lexical would otherwise place selection just before/after
             the pill and fight our node selection. */
          event.preventDefault()
          if (event.shiftKey) {
            setSelected(!isSelected)
          } else {
            clearSelection()
            setSelected(true)
          }
          return true
        },
        COMMAND_PRIORITY_LOW,
      ),
      editor.registerCommand(
        KEY_BACKSPACE_COMMAND,
        removeIfSelected,
        COMMAND_PRIORITY_LOW,
      ),
      editor.registerCommand(
        KEY_DELETE_COMMAND,
        removeIfSelected,
        COMMAND_PRIORITY_LOW,
      ),
    )
  }, [editor, nodeKey, isSelected, setSelected, clearSelection])

  return { pillRef, isSelected }
}
