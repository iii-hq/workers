import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext'
import { TextNode } from 'lexical'
import { useEffect } from 'react'
import { $createEnvPlaceholderNode } from './EnvPlaceholderNode'
import { findFirstPlaceholder } from './env-template'

/**
 * Auto-converts plain `${VAR}` / `${VAR:default}` text into the env
 * placeholder pill. Modeled on `FunctionMentionTransformPlugin` — fires
 * whenever a `TextNode` mutates (typing, paste, drop, programmatic
 * insertion) and replaces the matched substring with a decorator node.
 *
 * The transform is intentionally non-greedy: it converts one match per
 * pass and lets Lexical re-run on the freshly-created text nodes until
 * no matches remain. That keeps the splits incremental and safe — a
 * paste containing five placeholders still ends up with five pills.
 */
export function EnvPlaceholderTransformPlugin() {
  const [editor] = useLexicalComposerContext()

  useEffect(() => {
    return editor.registerNodeTransform(TextNode, (node) => {
      if (!node.isSimpleText()) return
      const text = node.getTextContent()
      const match = findFirstPlaceholder(text)
      if (!match) return

      const { start, end, segment } = match

      // `splitText` returns the original node + tail chunks in source
      // order. We pick whichever chunk is the matched substring and
      // replace it in place with the decorator node.
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

      const pill = $createEnvPlaceholderNode(
        segment.variable,
        segment.defaultValue,
      )
      target.replace(pill)
    })
  }, [editor])

  return null
}
