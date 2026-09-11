import { $createTextNode, type ElementNode } from 'lexical'
import { skillTokenRe } from '@/lib/slash-commands'
import { $createSlashCommandNode } from './SlashCommandNode'

/**
 * Append plain composer text to `parent` as nodes: every `/skill:<id>`
 * token becomes its command pill, the rest text. Programmatic loads (a
 * restored draft, a queued message pulled back for editing) go through
 * here because the text-node transform only converts a token the writer
 * has finished with a trailing space — and a loaded message may well end
 * on its command. `@fn(…)` and `#file(…)` need no help: their closing paren
 * lets the transforms convert them as soon as the text lands.
 */
export function $appendComposerText(parent: ElementNode, text: string): void {
  let last = 0
  for (const m of text.matchAll(skillTokenRe())) {
    if (m.index > last) {
      parent.append($createTextNode(text.slice(last, m.index)))
    }
    parent.append($createSlashCommandNode(m[0]))
    last = m.index + m[0].length
  }
  if (last < text.length) parent.append($createTextNode(text.slice(last)))
}
