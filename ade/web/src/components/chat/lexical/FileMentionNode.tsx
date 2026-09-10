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
import { File, FolderOpen } from 'lucide-react'
import { type JSX, type RefObject, useContext, useEffect, useRef } from 'react'
import {
  formatFileMention,
  formatFileMentionInner,
  formatLineRange,
  type LineRange,
  parseFileMentionInner,
} from '@/lib/file-mention-token'
import { cn } from '@/lib/utils'
import { ComposerMentionContext } from './mention-context'

export type SerializedFileMentionNode = Spread<
  { path: string; range?: LineRange },
  SerializedLexicalNode
>

/**
 * An inline pill representing a `#file(<path>[:<from>-<to>])` mention.
 * Rendered through Lexical's `decorate()` so React owns the visuals (file
 * glyph + relative path + line window + panel background), while
 * `getTextContent()` returns the plain-text token so the existing OnChange
 * lift in LexicalShell keeps working. The markdown renderer detects the same
 * token and reuses the presentational pill (`FileMentionPill`) below.
 */
export class FileMentionNode extends DecoratorNode<JSX.Element> {
  __path: string
  __range: LineRange | null

  static getType(): string {
    return 'file-mention'
  }

  static clone(node: FileMentionNode): FileMentionNode {
    return new FileMentionNode(
      node.__path,
      node.__range ?? undefined,
      node.__key,
    )
  }

  static importJSON(serialized: SerializedFileMentionNode): FileMentionNode {
    return $createFileMentionNode(serialized.path, serialized.range)
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

  constructor(path: string, range?: LineRange, key?: NodeKey) {
    super(key)
    this.__path = path
    this.__range = range ?? null
  }

  exportJSON(): SerializedFileMentionNode {
    return {
      type: FileMentionNode.getType(),
      version: 1,
      path: this.__path,
      ...(this.__range ? { range: this.__range } : {}),
    }
  }

  exportDOM(): DOMExportOutput {
    const element = document.createElement('span')
    element.setAttribute('data-lexical-file-mention', 'true')
    element.setAttribute('data-file-path', this.getInner())
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

  /** `src/a.ts:12-40` — the text inside the token's parens. */
  getInner(): string {
    return formatFileMentionInner({
      path: this.__path,
      range: this.__range ?? undefined,
    })
  }

  getTextContent(): string {
    return formatFileMention({
      path: this.__path,
      range: this.__range ?? undefined,
    })
  }

  getPath(): string {
    return this.__path
  }

  getRange(): LineRange | null {
    return this.__range
  }

  decorate(): JSX.Element {
    return (
      <EditableFileMentionPill
        path={this.__path}
        range={this.__range ?? undefined}
        nodeKey={this.__key}
      />
    )
  }
}

function convertFileMentionElement(el: HTMLElement): DOMConversionOutput {
  const inner = el.getAttribute('data-file-path') ?? ''
  if (!inner) return { node: null }
  const ref = parseFileMentionInner(inner)
  return { node: $createFileMentionNode(ref.path, ref.range) }
}

interface PillProps {
  path: string
  range?: LineRange
  /** Visible-selected state. Defaults to false; only the Lexical decorator
      wrapper passes a real value. Markdown renders never set this. */
  selected?: boolean
  /** Click-target ref; only the Lexical wrapper supplies one (so its
      `CLICK_COMMAND` handler can scope hit-tests to the pill). Markdown
      renders leave this unset and the pill behaves as pure decoration. */
  pillRef?: RefObject<HTMLSpanElement | null>
  /** Set when a click opens the file somewhere: the pill says so. */
  openable?: boolean
}

/**
 * Glyph for a mention path: Lucide `FolderOpen` when the path ends in `/`,
 * Lucide `File` otherwise. Shared by the pill and the typeahead menus; 14 px
 * with a lighter stroke so it sits level with 13 px monospace text.
 */
export function PathGlyph({ path }: { path: string }) {
  const Icon = path.endsWith('/') ? FolderOpen : File
  return <Icon size={14} strokeWidth={1.75} aria-hidden />
}

/**
 * The inserted-token visual. File or folder glyph in accent, relative path
 * in ink, the line window (when there is one) in faint ink after it, on a
 * panel background. Rectilinear; no rounded corners; monospace; tight
 * inline-block sizing so it flows with text. When `selected` is true the
 * surface lifts one step — same 1px footprint, no layout shift. No
 * DOM-level click handler lives here; the Lexical wrapper drives selection
 * and opening via `CLICK_COMMAND` so the pill stays a static, accessible
 * inline element in both editor and markdown.
 */
export function FileMentionPill({
  path,
  range,
  selected,
  pillRef,
  openable,
}: PillProps) {
  return (
    <span
      ref={pillRef}
      contentEditable={false}
      data-file-path={formatFileMentionInner({ path, range })}
      title={openable ? `open ${path} in the shell` : undefined}
      className={cn(
        'inline-flex items-center gap-1 px-1.5 h-[20px] -mt-[2px] rounded-xs align-middle font-mono text-[13px] text-ink select-none transition-colors',
        selected ? 'bg-surface-selected cursor-pointer' : 'bg-surface',
        pillRef && 'cursor-pointer',
        openable && 'hover:bg-surface-hover',
      )}
    >
      <span aria-hidden className="text-accent leading-none shrink-0">
        <PathGlyph path={path} />
      </span>
      <span className="leading-none truncate max-w-[280px]">{path}</span>
      {range ? (
        <span className="leading-none text-ink-faint tabular-nums shrink-0">
          :{formatLineRange(range)}
        </span>
      ) : null}
    </span>
  )
}

interface EditablePillProps {
  path: string
  range?: LineRange
  nodeKey: NodeKey
}

/**
 * Lexical-decorator wrapper: tracks selection via `useLexicalNodeSelection`
 * and listens for `CLICK_COMMAND` / `KEY_BACKSPACE_COMMAND` /
 * `KEY_DELETE_COMMAND` so the user can select the pill, then cut/copy/paste
 * or delete it. When the composer can open files, a plain click opens the
 * mentioned file (on its lines) and shift-click selects the pill; without
 * that, a plain click selects and shift-click toggles, as before. Folder
 * mentions only ever select.
 */
function EditableFileMentionPill({ path, range, nodeKey }: EditablePillProps) {
  const [editor] = useLexicalComposerContext()
  const [isSelected, setSelected, clearSelection] =
    useLexicalNodeSelection(nodeKey)
  const pillRef = useRef<HTMLSpanElement | null>(null)
  const { openFile } = useContext(ComposerMentionContext)
  const openable = Boolean(openFile) && !path.endsWith('/')

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
          if (openable && openFile && !event.shiftKey) {
            openFile({ path, range })
            return true
          }
          if (event.shiftKey && !openable) {
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
  }, [
    editor,
    nodeKey,
    isSelected,
    setSelected,
    clearSelection,
    openFile,
    openable,
    path,
    range,
  ])

  return (
    <FileMentionPill
      path={path}
      range={range}
      selected={isSelected}
      pillRef={pillRef}
      openable={openable}
    />
  )
}

export function $createFileMentionNode(
  path: string,
  range?: LineRange,
): FileMentionNode {
  return new FileMentionNode(path, range)
}

export function $isFileMentionNode(
  node: LexicalNode | null | undefined,
): node is FileMentionNode {
  return node instanceof FileMentionNode
}
