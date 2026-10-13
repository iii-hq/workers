import { $isCodeNode } from '@lexical/code-core'
import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext'
import { TextNode } from 'lexical'
import { useEffect } from 'react'
import { findMentions } from '@/lib/mentions/token'
import { $createWorkerMentionNode } from './WorkerMentionNode'

/**
 * Turns literal `@<provider>(id="…")` text into the mention pill — a paste,
 * a restored draft, a queued message brought back for editing, or text
 * handed over by another surface — so it looks exactly like one picked
 * from the `@` menu. One token per pass: Lexical re-runs the transform on
 * the split-off nodes until no text matches. Code stays literal.
 */
export function WorkerMentionTransformPlugin() {
  const [editor] = useLexicalComposerContext()

  useEffect(() => {
    return editor.registerNodeTransform(TextNode, (node) => {
      if (!node.isSimpleText()) return
      if (node.hasFormat('code') || $isCodeNode(node.getParent())) return
      const text = node.getTextContent()
      const [match] = findMentions(text)
      if (!match) return

      const start = match.index
      const end = start + match.token.length
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
      target.replace($createWorkerMentionNode(match.name, match.id))
    })
  }, [editor])

  return null
}
