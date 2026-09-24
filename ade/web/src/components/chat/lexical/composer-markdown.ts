import { $createCodeNode, $isCodeNode, CodeNode } from '@lexical/code-core'
import {
  $createListItemNode,
  $createListNode,
  $isListItemNode,
  $isListNode,
  ListItemNode,
  ListNode,
  type ListType,
} from '@lexical/list'
import {
  CODE,
  INLINE_CODE,
  ORDERED_LIST,
  type Transformer,
  UNORDERED_LIST,
} from '@lexical/markdown'
import {
  $createLineBreakNode,
  $createParagraphNode,
  $createTextNode,
  $getRoot,
  $isElementNode,
  $isLineBreakNode,
  $isParagraphNode,
  $isTextNode,
  type ElementNode,
  type LexicalNode,
} from 'lexical'
import { $appendComposerText } from './composer-text'

/**
 * The composer's markdown dialect: bullet and numbered lists, fenced code
 * blocks and inline code — the structure a prompt is actually written with,
 * shown in the editor the way the sent message will render. Everything else
 * is prose and travels verbatim: no emphasis, no headings, and no escaping
 * of `*`, `_` or `\`, so a regex or a snake_case identifier reaches the
 * model exactly as typed. That is why this module serializes by hand rather
 * than through `$convertToMarkdownString` (which escapes punctuation) and
 * `$convertFromMarkdownString` (which strips backslash escapes on the way
 * back in): a draft or a queued message must round-trip byte for byte.
 */

/** What the writer types to get structure: `- `, `1. `, "``` " and `` `code` ``. */
export const COMPOSER_SHORTCUT_TRANSFORMERS: Transformer[] = [
  UNORDERED_LIST,
  ORDERED_LIST,
  CODE,
  INLINE_CODE,
]

/** The nodes the dialect needs registered on the editor. */
export const COMPOSER_MARKDOWN_NODES = [ListNode, ListItemNode, CodeNode]

/** An opening fence on a line of its own, with an optional language. */
const FENCE_OPEN = /^ {0,3}(`{3,})[ \t]*([\w+#.-]*)[ \t]*$/

/** A list line: indentation, `-`/`*`/`+` or `N.`, one space, the item. */
const LIST_LINE = /^( *)(?:([-*+])|(\d{1,9})\.) (.*)$/

/**
 * An inline code span: a run of backticks, content that never contains that
 * same run, the run again (not followed by a further backtick). Shorter runs
 * inside are fine, so ``` `` `a` `` ``` reads as the code `` `a` ``.
 */
const INLINE_CODE_SPAN = /(`+)((?:(?!\1)[^\n])+?)\1(?!`)/g

/** A paragraph holding only an opening fence — Enter turns it into a block. */
export const FENCE_ONLY_LINE = FENCE_OPEN

/** The line breaks or the markup that make a paste worth reading as markdown. */
export function looksLikeComposerMarkdown(text: string): boolean {
  return /[\n`]/.test(text) || LIST_LINE.test(text)
}

/** The list item or code block `node` sits in, if any. */
export function $getComposerContainer(
  node: LexicalNode,
): ListItemNode | CodeNode | null {
  let current: LexicalNode | null = node
  while (current !== null) {
    if ($isListItemNode(current) || $isCodeNode(current)) return current
    current = current.getParent()
  }
  return null
}

/* ------------------------------------------------------------------------ */
/* Export                                                                    */
/* ------------------------------------------------------------------------ */

/** The editor's content as composer markdown — what a submit sends. */
export function $exportComposerMarkdown(
  root: ElementNode = $getRoot(),
): string {
  const blocks = root.getChildren()
  const out: string[] = []
  blocks.forEach((block, i) => {
    out.push($exportBlock(block))
    // Markdown reads a line straight after a list as the last item's
    // continuation, so a list is closed with a blank line before prose.
    const next = blocks[i + 1]
    if (
      $isListNode(block) &&
      next !== undefined &&
      !$isListNode(next) &&
      !isEmptyParagraph(next)
    ) {
      out.push('')
    }
  })
  return out.join('\n')
}

function isEmptyParagraph(node: LexicalNode): boolean {
  return $isParagraphNode(node) && node.getTextContent().length === 0
}

function $exportBlock(node: LexicalNode): string {
  if ($isCodeNode(node)) {
    const body = node.getTextContent()
    const fence = fenceFor(body, 3)
    const language = node.getLanguage() ?? ''
    return body.length === 0
      ? `${fence}${language}\n${fence}`
      : `${fence}${language}\n${body}\n${fence}`
  }
  if ($isListNode(node)) return $exportList(node, '')
  if ($isElementNode(node)) return $exportInline(node)
  return node.getTextContent()
}

function $exportList(list: ListNode, indent: string): string {
  const lines: string[] = []
  const numbered = list.getListType() === 'number'
  let index = list.getStart()
  // A nested list is wrapped in an item of its own; it indents under the
  // marker of the item before it.
  let lastMarker = numbered ? `${index}. ` : '- '
  for (const item of list.getChildren()) {
    if (!$isListItemNode(item)) continue
    const only = item.getChildrenSize() === 1 ? item.getFirstChild() : null
    if ($isListNode(only)) {
      lines.push($exportList(only, indent + ' '.repeat(lastMarker.length)))
      continue
    }
    const marker = numbered ? `${index}. ` : '- '
    // A soft line break inside an item continues it, aligned under its text.
    const text = $exportInline(item).replace(
      /\n/g,
      `\n${indent}${' '.repeat(marker.length)}`,
    )
    lines.push(indent + marker + text)
    lastMarker = marker
    if (numbered) index++
  }
  return lines.join('\n')
}

function $exportInline(node: ElementNode): string {
  let out = ''
  let code = ''
  const flush = () => {
    if (code.length > 0) out += wrapInlineCode(code)
    code = ''
  }
  for (const child of node.getChildren()) {
    if ($isLineBreakNode(child)) {
      flush()
      out += '\n'
    } else if ($isTextNode(child)) {
      if (child.hasFormat('code')) {
        code += child.getTextContent()
      } else {
        flush()
        out += child.getTextContent()
      }
    } else {
      flush()
      // Pills serialize to their tokens (`@fn(…)`, `#file(…)`, `/skill:…`).
      out += $isElementNode(child)
        ? $exportInline(child)
        : child.getTextContent()
    }
  }
  flush()
  return out
}

function wrapInlineCode(text: string): string {
  const fence = fenceFor(text, 1)
  // A span that starts or ends with a backtick needs a space inside the fence.
  const pad = text.startsWith('`') || text.endsWith('`') ? ' ' : ''
  return `${fence}${pad}${text}${pad}${fence}`
}

/** A backtick run longer than any inside `text`, at least `min` long. */
function fenceFor(text: string, min: number): string {
  let longest = 0
  for (const run of text.match(/`+/g) ?? []) {
    longest = Math.max(longest, run.length)
  }
  return '`'.repeat(Math.max(min, longest + 1))
}

/* ------------------------------------------------------------------------ */
/* Import                                                                    */
/* ------------------------------------------------------------------------ */

/** Replace the editor's content with `text` read as composer markdown. */
export function $fillComposerMarkdown(text: string): void {
  const root = $getRoot()
  root.clear()
  root.append(...$composerNodesFromMarkdown(text))
}

/** The top-level blocks `text` describes; at least one (empty) paragraph. */
export function $composerNodesFromMarkdown(text: string): ElementNode[] {
  const lines = text.split('\n')
  const nodes: ElementNode[] = []
  let i = 0
  while (i < lines.length) {
    const line = lines[i]
    const fence = line.match(FENCE_OPEN)
    if (fence) {
      const close = new RegExp(`^ {0,3}\`{${fence[1].length},}[ \\t]*$`)
      const body: string[] = []
      let j = i + 1
      while (j < lines.length && !close.test(lines[j])) body.push(lines[j++])
      nodes.push($createCodeBlock(fence[2] || undefined, body))
      // Past the closing fence — or past the end when it never came.
      i = j + 1
      continue
    }
    if (LIST_LINE.test(line)) {
      const [list, next] = $importList(lines, i)
      nodes.push(list)
      i = next
      // The blank line the export puts after a list closes the list; it is
      // not a line of the draft.
      if (
        lines[i] === '' &&
        i + 1 < lines.length &&
        lines[i + 1] !== '' &&
        !LIST_LINE.test(lines[i + 1])
      ) {
        i++
      }
      continue
    }
    const paragraph = $createParagraphNode()
    $appendComposerInline(paragraph, line)
    nodes.push(paragraph)
    i++
  }
  if (nodes.length === 0) nodes.push($createParagraphNode())
  return nodes
}

function $createCodeBlock(language: string | undefined, body: string[]) {
  const code = $createCodeNode(language)
  body.forEach((line, k) => {
    if (k > 0) code.append($createLineBreakNode())
    if (line.length > 0) code.append($createTextNode(line))
  })
  return code
}

/**
 * Read consecutive list lines from `start` into one list, nesting by
 * indentation. Returns the list and the index of the first line after it.
 */
function $importList(lines: string[], start: number): [ListNode, number] {
  interface Level {
    indent: number
    list: ListNode
  }
  const first = lines[start].match(LIST_LINE) as RegExpMatchArray
  const rootLevel: Level = {
    indent: first[1].length,
    list: $createListNode(...listKind(first)),
  }
  const levels: Level[] = [rootLevel]
  let item: ListItemNode | null = null
  let i = start
  while (i < lines.length) {
    const line = lines[i]
    const m = line.match(LIST_LINE)
    if (!m) {
      // An indented line continues the item above it (a soft line break).
      if (item !== null && /^ +\S/.test(line)) {
        item.append($createLineBreakNode())
        $appendComposerInline(item, line.trimStart())
        i++
        continue
      }
      break
    }
    const indent = m[1].length
    const [type, startAt] = listKind(m)
    while (levels.length > 1 && indent < levels[levels.length - 1].indent) {
      levels.pop()
    }
    let level = levels[levels.length - 1]
    if (indent > level.indent) {
      const nested = $createListNode(type, startAt)
      const wrapper = $createListItemNode()
      wrapper.append(nested)
      level.list.append(wrapper)
      level = { indent, list: nested }
      levels.push(level)
    } else if (levels.length === 1 && level.list.getListType() !== type) {
      // Another kind of list at the top level is a list of its own.
      break
    }
    item = $createListItemNode()
    $appendComposerInline(item, m[4])
    level.list.append(item)
    i++
  }
  return [rootLevel.list, i]
}

function listKind(m: RegExpMatchArray): [ListType, number] {
  return m[3] !== undefined ? ['number', Number(m[3])] : ['bullet', 1]
}

/**
 * Append one line of composer text to `parent`: inline code spans become
 * code-formatted text, the rest goes through `$appendComposerText` so a
 * `/skill:` token still lands as its pill.
 */
export function $appendComposerInline(parent: ElementNode, text: string): void {
  let last = 0
  for (const m of text.matchAll(INLINE_CODE_SPAN)) {
    if (m.index > last) $appendComposerText(parent, text.slice(last, m.index))
    let code = m[2]
    // The space that pads a span starting or ending with a backtick.
    if (
      code.length > 2 &&
      code.startsWith(' ') &&
      code.endsWith(' ') &&
      code.trim().length > 0
    ) {
      code = code.slice(1, -1)
    }
    parent.append($createTextNode(code).setFormat('code'))
    last = m.index + m[0].length
  }
  if (last < text.length) $appendComposerText(parent, text.slice(last))
}
