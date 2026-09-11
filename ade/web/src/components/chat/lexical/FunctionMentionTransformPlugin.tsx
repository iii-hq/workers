import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext'
import { TextNode } from 'lexical'
import { useEffect } from 'react'
import { $createFunctionMentionNode } from './FunctionMentionNode'

/* Matches a single `@fn(<id>)` token where `<id>` contains no whitespace and
   no closing paren. Intentionally non-global so the transform processes one
   match per pass — Lexical will re-run it on the resulting nodes until the
   text stops matching, which keeps the conversion incremental and safe. */
const FN_PATTERN = /@fn\(([^)\s]+)\)/

/**
 * Auto-converts plain `@fn(<id>)` text into the decorator pill. Fires whenever
 * a `TextNode` is mutated — paste, drop, typing, or programmatic insertion —
 * so the user can drop a literal mention into the composer and see it render
 * exactly the same as one inserted through the typeahead menu.
 *
 * The transform splits the matched substring out as its own TextNode (using
 * `splitText`) and replaces it in place with a `FunctionMentionNode`. Leading
 * and trailing text around the match is preserved.
 */
export function FunctionMentionTransformPlugin() {
  const [editor] = useLexicalComposerContext()

  useEffect(() => {
    return editor.registerNodeTransform(TextNode, (node) => {
      if (!node.isSimpleText()) return
      const text = node.getTextContent()
      const match = text.match(FN_PATTERN)
      if (!match || match.index === undefined) return

      const start = match.index
      const end = start + match[0].length
      const functionId = match[1]

      /* `splitText` returns the chunks in order; the original node is mutated
         to hold the first chunk and any tail chunks are returned as new nodes.
         We pick whichever chunk is the matched substring as `target`. */
      let target: TextNode
      if (start === 0 && end === text.length) {
        target = node
      } else if (start === 0) {
        target = node.splitText(end)[0]
      } else if (end === text.length) {
        target = node.splitText(start)[1]
      } else {
        target = node.splitText(start, end)[1]
      }

      const mention = $createFunctionMentionNode(functionId)
      target.replace(mention)
    })
  }, [editor])

  return null
}
