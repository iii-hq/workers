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
import { wt } from '../../typography'
import { serializePlaceholder } from './env-template'

export type SerializedEnvPlaceholderNode = Spread<
  { variable: string; defaultValue: string | null },
  SerializedLexicalNode
>

/**
 * Inline pill for a `${VAR}` or `${VAR:default}` env placeholder. Modeled
 * on `components/chat/lexical/FunctionMentionNode` so the editor's
 * decorator vocabulary stays consistent across surfaces.
 *
 * Why a decorator node:
 *
 * - The visual is a React component (panel-tinted pill with three
 *   typographic slots) rather than a single styled span, which
 *   decorator nodes mount inside their host DOM element.
 * - `getTextContent()` returns the canonical `${VAR:default}` string so
 *   the editor's overall plain-text content is exactly the template
 *   string that gets saved. The transform plugin uses this property to
 *   convert pasted/typed text back into pills idempotently.
 * - `exportDOM()` stamps a `data-env-placeholder` flag + the variable
 *   and default attributes so cross-editor paste round-trips losslessly.
 */
export class EnvPlaceholderNode extends DecoratorNode<JSX.Element> {
  __variable: string
  __defaultValue: string | null

  static getType(): string {
    return 'env-placeholder'
  }

  static clone(node: EnvPlaceholderNode): EnvPlaceholderNode {
    return new EnvPlaceholderNode(
      node.__variable,
      node.__defaultValue,
      node.__key,
    )
  }

  static importJSON(
    serialized: SerializedEnvPlaceholderNode,
  ): EnvPlaceholderNode {
    return $createEnvPlaceholderNode(
      serialized.variable,
      serialized.defaultValue,
    )
  }

  static importDOM(): DOMConversionMap | null {
    return {
      span: (el: HTMLElement): DOMConversion<HTMLElement> | null => {
        if (el.getAttribute('data-env-placeholder') !== 'true') return null
        return {
          conversion: convertEnvPlaceholderElement,
          priority: 1,
        }
      },
    }
  }

  constructor(variable: string, defaultValue: string | null, key?: NodeKey) {
    super(key)
    this.__variable = variable
    this.__defaultValue = defaultValue
  }

  exportJSON(): SerializedEnvPlaceholderNode {
    return {
      type: EnvPlaceholderNode.getType(),
      version: 1,
      variable: this.__variable,
      defaultValue: this.__defaultValue,
    }
  }

  exportDOM(): DOMExportOutput {
    const element = document.createElement('span')
    element.setAttribute('data-env-placeholder', 'true')
    element.setAttribute('data-env-variable', this.__variable)
    if (this.__defaultValue !== null) {
      element.setAttribute('data-env-default', this.__defaultValue)
    }
    element.textContent = this.getTextContent()
    return { element }
  }

  createDOM(_config: EditorConfig): HTMLElement {
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
    return serializePlaceholder({
      kind: 'placeholder',
      variable: this.__variable,
      defaultValue: this.__defaultValue,
    })
  }

  getVariable(): string {
    return this.__variable
  }

  getDefaultValue(): string | null {
    return this.__defaultValue
  }

  decorate(): JSX.Element {
    return (
      <EditableEnvPlaceholderPill
        variable={this.__variable}
        defaultValue={this.__defaultValue}
        nodeKey={this.__key}
      />
    )
  }
}

function convertEnvPlaceholderElement(el: HTMLElement): DOMConversionOutput {
  const variable = el.getAttribute('data-env-variable') ?? ''
  const rawDefault = el.getAttribute('data-env-default')
  if (!variable) return { node: null }
  return {
    node: $createEnvPlaceholderNode(
      variable,
      rawDefault === null ? null : rawDefault,
    ),
  }
}

interface PillProps {
  variable: string
  defaultValue: string | null
  selected?: boolean
  pillRef?: RefObject<HTMLSpanElement | null>
}

/**
 * Presentational pill. Rectilinear, no rounded corners, monospace —
 * matches the existing function-mention pill's geometry so the two
 * decorator vocabularies feel like one system. Three typographic slots:
 *
 *   $ · VARIABLE · :default
 *
 * The `$` glyph is the affordance ("this is an env reference"), the
 * variable name is the content the operator cares about, and the
 * default (when present) is muted so it reads as auxiliary metadata.
 */
export function EnvPlaceholderPill({
  variable,
  defaultValue,
  selected,
  pillRef,
}: PillProps) {
  return (
    <span
      ref={pillRef}
      contentEditable={false}
      data-env-variable={variable}
      className={cn(
        'inline-flex items-center gap-0.5 px-1.5 h-[22px] -mt-[2px] border align-middle text-ink select-none transition-colors',
        wt.body,
        selected
          ? 'bg-paper-2 border-accent cursor-pointer'
          : 'bg-panel border-rule',
        pillRef && 'cursor-pointer',
      )}
      title={
        defaultValue === null
          ? `env: ${variable}`
          : `env: ${variable} · default: ${defaultValue}`
      }
    >
      <span aria-hidden className="text-accent leading-none">
        $
      </span>
      <span className="leading-none">{variable}</span>
      {defaultValue !== null ? (
        <span className="leading-none text-ink-faint">
          {`:${defaultValue}`}
        </span>
      ) : null}
    </span>
  )
}

interface EditablePillProps {
  variable: string
  defaultValue: string | null
  nodeKey: NodeKey
}

/**
 * Lexical wrapper that tracks selection and wires keyboard shortcuts
 * (Backspace / Delete remove the pill when selected; click selects).
 * Mirrors the equivalent wrapper in `FunctionMentionNode`.
 */
function EditableEnvPlaceholderPill({
  variable,
  defaultValue,
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
    <EnvPlaceholderPill
      variable={variable}
      defaultValue={defaultValue}
      selected={isSelected}
      pillRef={pillRef}
    />
  )
}

export function $createEnvPlaceholderNode(
  variable: string,
  defaultValue: string | null,
): EnvPlaceholderNode {
  return new EnvPlaceholderNode(variable, defaultValue)
}

export function $isEnvPlaceholderNode(
  node: LexicalNode | null | undefined,
): node is EnvPlaceholderNode {
  return node instanceof EnvPlaceholderNode
}
