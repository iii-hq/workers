import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext'
import { TextNode } from 'lexical'
import { useEffect } from 'react'
import { SKILL_ID_SOURCE } from '@/lib/slash-commands'
import { $createSlashCommandNode } from './SlashCommandNode'

/* A single `/skill:<id>` token that the writer has finished: it sits after
   a start, whitespace or `(` and is followed by whitespace. Without the
   trailing-space rule the pill would snap shut on the first character
   typed after `/skill:` — while the palette is still open on it. Non-global
   so the transform handles one match per pass; Lexical re-runs it on the
   resulting nodes until the text stops matching. */
const SKILL_PATTERN = new RegExp(
  String.raw`(?<=^|[\s(])(\/skill:${SKILL_ID_SOURCE})(?=\s)`,
)

/**
 * Auto-converts a typed or pasted `/skill:<id> ` into the command pill, so
 * a literal invocation looks exactly like one picked from the palette.
 * Built-ins (`/compact`) are left as text: they only act when they lead the
 * message, so a mid-sentence `/compact` really is just prose.
 *
 * The transform splits the matched substring out as its own TextNode (using
 * `splitText`) and replaces it in place with a `SlashCommandNode`. Leading
 * and trailing text around the match is preserved.
 */
export function SlashCommandTransformPlugin() {
  const [editor] = useLexicalComposerContext()

  useEffect(() => {
    return editor.registerNodeTransform(TextNode, (node) => {
      if (!node.isSimpleText()) return
      const text = node.getTextContent()
      const match = text.match(SKILL_PATTERN)
      if (!match || match.index === undefined) return

      const start = match.index
      const end = start + match[1].length

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

      target.replace($createSlashCommandNode(match[1]))
    })
  }, [editor])

  return null
}
