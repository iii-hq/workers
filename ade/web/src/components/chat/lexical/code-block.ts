import type { CodeNode } from '@lexical/code-core'
import {
  $createParagraphNode,
  $isLineBreakNode,
  type LexicalNode,
  type PointType,
  type RangeSelection,
} from 'lexical'

/**
 * Enter inside a code block. A new line keeps the indentation of the line
 * it leaves; on a second blank line at the end of the block the caret steps
 * out into a paragraph after it, and the blank lines go with it. A line
 * holding only the inherited indentation counts as blank. Owning this here
 * also spares the CodeNode's own Enter, which expects the extension setup
 * this editor does not use (and warns about it).
 */
export function $insertCodeLine(
  code: CodeNode,
  selection: RangeSelection,
): void {
  const children = code.getChildren()
  if ($isAtEnd(code, selection.anchor)) {
    const last = lineBefore(children, children.length)
    const previous = last.blank ? lineBefore(children, last.start) : null
    // Both lines blank, and something (at least a line) before them.
    if (previous?.blank && previous.start >= 0) {
      for (let i = children.length - 1; i >= previous.start; i--) {
        children[i].remove()
      }
      const paragraph = $createParagraphNode()
      code.insertAfter(paragraph)
      paragraph.select()
      return
    }
  }
  // `insertRawText` turns the \n into a line break and tabs into tab nodes.
  selection.insertRawText(`\n${$lineIndent(code, selection)}`)
}

/** No line break between the caret and the end of the block. */
export function $isOnLastCodeLine(code: CodeNode, anchor: PointType): boolean {
  const node = anchor.getNode()
  let next: LexicalNode | null
  if (anchor.type === 'element') {
    if (!node.is(code)) return false
    next = code.getChildAtIndex(anchor.offset)
  } else {
    next = node.getNextSibling()
  }
  while (next !== null) {
    if ($isLineBreakNode(next)) return false
    next = next.getNextSibling()
  }
  return true
}

/** The caret sits after the block's last node. */
function $isAtEnd(code: CodeNode, anchor: PointType): boolean {
  const node = anchor.getNode()
  if (anchor.type === 'element') {
    return node.is(code) && anchor.offset === code.getChildrenSize()
  }
  return (
    node.getNextSibling() === null &&
    anchor.offset === node.getTextContentSize()
  )
}

/**
 * The line ending just before child index `end`: the index of the line
 * break that starts it (-1 for the block's first line) and whether it holds
 * only whitespace.
 */
function lineBefore(
  children: LexicalNode[],
  end: number,
): { start: number; blank: boolean } {
  let blank = true
  let i = end - 1
  while (i >= 0 && !$isLineBreakNode(children[i])) {
    if (children[i].getTextContent().trim().length > 0) blank = false
    i--
  }
  return { start: i, blank }
}

/** The leading whitespace of the line the caret is on. */
function $lineIndent(code: CodeNode, selection: RangeSelection): string {
  const { anchor } = selection
  let text = ''
  let node: LexicalNode | null
  if (anchor.type === 'text') {
    const anchorNode = anchor.getNode()
    text = anchorNode.getTextContent().slice(0, anchor.offset)
    node = anchorNode.getPreviousSibling()
  } else {
    node = code.getChildAtIndex(anchor.offset - 1)
  }
  while (node !== null && !$isLineBreakNode(node)) {
    text = node.getTextContent() + text
    node = node.getPreviousSibling()
  }
  return text.match(/^[ \t]*/)?.[0] ?? ''
}
