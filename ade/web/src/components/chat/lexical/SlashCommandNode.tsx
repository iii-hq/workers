import {
  DecoratorNode,
  type DOMConversion,
  type DOMConversionMap,
  type DOMConversionOutput,
  type DOMExportOutput,
  type EditorConfig,
  type LexicalNode,
  type NodeKey,
  type SerializedLexicalNode,
  type Spread,
} from 'lexical'
import type { JSX, RefObject } from 'react'
import { slashCommandLabel } from '@/lib/slash-commands'
import { cn } from '@/lib/utils'
import { usePillSelection } from './use-pill-selection'

export type SerializedSlashCommandNode = Spread<
  { command: string },
  SerializedLexicalNode
>

/**
 * An inline pill for a `/` command the palette inserted — `/compact` or a
 * `/skill:<id>` invocation. Rendered through Lexical's `decorate()` so React
 * owns the visuals (`/` glyph + slug on a surface), while `getTextContent()`
 * returns the literal command so the OnChange lift in LexicalShell and the
 * submit-time expansion keep seeing plain `/skill:<id>` text. The markdown
 * renderer detects the same token and reuses the presentational pill
 * (`SlashCommandPill`) below.
 */
export class SlashCommandNode extends DecoratorNode<JSX.Element> {
  __command: string

  static getType(): string {
    return 'slash-command'
  }

  static clone(node: SlashCommandNode): SlashCommandNode {
    return new SlashCommandNode(node.__command, node.__key)
  }

  static importJSON(serialized: SerializedSlashCommandNode): SlashCommandNode {
    return $createSlashCommandNode(serialized.command)
  }

  /* Recreate the pill on HTML paste (cross-editor or external apps).
     `exportDOM` stamps the `data-lexical-slash-command` flag and a
     `data-command` attribute, so the round-trip is symmetrical. */
  static importDOM(): DOMConversionMap | null {
    return {
      span: (el: HTMLElement): DOMConversion<HTMLElement> | null => {
        if (el.getAttribute('data-lexical-slash-command') !== 'true') {
          return null
        }
        return { conversion: convertSlashCommandElement, priority: 1 }
      },
    }
  }

  constructor(command: string, key?: NodeKey) {
    super(key)
    this.__command = command
  }

  exportJSON(): SerializedSlashCommandNode {
    return {
      type: SlashCommandNode.getType(),
      version: 1,
      command: this.__command,
    }
  }

  exportDOM(): DOMExportOutput {
    const element = document.createElement('span')
    element.setAttribute('data-lexical-slash-command', 'true')
    element.setAttribute('data-command', this.__command)
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
    return this.__command
  }

  getCommand(): string {
    return this.__command
  }

  decorate(): JSX.Element {
    return (
      <EditableSlashCommandPill command={this.__command} nodeKey={this.__key} />
    )
  }
}

function convertSlashCommandElement(el: HTMLElement): DOMConversionOutput {
  const command = el.getAttribute('data-command') ?? ''
  if (!command) return { node: null }
  return { node: $createSlashCommandNode(command) }
}

interface PillProps {
  /** The literal command: `/compact` or `/skill:<id>`. */
  command: string
  /** Visible-selected state. Defaults to false; only the Lexical decorator
      wrapper passes a real value. Markdown renders never set this. */
  selected?: boolean
  /** Click-target ref; only the Lexical wrapper supplies one (so its
      `CLICK_COMMAND` handler can scope hit-tests to the pill). Markdown
      renders leave this unset and the pill behaves as pure decoration. */
  pillRef?: RefObject<HTMLSpanElement | null>
}

/**
 * The inserted-token visual. Same anatomy as `FunctionMentionPill`: `/`
 * glyph in accent, slug in ink, on a surface — rectilinear, monospace,
 * tight inline-block sizing so it flows with text. The glyph alone tells a
 * command apart from a mention, the way `ƒ` marks a function. The slug
 * drops the `/` and the `skill:` namespace (`coder/index`), the way the
 * function pill drops `@fn(`. When `selected` the surface lifts one step —
 * same footprint, no layout shift.
 */
export function SlashCommandPill({ command, selected, pillRef }: PillProps) {
  return (
    <span
      ref={pillRef}
      contentEditable={false}
      data-slash-command={command}
      className={cn(
        'inline-flex items-center gap-1 px-1.5 h-[20px] -mt-[2px] rounded-xs align-middle font-mono text-[13px] text-ink select-none transition-colors',
        selected ? 'bg-surface-selected cursor-pointer' : 'bg-surface',
        pillRef && 'cursor-pointer',
      )}
    >
      <span aria-hidden className="text-accent font-semibold leading-none">
        /
      </span>
      <span className="leading-none">{slashCommandLabel(command)}</span>
    </span>
  )
}

function EditableSlashCommandPill({
  command,
  nodeKey,
}: {
  command: string
  nodeKey: NodeKey
}) {
  const { pillRef, isSelected } = usePillSelection(nodeKey)
  return (
    <SlashCommandPill
      command={command}
      selected={isSelected}
      pillRef={pillRef}
    />
  )
}

export function $createSlashCommandNode(command: string): SlashCommandNode {
  return new SlashCommandNode(command)
}

export function $isSlashCommandNode(
  node: LexicalNode | null | undefined,
): node is SlashCommandNode {
  return node instanceof SlashCommandNode
}
