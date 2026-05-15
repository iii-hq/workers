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

export type SerializedFunctionMentionNode = Spread<
  { functionId: string },
  SerializedLexicalNode
>

/**
 * An inline pill representing an `@fn(<functionId>)` mention. Rendered through
 * Lexical's `decorate()` so React owns the visuals (ƒ glyph + name + panel
 * background), while `getTextContent()` returns the plain-text `@fn(<id>)`
 * form so the existing OnChange lift in LexicalShell keeps working. The
 * markdown renderer detects the same `@fn(<id>)` token and reuses the
 * presentational pill (`FunctionMentionPill`) below.
 */
export class FunctionMentionNode extends DecoratorNode<JSX.Element> {
  __functionId: string

  static getType(): string {
    return 'function-mention'
  }

  static clone(node: FunctionMentionNode): FunctionMentionNode {
    return new FunctionMentionNode(node.__functionId, node.__key)
  }

  static importJSON(
    serialized: SerializedFunctionMentionNode,
  ): FunctionMentionNode {
    return $createFunctionMentionNode(serialized.functionId)
  }

  /* Recreate the pill on HTML paste (cross-editor or external apps).
     `exportDOM` already stamps the `data-lexical-function-mention` flag and
     a `data-function-id` attribute, so the round-trip is symmetrical. */
  static importDOM(): DOMConversionMap | null {
    return {
      span: (el: HTMLElement): DOMConversion<HTMLElement> | null => {
        if (el.getAttribute('data-lexical-function-mention') !== 'true') {
          return null
        }
        return {
          conversion: convertFunctionMentionElement,
          priority: 1,
        }
      },
    }
  }

  constructor(functionId: string, key?: NodeKey) {
    super(key)
    this.__functionId = functionId
  }

  exportJSON(): SerializedFunctionMentionNode {
    return {
      type: FunctionMentionNode.getType(),
      version: 1,
      functionId: this.__functionId,
    }
  }

  exportDOM(): DOMExportOutput {
    const element = document.createElement('span')
    element.setAttribute('data-lexical-function-mention', 'true')
    element.setAttribute('data-function-id', this.__functionId)
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
    return `@fn(${this.__functionId})`
  }

  getFunctionId(): string {
    return this.__functionId
  }

  decorate(): JSX.Element {
    return (
      <EditableFunctionMentionPill
        functionId={this.__functionId}
        nodeKey={this.__key}
      />
    )
  }
}

function convertFunctionMentionElement(el: HTMLElement): DOMConversionOutput {
  const functionId = el.getAttribute('data-function-id') ?? ''
  if (!functionId) return { node: null }
  return { node: $createFunctionMentionNode(functionId) }
}

interface PillProps {
  functionId: string
  /** Visible-selected state. Defaults to false; only the Lexical decorator
      wrapper passes a real value. Markdown renders never set this. */
  selected?: boolean
  /** Click-target ref; only the Lexical wrapper supplies one (so its
      `CLICK_COMMAND` handler can scope hit-tests to the pill). Markdown
      renders leave this unset and the pill behaves as pure decoration. */
  pillRef?: RefObject<HTMLSpanElement | null>
}

/**
 * The inserted-token visual. ƒ icon in accent, function id in ink, on a
 * panel background. Rectilinear; no rounded corners; monospace; tight
 * inline-block sizing so it flows with text. When `selected` is true the
 * border swaps to accent and the surface lifts one step to `paper-2` — same
 * 1px footprint, no layout shift. No DOM-level click handler lives here;
 * the Lexical wrapper drives selection via `CLICK_COMMAND` so the pill
 * stays a static, accessible inline element in both editor and markdown.
 */
export function FunctionMentionPill({
  functionId,
  selected,
  pillRef,
}: PillProps) {
  return (
    <span
      ref={pillRef}
      contentEditable={false}
      data-function-id={functionId}
      className={cn(
        'inline-flex items-center gap-1 px-1.5 h-[20px] -mt-[2px] border align-middle font-mono text-[13px] text-ink select-none transition-colors',
        selected
          ? 'bg-paper-2 border-accent cursor-pointer'
          : 'bg-panel border-rule',
        pillRef && 'cursor-pointer',
      )}
    >
      <span
        aria-hidden
        className="text-accent font-semibold italic leading-none"
      >
        ƒ
      </span>
      <span className="leading-none">{functionId}</span>
    </span>
  )
}

interface EditablePillProps {
  functionId: string
  nodeKey: NodeKey
}

/**
 * Lexical-decorator wrapper: tracks selection via `useLexicalNodeSelection`
 * and listens for `CLICK_COMMAND` / `KEY_BACKSPACE_COMMAND` /
 * `KEY_DELETE_COMMAND` so the user can select the pill, then cut/copy/paste
 * or delete it. Click-with-shift toggles the selection; plain click replaces
 * the current selection.
 */
function EditableFunctionMentionPill({
  functionId,
  nodeKey,
}: EditablePillProps) {
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

  return (
    <FunctionMentionPill
      functionId={functionId}
      selected={isSelected}
      pillRef={pillRef}
    />
  )
}

export function $createFunctionMentionNode(
  functionId: string,
): FunctionMentionNode {
  return new FunctionMentionNode(functionId)
}

export function $isFunctionMentionNode(
  node: LexicalNode | null | undefined,
): node is FunctionMentionNode {
  return node instanceof FunctionMentionNode
}
