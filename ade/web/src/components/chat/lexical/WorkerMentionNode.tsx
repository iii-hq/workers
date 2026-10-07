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
import type { JSX } from 'react'
import { WorkerMentionPill } from '@/components/chat/mentions/WorkerMentionPill'
import { formatMention } from '@/lib/mentions/token'
import { usePillSelection } from './use-pill-selection'

export type SerializedWorkerMentionNode = Spread<
  { provider: string; mentionId: string },
  SerializedLexicalNode
>

/**
 * An inline pill for a worker-defined mention, `@<provider>(id="<id>")`
 * (a kanban ticket, a session, a trace…). `getTextContent()` is the token,
 * so the composer's markdown export — and therefore the sent message — is
 * plain text; the pill resolves its icon and name through the provider's
 * get function.
 */
export class WorkerMentionNode extends DecoratorNode<JSX.Element> {
  __provider: string
  __mentionId: string

  static getType(): string {
    return 'worker-mention'
  }

  static clone(node: WorkerMentionNode): WorkerMentionNode {
    return new WorkerMentionNode(node.__provider, node.__mentionId, node.__key)
  }

  static importJSON(
    serialized: SerializedWorkerMentionNode,
  ): WorkerMentionNode {
    return $createWorkerMentionNode(serialized.provider, serialized.mentionId)
  }

  static importDOM(): DOMConversionMap | null {
    return {
      span: (el: HTMLElement): DOMConversion<HTMLElement> | null => {
        if (el.getAttribute('data-lexical-worker-mention') !== 'true')
          return null
        return { conversion: convertWorkerMentionElement, priority: 1 }
      },
    }
  }

  constructor(provider: string, mentionId: string, key?: NodeKey) {
    super(key)
    this.__provider = provider
    this.__mentionId = mentionId
  }

  exportJSON(): SerializedWorkerMentionNode {
    return {
      type: WorkerMentionNode.getType(),
      version: 1,
      provider: this.__provider,
      mentionId: this.__mentionId,
    }
  }

  exportDOM(): DOMExportOutput {
    const element = document.createElement('span')
    element.setAttribute('data-lexical-worker-mention', 'true')
    element.setAttribute('data-mention-provider', this.__provider)
    element.setAttribute('data-mention-id', this.__mentionId)
    element.textContent = this.getTextContent()
    return { element }
  }

  createDOM(_config: EditorConfig): HTMLElement {
    const span = document.createElement('span')
    span.style.display = 'inline-block'
    span.style.verticalAlign = 'middle'
    span.style.maxWidth = '100%'
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
    return formatMention(this.__provider, this.__mentionId)
  }

  getProvider(): string {
    return this.__provider
  }

  getMentionId(): string {
    return this.__mentionId
  }

  decorate(): JSX.Element {
    return (
      <EditableWorkerMentionPill
        provider={this.__provider}
        mentionId={this.__mentionId}
        nodeKey={this.__key}
      />
    )
  }
}

function convertWorkerMentionElement(el: HTMLElement): DOMConversionOutput {
  const provider = el.getAttribute('data-mention-provider') ?? ''
  const mentionId = el.getAttribute('data-mention-id') ?? ''
  if (!provider || !mentionId) return { node: null }
  return { node: $createWorkerMentionNode(provider, mentionId) }
}

/** Selectable (click, shift-click) and removable (Backspace/Delete) as one token. */
function EditableWorkerMentionPill({
  provider,
  mentionId,
  nodeKey,
}: {
  provider: string
  mentionId: string
  nodeKey: NodeKey
}) {
  const { pillRef, isSelected } = usePillSelection(nodeKey)
  return (
    <WorkerMentionPill
      name={provider}
      id={mentionId}
      selected={isSelected}
      pillRef={pillRef}
    />
  )
}

export function $createWorkerMentionNode(
  provider: string,
  mentionId: string,
): WorkerMentionNode {
  return new WorkerMentionNode(provider, mentionId)
}

export function $isWorkerMentionNode(
  node: LexicalNode | null | undefined,
): node is WorkerMentionNode {
  return node instanceof WorkerMentionNode
}
