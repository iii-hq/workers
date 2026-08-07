import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext'
import { useLexicalNodeSelection } from '@lexical/react/useLexicalNodeSelection'
import {
  $getNodeByKey,
  CLICK_COMMAND,
  COMMAND_PRIORITY_LOW,
  DecoratorNode,
  type DOMConversion,
  type DOMConversionMap,
  type DOMConversionOutput,
  type DOMExportOutput,
  type EditorConfig,
  KEY_BACKSPACE_COMMAND,
  KEY_DELETE_COMMAND,
  type LexicalNode,
  mergeRegister,
  type NodeKey,
  type SerializedLexicalNode,
  type Spread,
} from 'lexical'
import { type JSX, type RefObject, useEffect, useRef } from 'react'
import { cn } from '@/lib/utils'

export type SerializedFileMentionNode = Spread<
  { path: string },
  SerializedLexicalNode
>

/**
 * An inline pill representing a `#file(<path>)` mention. Rendered through
 * Lexical's `decorate()` so React owns the visuals (file glyph + relative
 * path + panel background), while `getTextContent()` returns the plain-text
 * `#file(<path>)` form so the existing OnChange lift in LexicalShell keeps
 * working. The markdown renderer detects the same `#file(<path>)` token and
 * reuses the presentational pill (`FileMentionPill`) below.
 */
export class FileMentionNode extends DecoratorNode<JSX.Element> {
  __path: string

  static getType(): string {
    return 'file-mention'
  }

  static clone(node: FileMentionNode): FileMentionNode {
    return new FileMentionNode(node.__path, node.__key)
  }

  static importJSON(serialized: SerializedFileMentionNode): FileMentionNode {
    return $createFileMentionNode(serialized.path)
  }

  /* Recreate the pill on HTML paste (cross-editor or external apps).
     `exportDOM` already stamps the `data-lexical-file-mention` flag and
     a `data-file-path` attribute, so the round-trip is symmetrical. */
  static importDOM(): DOMConversionMap | null {
    return {
      span: (el: HTMLElement): DOMConversion<HTMLElement> | null => {
        if (el.getAttribute('data-lexical-file-mention') !== 'true') {
          return null
        }
        return {
          conversion: convertFileMentionElement,
          priority: 1,
        }
      },
    }
  }

  constructor(path: string, key?: NodeKey) {
    super(key)
    this.__path = path
  }

  exportJSON(): SerializedFileMentionNode {
    return {
      type: FileMentionNode.getType(),
      version: 1,
      path: this.__path,
    }
  }

  exportDOM(): DOMExportOutput {
    const element = document.createElement('span')
    element.setAttribute('data-lexical-file-mention', 'true')
    element.setAttribute('data-file-path', this.__path)
    element.textContent = this.getTextContent()
    return { element }
  }

  createDOM(_config: EditorConfig): HTMLElement {
    /* Lexical needs a host DOM node; React's decorate() output mounts inside. */
    const span = document.createElement('span')
    span.style.display = 'inline-block'
    span.style.verticalAlign = 'middle'
    return span
  }

  updateDOM(): false {
    return false
  }

  isInline(): true {
    return true
  }

  isKeyboardSelectable(): true {
    return true
  }

  getTextContent(): string {
    return `#file(${this.__path})`
  }

  getPath(): string {
    return this.__path
  }

  decorate(): JSX.Element {
    return <EditableFileMentionPill path={this.__path} nodeKey={this.__key} />
  }
}

function convertFileMentionElement(el: HTMLElement): DOMConversionOutput {
  const path = el.getAttribute('data-file-path') ?? ''
  if (!path) return { node: null }
  return { node: $createFileMentionNode(path) }
}

interface PillProps {
  path: string
  /** Visible-selected state. Defaults to false; only the Lexical decorator
      wrapper passes a real value. Markdown renders never set this. */
  selected?: boolean
  /** Click-target ref; only the Lexical wrapper supplies one (so its
      `CLICK_COMMAND` handler can scope hit-tests to the pill). Markdown
      renders leave this unset and the pill behaves as pure decoration. */
  pillRef?: RefObject<HTMLSpanElement | null>
}

/**
 * Hairline glyph for a mention path: folder tab-outline when the path ends
 * in `/`, rectangle-with-corner-fold otherwise. Shared by the pill and the
 * `#` typeahead menu.
 */
export function PathGlyph({ path }: { path: string }) {
  return (
    <svg
      width="9"
      height="11"
      viewBox="0 0 10 12"
      fill="none"
      stroke="currentColor"
      strokeWidth="1"
      aria-hidden="true"
    >
      {path.endsWith('/') ? (
        <path d="M1 2.5H4L5.5 4H9V10H1V2.5Z" />
      ) : (
        <>
          <path d="M1 1H6L9 4V11H1V1Z" />
          <path d="M6 1V4H9" />
        </>
      )}
    </svg>
  )
}

/**
 * The inserted-token visual. Hairline file glyph in accent, relative path in
 * ink, on a panel background. Rectilinear; no rounded corners; monospace;
 * tight inline-block sizing so it flows with text. When `selected` is true
 * the border swaps to accent and the surface lifts one step to `paper-2` —
 * same 1px footprint, no layout shift. No DOM-level click handler lives
 * here; the Lexical wrapper drives selection via `CLICK_COMMAND` so the pill
 * stays a static, accessible inline element in both editor and markdown.
 */
export function FileMentionPill({ path, selected, pillRef }: PillProps) {
  return (
    <span
      ref={pillRef}
      contentEditable={false}
      data-file-path={path}
      className={cn(
        'inline-flex items-center gap-1 px-1.5 h-[20px] -mt-[2px] rounded-xs align-middle font-mono text-[13px] text-ink select-none transition-colors',
        selected ? 'bg-surface-selected cursor-pointer' : 'bg-surface',
        pillRef && 'cursor-pointer',
      )}
    >
      <span aria-hidden className="text-accent leading-none shrink-0">
        <PathGlyph path={path} />
      </span>
      <span className="leading-none truncate max-w-[280px]">{path}</span>
    </span>
  )
}

interface EditablePillProps {
  path: string
  nodeKey: NodeKey
}

/**
 * Lexical-decorator wrapper: tracks selection via `useLexicalNodeSelection`
 * and listens for `CLICK_COMMAND` / `KEY_BACKSPACE_COMMAND` /
 * `KEY_DELETE_COMMAND` so the user can select the pill, then cut/copy/paste
 * or delete it. Click-with-shift toggles the selection; plain click replaces
 * the current selection.
 */
function EditableFileMentionPill({ path, nodeKey }: EditablePillProps) {
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

  return <FileMentionPill path={path} selected={isSelected} pillRef={pillRef} />
}

export function $createFileMentionNode(path: string): FileMentionNode {
  return new FileMentionNode(path)
}

export function $isFileMentionNode(
  node: LexicalNode | null | undefined,
): node is FileMentionNode {
  return node instanceof FileMentionNode
}
